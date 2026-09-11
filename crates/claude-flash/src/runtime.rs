//! The agent's main loop.
//!
//! It owns the policy engine, feeds it hook events, API calls and timer ticks, and
//! carries out what the engine decides: flashes and notifications through the front
//! end, pushes, the journal, and the switches that must survive a restart.

use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant, SystemTime};

use flash_core::config::{self, Config};
use flash_core::engine::{Action, Context, Control as Switch, Engine, Input, Notice};
use flash_core::event::{Attention, HookEvent, Signal};
use flash_core::journal::Record;
use flash_core::{duration, glob, time};

use crate::api::{Control, LastEvent, Status, Today};
use crate::hub::Hub;
use crate::journal::JournalWriter;
use crate::paths::Paths;
use crate::store::{self, State};
use crate::ui::{Probe, UiEvent, UiHandle};
use crate::{log, push, system};

/// How often presence, configuration changes and deadlines are checked while
/// nothing else is happening.
const TICK: Duration = Duration::from_secs(1);

pub enum Request {
    Hook(HookEvent),
    Signal(Signal),
    Control(Control, Option<SyncSender<Result<Status, String>>>),
    Status(SyncSender<Status>),
}

/// Counters the HTTP server updates directly.
#[derive(Default)]
pub struct Shared {
    /// Hook events dropped because their session set `CLAUDE_FLASH=off`.
    pub opted_out: AtomicU64,
}

pub struct Runtime {
    engine: Engine,
    paths: Paths,
    port: u16,
    display: &'static str,
    ui: Box<dyn UiHandle>,
    probe: Box<dyn Probe>,
    hub: Arc<Hub>,
    shared: Arc<Shared>,
    journal: JournalWriter,
    journal_failing: bool,
    state_failing: bool,
    epoch: Instant,
    started_unix_ms: u64,
    config_stamp: Option<(SystemTime, u64)>,
    config_error: Option<String>,
    state: State,
    today: (String, Today),
    last_event: Option<LastEvent>,
    quitting: bool,
}

impl Runtime {
    pub fn new(
        paths: Paths,
        port: u16,
        display: &'static str,
        ui: Box<dyn UiHandle>,
        probe: Box<dyn Probe>,
        hub: Arc<Hub>,
        shared: Arc<Shared>,
    ) -> Runtime {
        let (config, config_error) = load_config(&paths.config_file());
        if let Some(e) = &config_error {
            log!("configuration not applied, using defaults: {e}");
        }
        let state = State::load(&paths.state_file());
        let mut runtime = Runtime {
            journal: JournalWriter::new(paths.journal_dir(), config.journal.retain_days),
            journal_failing: false,
            state_failing: false,
            config_stamp: file_stamp(&paths.config_file()),
            engine: Engine::new(config),
            paths,
            port,
            display,
            ui,
            probe,
            hub,
            shared,
            epoch: Instant::now(),
            started_unix_ms: system::unix_ms(),
            config_error,
            state: state.clone(),
            today: (String::new(), Today::default()),
            last_event: None,
            quitting: false,
        };
        let ctx = runtime.context();
        let pause = state.paused_until.map(|until| until.saturating_sub(ctx.unix_ms)).filter(|ms| *ms > 0);
        runtime.engine.restore(state.enabled, pause, &ctx);
        runtime.engine.restore_interactive(state.interactive, ctx.unix_ms);
        runtime
    }

    pub fn run(mut self, requests: Receiver<Request>) {
        log!("agent {} listening on 127.0.0.1:{} ({})", flash_core::VERSION, self.port, self.display);
        self.publish_status();
        let mut last_tick = Instant::now();
        let mut next_periodic = last_tick + TICK;
        while !self.quitting {
            // A deadline the last tick could not act on (a reminder during quiet
            // hours, say) stays in the past; ignore it rather than spin on it.
            let deadline = self.deadline().filter(|d| *d > last_tick);
            let wake = deadline.map_or(next_periodic, |d| d.min(next_periodic));
            match requests.recv_timeout(wake.saturating_duration_since(Instant::now())) {
                Ok(request) => self.handle(request),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            let now = Instant::now();
            let periodic = now >= next_periodic;
            if periodic {
                self.reload_if_changed();
                next_periodic = now + TICK;
            }
            if periodic || deadline.is_some_and(|d| now >= d) {
                self.dispatch(Input::Tick);
                last_tick = now;
            }
        }
        self.persist();
        self.ui.send(UiEvent::Quit);
        log!("agent stopped");
    }

    fn deadline(&self) -> Option<Instant> {
        self.engine.next_deadline().map(|ms| self.epoch + Duration::from_millis(ms))
    }

    fn handle(&mut self, request: Request) {
        match request {
            Request::Hook(event) => {
                self.last_event = Some(LastEvent {
                    event: event.name.clone(),
                    project: event.cwd.as_deref().map(glob::basename).filter(|b| !b.is_empty()).map(str::to_owned),
                    ts: time::rfc3339(system::unix_ms()),
                });
                self.dispatch(Input::Hook(event));
            }
            Request::Signal(signal) => self.dispatch(Input::Signal(signal)),
            Request::Control(control, reply) => {
                let result = self.control(control).map(|()| self.status());
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
            }
            Request::Status(reply) => {
                let _ = reply.send(self.status());
            }
        }
    }

    fn control(&mut self, control: Control) -> Result<(), String> {
        let switch = match control {
            Control::On => Switch::Enable,
            Control::Off => Switch::Disable,
            Control::Toggle { confirm } => {
                let on = !self.engine.is_enabled();
                self.dispatch(Input::Control(if on { Switch::Enable } else { Switch::Disable }));
                if confirm && on {
                    self.dispatch(Input::Control(Switch::Test(Attention::Done)));
                } else if confirm {
                    self.ui.send(UiEvent::Notify(Notice {
                        kind: Attention::Done,
                        title: "Claude Flash is off".to_owned(),
                        body: "No flashes until it is turned back on.".to_owned(),
                    }));
                }
                return Ok(());
            }
            Control::Pause { duration } => match duration::parse(&duration).map_err(|e| e.to_string())? {
                span if span.is_zero() => Switch::Resume,
                span => Switch::Pause { for_ms: span.ms() },
            },
            Control::Resume => Switch::Resume,
            Control::Test { kind } => Switch::Test(kind),
            Control::Reload => {
                self.config_stamp = file_stamp(&self.paths.config_file());
                self.reload();
                return self.config_error.clone().map_or(Ok(()), Err);
            }
            Control::Quit => {
                log!("quit requested");
                self.quitting = true;
                return Ok(());
            }
        };
        self.dispatch(Input::Control(switch));
        Ok(())
    }

    fn dispatch(&mut self, input: Input) {
        let ctx = self.context();
        let mut changed = false;
        for action in self.engine.handle(input, &ctx) {
            match action {
                Action::Flash(spec) => {
                    self.today(&ctx).flashes += 1;
                    self.ui.send(UiEvent::Flash(spec));
                }
                Action::Notify(notice) => self.ui.send(UiEvent::Notify(notice)),
                Action::Push(notice) => push::send(&self.engine.config().push, &notice),
                Action::Record(record) => self.record(record, &ctx),
                Action::StateChanged | Action::SessionsChanged => changed = true,
            }
        }
        if changed {
            self.persist();
            self.publish_status();
        }
    }

    fn context(&self) -> Context {
        let wants_focus = !self.engine.config().flash.skip_when_focused.is_empty();
        Context {
            now_ms: self.now_ms(),
            unix_ms: system::unix_ms(),
            utc_offset_min: system::utc_offset_minutes(),
            idle_ms: self.probe.idle_ms().unwrap_or(0),
            focused_app: if wants_focus { self.probe.focused_app() } else { None },
        }
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    fn today(&mut self, ctx: &Context) -> &mut Today {
        let date = local_date(ctx.unix_ms, ctx.utc_offset_min);
        if self.today.0 != date {
            self.today = (date, Today::default());
        }
        &mut self.today.1
    }

    fn record(&mut self, record: Record, ctx: &Context) {
        let today = self.today(ctx);
        if record.is_signal()
            && record.event != "test"
            && let Some(kind) = record.kind
        {
            today.count(kind);
        }
        if record.suppressed.is_some() {
            today.suppressed += 1;
        }
        if self.engine.config().journal.enabled {
            match self.journal.append(&record, ctx.unix_ms, ctx.utc_offset_min) {
                Ok(()) => self.journal_failing = false,
                Err(e) if !self.journal_failing => {
                    log!("could not write the journal: {e}");
                    self.journal_failing = true;
                }
                Err(_) => {}
            }
        }
        self.hub.publish(record);
    }

    fn status(&self) -> Status {
        let ctx = Context { now_ms: self.now_ms(), unix_ms: system::unix_ms(), ..Context::default() };
        let snapshot = self.engine.snapshot(&ctx);
        let today = if self.today.0 == local_date(ctx.unix_ms, system::utc_offset_minutes()) {
            self.today.1.clone()
        } else {
            Today::default()
        };
        let config = self.engine.config();
        Status {
            version: flash_core::VERSION.to_owned(),
            pid: std::process::id(),
            port: self.port,
            started: time::rfc3339(self.started_unix_ms),
            display: self.display.to_owned(),
            enabled: snapshot.enabled,
            paused_for_ms: snapshot.paused_for_ms,
            away: snapshot.away,
            waiting: snapshot.waiting,
            today,
            config_path: self.paths.config_file().display().to_string(),
            config_error: self.config_error.clone(),
            journal_path: config.journal.enabled.then(|| self.paths.journal_dir().display().to_string()),
            last_event: self.last_event.clone(),
            opted_out: self.shared.opted_out.load(Ordering::Relaxed),
        }
    }

    fn publish_status(&self) {
        self.ui.send(UiEvent::Status(Box::new(self.status())));
    }

    /// Saves the switches and known sessions when they differ from what is on disk.
    fn persist(&mut self) {
        let ctx = Context { now_ms: self.now_ms(), unix_ms: system::unix_ms(), ..Context::default() };
        let state = State {
            enabled: self.engine.is_enabled(),
            // Whole seconds, so the same pause does not look new on every save.
            paused_until: self.engine.pause_remaining(&ctx).map(|ms| (ctx.unix_ms + ms).div_ceil(1000) * 1000),
            interactive: self.engine.interactive_sessions().clone(),
        };
        if state != self.state {
            // Only a saved state counts as saved, so a failed write is tried again.
            match state.save(&self.paths.state_file()) {
                Ok(()) => {
                    self.state = state;
                    self.state_failing = false;
                }
                Err(e) if !self.state_failing => {
                    log!("could not save {}: {e}", self.paths.state_file().display());
                    self.state_failing = true;
                }
                Err(_) => {}
            }
        }
    }

    fn reload_if_changed(&mut self) {
        let stamp = file_stamp(&self.paths.config_file());
        if stamp != self.config_stamp {
            self.config_stamp = stamp;
            self.reload();
        }
    }

    /// Applies the configuration file. One that does not parse leaves the running
    /// configuration in place, and the problem is reported in the status.
    fn reload(&mut self) {
        let path = self.paths.config_file();
        let parsed = match fs::read_to_string(&path) {
            Ok(text) => Config::parse(&text).map_err(|e| e.to_string()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(format!("could not read {}: {e}", path.display())),
        };
        match parsed {
            Ok(config) => {
                if config.agent.port != self.port {
                    log!(
                        "agent.port is now {}; the agent keeps listening on {} until it restarts",
                        config.agent.port,
                        self.port
                    );
                }
                self.journal.set_retention(config.journal.retain_days);
                self.engine.set_config(config);
                self.config_error = None;
                log!("configuration reloaded");
            }
            Err(message) => {
                if self.config_error.as_deref() != Some(message.as_str()) {
                    log!("configuration not applied: {message}");
                }
                self.config_error = Some(message);
            }
        }
        self.publish_status();
    }
}

/// Reads the configuration, writing the commented default file on first run.
fn load_config(path: &Path) -> (Config, Option<String>) {
    match fs::read_to_string(path) {
        Ok(text) => match Config::parse(&text) {
            Ok(config) => (config, None),
            Err(e) => (Config::default(), Some(e.to_string())),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if let Err(e) = store::write_atomic(path, config::DEFAULT_TOML.as_bytes()) {
                log!("could not write the default configuration to {}: {e}", path.display());
            }
            (Config::default(), None)
        }
        Err(e) => (Config::default(), Some(format!("could not read {}: {e}", path.display()))),
    }
}

/// Modification time and size: enough to notice an edit, including one that lands
/// within the file system's timestamp resolution.
fn file_stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn local_date(unix_ms: u64, utc_offset_min: i32) -> String {
    let (y, m, d) = time::local_date(unix_ms, utc_offset_min);
    format!("{y:04}-{m:02}-{d:02}")
}
