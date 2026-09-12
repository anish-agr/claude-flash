//! The policy engine.
//!
//! Given an input — a hook event, a signal from the local API, a control command or
//! a timer tick — and a [`Context`] describing the moment, the engine decides what
//! to show and what to record. It performs no I/O and reads no clocks, which is what
//! lets the scenarios in `spec/scenarios` pin its behaviour down exactly, identically
//! on every platform.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::color::Rgb;
use crate::config::{Config, MIN_FLASH_INTERVAL_MS, ProjectRule, SignalStyle};
use crate::duration;
use crate::event::{self, Attention, Effect, HookEvent, Signal};
use crate::glob;
use crate::journal::{Channel, Reason, Record};
use crate::render::{Style, Timing};
use crate::time::{self, Clock};

/// A permission request and Claude Code's notification about the same dialog arrive
/// as separate events; within this window they count as one wait.
const DUPLICATE_WINDOW_MS: u64 = 5_000;
/// Sessions with no activity and nothing outstanding are forgotten after this long.
const SESSION_TTL_MS: u64 = 12 * 3_600_000;
/// A wait this old belongs to a session that died mid-dialog; it is dropped.
const WAIT_TTL_MS: u64 = 24 * 3_600_000;
/// How long a session stays known as interactive, across agent restarts.
pub const INTERACTIVE_TTL_MS: u64 = 7 * 86_400_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context {
    /// Monotonic milliseconds. Only differences matter.
    pub now_ms: u64,
    /// Wall-clock Unix milliseconds, for the journal and quiet hours.
    pub unix_ms: u64,
    /// Local offset from UTC, in minutes.
    pub utc_offset_min: i32,
    /// Milliseconds since the last keyboard or mouse input anywhere on the system.
    pub idle_ms: u64,
    /// Name of the application that owns the focused window, when known.
    pub focused_app: Option<String>,
}

impl Context {
    pub fn local_clock(&self) -> Clock {
        let minutes = (self.unix_ms / 60_000) as i64 + i64::from(self.utc_offset_min);
        Clock(minutes.rem_euclid(1_440) as u16)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Hook(HookEvent),
    Signal(Signal),
    Control(Control),
    /// A timer wake-up. Presence changes and deadlines are noticed on ticks.
    Tick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Enable,
    Disable,
    /// Suppress signals for this long. Zero resumes.
    Pause {
        for_ms: u64,
    },
    Resume,
    /// Show a signal now, whatever the pause, presence or quiet hours say. It still
    /// respects the minimum flash interval, which is a safety limit, not a preference.
    Test(Attention),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlashSpec {
    pub kind: Attention,
    pub color: Rgb,
    pub opacity: f32,
    pub style: Style,
    pub vignette: f32,
    pub timing: Timing,
    pub respect_reduce_motion: bool,
    /// Play this signal's system sound as the flash starts.
    pub sound: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub kind: Attention,
    pub title: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Flash(FlashSpec),
    Notify(Notice),
    Push(Notice),
    Record(Record),
    /// Pause, presence or outstanding waits changed; refresh any status display.
    StateChanged,
    /// The set of interactive sessions changed; persist it.
    SessionsChanged,
}

/// A one-line description of a delivery or suppression — `flash approval`,
/// `notify done`, `suppressed paused` — or `None` for bookkeeping.
pub fn describe(action: &Action) -> Option<String> {
    match action {
        Action::Flash(spec) => Some(format!("flash {}", spec.kind)),
        Action::Notify(n) => Some(format!("notify {}", n.kind)),
        Action::Push(n) => Some(format!("push {}", n.kind)),
        Action::Record(r) => r.suppressed.map(|reason| format!("suppressed {}", reason.as_str())),
        Action::StateChanged | Action::SessionsChanged => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub enabled: bool,
    pub paused_for_ms: Option<u64>,
    pub away: bool,
    /// Everything Claude is currently blocked on, most urgent first.
    pub waiting: Vec<Waiting>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waiting {
    pub kind: Attention,
    pub project: String,
    pub session: String,
    pub tool: Option<String>,
    pub for_ms: u64,
}

#[derive(Clone, Debug)]
struct Wait {
    kind: Attention,
    since_ms: u64,
    /// The `tool_use_id` this wait is for, when the event carried one.
    id: Option<String>,
    tool: Option<String>,
    reminded_ms: Option<u64>,
}

#[derive(Clone, Debug)]
struct Session {
    project: String,
    cwd: Option<String>,
    waits: Vec<Wait>,
    last_seen_ms: u64,
}

#[derive(Clone, Debug, Default)]
struct Digest {
    counts: BTreeMap<Attention, u32>,
}

impl Digest {
    fn add(&mut self, kind: Attention) {
        *self.counts.entry(kind).or_default() += 1;
    }

    fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    fn strongest(&self) -> Option<Attention> {
        self.counts.keys().next_back().copied()
    }

    fn summary(&self) -> String {
        let parts: Vec<String> = self
            .counts
            .iter()
            .rev()
            .map(|(kind, &n)| {
                let noun = match (kind, n) {
                    (Attention::Done, _) => "finished",
                    (Attention::Error, 1) => "error",
                    (Attention::Error, _) => "errors",
                    (Attention::Question, 1) => "question",
                    (Attention::Question, _) => "questions",
                    (Attention::Approval, 1) => "approval",
                    (Attention::Approval, _) => "approvals",
                };
                format!("{n} {noun}")
            })
            .collect();
        parts.join(", ")
    }
}

#[derive(Clone, Debug)]
struct Deferred {
    at_ms: u64,
    spec: FlashSpec,
}

/// What became of a request to flash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Requested {
    Shown,
    /// Held until the minimum interval ends.
    Queued,
    /// A flash of at least the same urgency has just started, and covers it.
    Covered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    Interactive,
    Background,
    Api,
}

/// What happened to one signal.
#[derive(Default)]
struct Outcome {
    delivered: Vec<Channel>,
    suppressed: Option<Reason>,
}

impl Outcome {
    fn apply(self, record: &mut Record) {
        record.delivered = self.delivered;
        record.suppressed = self.suppressed;
    }
}

pub struct Engine {
    config: Config,
    enabled: bool,
    paused_until_ms: Option<u64>,
    sessions: HashMap<String, Session>,
    /// Sessions the user has typed a prompt into, with when they last did.
    interactive: BTreeMap<String, u64>,
    away: bool,
    digest: Digest,
    last_flash: Option<(u64, Attention)>,
    deferred: Option<Deferred>,
}

impl Engine {
    pub fn new(config: Config) -> Self {
        Engine {
            config,
            enabled: true,
            paused_until_ms: None,
            sessions: HashMap::new(),
            interactive: BTreeMap::new(),
            away: false,
            digest: Digest::default(),
            last_flash: None,
            deferred: None,
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn set_config(&mut self, config: Config) {
        self.config = config;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn is_away(&self) -> bool {
        self.away
    }

    pub fn pause_remaining(&self, ctx: &Context) -> Option<u64> {
        self.paused_until_ms.filter(|&t| t > ctx.now_ms).map(|t| t - ctx.now_ms)
    }

    /// Reapplies persisted switches after the agent starts.
    pub fn restore(&mut self, enabled: bool, pause_remaining_ms: Option<u64>, ctx: &Context) {
        self.enabled = enabled;
        self.paused_until_ms = pause_remaining_ms.filter(|&ms| ms > 0).map(|ms| ctx.now_ms.saturating_add(ms));
    }

    pub fn interactive_sessions(&self) -> &BTreeMap<String, u64> {
        &self.interactive
    }

    pub fn restore_interactive(&mut self, sessions: BTreeMap<String, u64>, unix_now_ms: u64) {
        let cutoff = unix_now_ms.saturating_sub(INTERACTIVE_TTL_MS);
        self.interactive = sessions.into_iter().filter(|(_, seen)| *seen >= cutoff).collect();
    }

    pub fn handle(&mut self, input: Input, ctx: &Context) -> Vec<Action> {
        let mut out = Vec::new();
        self.expire_pause(ctx, &mut out);
        self.update_presence(ctx, &mut out);
        match input {
            Input::Hook(ev) => self.on_hook(&ev, ctx, &mut out),
            Input::Signal(signal) => self.on_signal(signal, ctx, &mut out),
            Input::Control(control) => self.on_control(control, ctx, &mut out),
            Input::Tick => {}
        }
        self.fire_deferred(ctx, &mut out);
        self.remind(ctx, &mut out);
        self.forget_stale(ctx);
        out
    }

    /// When the engine next needs a [`Input::Tick`], in monotonic milliseconds.
    /// Presence is sampled by periodic ticks, so it is not reflected here.
    pub fn next_deadline(&self) -> Option<u64> {
        let mut soonest = self.paused_until_ms;
        let mut consider = |t: u64| soonest = Some(soonest.map_or(t, |s| s.min(t)));
        if let Some(deferred) = &self.deferred {
            consider(deferred.at_ms);
        }
        let every = self.config.flash.remind_after.ms();
        if every > 0 {
            for (id, session) in &self.sessions {
                if self.visible(id) {
                    for wait in &session.waits {
                        consider(wait.reminded_ms.unwrap_or(wait.since_ms).saturating_add(every));
                    }
                }
            }
        }
        soonest
    }

    pub fn snapshot(&self, ctx: &Context) -> Snapshot {
        Snapshot {
            enabled: self.enabled,
            paused_for_ms: self.pause_remaining(ctx),
            away: self.away,
            waiting: self.waiting(ctx),
        }
    }

    fn on_hook(&mut self, ev: &HookEvent, ctx: &Context, out: &mut Vec<Action>) {
        let effect = event::classify(ev);
        if effect == Effect::Ignored {
            return;
        }
        self.touch(ev, ctx);
        match effect {
            Effect::Prompted => {
                self.resolve(ev, ctx, out, |_| true);
                self.interactive.insert(ev.session_id.clone(), ctx.unix_ms);
                out.push(Action::SessionsChanged);
                out.push(Action::Record(self.session_record(ctx, &ev.name, &ev.session_id)));
            }
            Effect::Progress { tool_use_id } => {
                self.resolve(ev, ctx, out, |w| match &tool_use_id {
                    Some(id) => w.id.as_deref() == Some(id.as_str()),
                    None => w.id.is_none(),
                });
            }
            Effect::SessionStarted => out.push(Action::Record(self.session_record(ctx, &ev.name, &ev.session_id))),
            Effect::SessionEnded => {
                self.resolve(ev, ctx, out, |_| true);
                let record = self.session_record(ctx, &ev.name, &ev.session_id);
                self.sessions.remove(&ev.session_id);
                out.push(Action::Record(record));
                out.push(Action::StateChanged);
            }
            Effect::Signal { kind, wait } => self.on_attention(ev, kind, wait, ctx, out),
            Effect::Ignored => {}
        }
    }

    fn on_attention(
        &mut self,
        ev: &HookEvent,
        kind: Attention,
        wait: Option<String>,
        ctx: &Context,
        out: &mut Vec<Action>,
    ) {
        if !kind.blocks_claude() {
            // The turn is over, so anything it was waiting on has been settled.
            self.resolve(ev, ctx, out, |_| true);
        }
        let mut record = self.session_record(ctx, &ev.name, &ev.session_id);
        record.kind = Some(kind);
        record.tool = ev.tool_name.clone();

        if kind.blocks_claude() {
            let now = ctx.now_ms;
            let session = self.sessions.get_mut(&ev.session_id).expect("session is touched before it signals");
            let duplicate = session.waits.iter_mut().find(|w| {
                w.kind == kind
                    && now.saturating_sub(w.since_ms) <= DUPLICATE_WINDOW_MS
                    && (w.id.is_none() || wait.is_none() || w.id == wait)
            });
            if let Some(existing) = duplicate {
                if existing.id.is_none() {
                    existing.id = wait;
                }
                if existing.tool.is_none() {
                    existing.tool.clone_from(&ev.tool_name);
                }
                record.suppressed = Some(Reason::Duplicate);
                out.push(Action::Record(record));
                return;
            }
            session.waits.push(Wait { kind, since_ms: now, id: wait, tool: ev.tool_name.clone(), reminded_ms: None });
            out.push(Action::StateChanged);
        }

        let (project, cwd) = self
            .sessions
            .get(&ev.session_id)
            .map(|s| (s.project.clone(), s.cwd.clone()))
            .unwrap_or_else(|| (project_name(ev.cwd.as_deref()), ev.cwd.clone()));
        let rule = self.config.rule_for(cwd.as_deref(), &project).cloned();
        let origin =
            if self.interactive.contains_key(&ev.session_id) { Origin::Interactive } else { Origin::Background };
        let notice = hook_notice(kind, &project, ev.tool_name.as_deref());
        self.deliver(kind, rule.as_ref(), origin, notice, ctx, out).apply(&mut record);
        out.push(Action::Record(record));
    }

    fn on_signal(&mut self, signal: Signal, ctx: &Context, out: &mut Vec<Action>) {
        let signal = signal.bounded();
        let kind = signal.kind;
        let project = signal.project.clone().or_else(|| signal.source.clone()).unwrap_or_else(|| "signal".to_owned());
        let rule = self.config.rule_for(None, &project).cloned();
        let mut record = self.record(ctx, "signal");
        record.kind = Some(kind);
        record.project = Some(project.clone());
        record.source.clone_from(&signal.source);
        let notice = Notice {
            kind,
            title: signal.title.unwrap_or_else(|| default_title(kind).to_owned()),
            body: signal.body.unwrap_or(project),
        };
        self.deliver(kind, rule.as_ref(), Origin::Api, notice, ctx, out).apply(&mut record);
        out.push(Action::Record(record));
    }

    fn on_control(&mut self, control: Control, ctx: &Context, out: &mut Vec<Action>) {
        let record = match control {
            Control::Enable => {
                self.enabled = true;
                self.record(ctx, "enable")
            }
            Control::Disable => {
                self.enabled = false;
                self.deferred = None;
                self.record(ctx, "disable")
            }
            Control::Pause { for_ms } if for_ms > 0 => {
                self.paused_until_ms = Some(ctx.now_ms.saturating_add(for_ms));
                self.deferred = None;
                let mut r = self.record(ctx, "pause");
                r.detail = Some(duration::format(for_ms));
                r
            }
            Control::Pause { .. } | Control::Resume => {
                self.paused_until_ms = None;
                self.record(ctx, "resume")
            }
            Control::Test(kind) => {
                let mut r = self.record(ctx, "test");
                r.kind = Some(kind);
                let spec = self.flash_spec(kind, self.config.signal(kind));
                if self.request_flash(spec, ctx, out) != Requested::Covered {
                    r.delivered.push(Channel::Flash);
                }
                r
            }
        };
        out.push(Action::Record(record));
        out.push(Action::StateChanged);
    }

    /// Decides how, or whether, a raised signal reaches the user.
    fn deliver(
        &mut self,
        kind: Attention,
        rule: Option<&ProjectRule>,
        origin: Origin,
        notice: Notice,
        ctx: &Context,
        out: &mut Vec<Action>,
    ) -> Outcome {
        let style = self.config.signal_for(kind, rule);
        let blocked = if !self.enabled {
            Some(Reason::Disabled)
        } else if self.paused(ctx) {
            Some(Reason::Paused)
        } else if rule.is_some_and(|r| r.mute) {
            Some(Reason::Muted)
        } else if !style.enabled {
            Some(Reason::SignalOff)
        } else if origin == Origin::Background && self.config.sessions.ignore_background && !self.interactive.is_empty()
        {
            // With no interactive session on record at all, fail open: silence is the
            // worse failure for a tool whose whole job is to be noticed.
            Some(Reason::Background)
        } else {
            None
        };
        let mut outcome = Outcome { suppressed: blocked, ..Outcome::default() };
        if blocked.is_some() {
            return outcome;
        }
        if self.quiet(ctx) {
            if self.config.quiet_hours.notify {
                outcome.delivered.push(Channel::Notify);
                out.push(Action::Notify(notice));
            }
            outcome.suppressed = Some(Reason::QuietHours);
            return outcome;
        }
        if self.away {
            self.digest.add(kind);
            outcome.delivered.push(Channel::Digest);
            if self.config.presence.notify_when_away {
                outcome.delivered.push(Channel::Notify);
                out.push(Action::Notify(notice.clone()));
            }
            let push = &self.config.push;
            if push.enabled() && push.kinds.contains(&kind) && ctx.idle_ms >= push.after.ms() {
                outcome.delivered.push(Channel::Push);
                out.push(Action::Push(notice));
            }
            return outcome;
        }
        if self.focused_skip(ctx) {
            outcome.suppressed = Some(Reason::Focused);
            return outcome;
        }
        let spec = self.flash_spec(kind, style);
        if self.request_flash(spec, ctx, out) != Requested::Covered {
            outcome.delivered.push(Channel::Flash);
        }
        outcome
    }

    /// Starts a flash, unless one started less than the minimum interval ago. A
    /// stronger signal arriving inside the interval is shown when it ends; anything
    /// else was already covered by the flash that just played. However many events
    /// arrive, flashes stay at least the interval apart.
    fn request_flash(&mut self, spec: FlashSpec, ctx: &Context, out: &mut Vec<Action>) -> Requested {
        // A queued flash that has come due goes first, so two never start together.
        self.fire_deferred(ctx, out);
        let interval = u64::from(self.config.flash.min_interval_ms.max(MIN_FLASH_INTERVAL_MS));
        match self.last_flash {
            Some((at, shown)) if ctx.now_ms < at.saturating_add(interval) => {
                if let Some(deferred) = self.deferred.as_mut() {
                    if spec.kind > deferred.spec.kind {
                        deferred.spec = spec;
                    }
                    Requested::Queued
                } else if spec.kind > shown {
                    self.deferred = Some(Deferred { at_ms: at + interval, spec });
                    Requested::Queued
                } else {
                    Requested::Covered
                }
            }
            _ => {
                self.last_flash = Some((ctx.now_ms, spec.kind));
                out.push(Action::Flash(spec));
                Requested::Shown
            }
        }
    }

    fn fire_deferred(&mut self, ctx: &Context, out: &mut Vec<Action>) {
        if self.deferred.as_ref().is_none_or(|d| ctx.now_ms < d.at_ms) {
            return;
        }
        let Some(deferred) = self.deferred.take() else { return };
        if !self.enabled || self.paused(ctx) || self.quiet(ctx) || self.away || self.focused_skip(ctx) {
            return;
        }
        self.last_flash = Some((ctx.now_ms, deferred.spec.kind));
        out.push(Action::Flash(deferred.spec));
    }

    fn expire_pause(&mut self, ctx: &Context, out: &mut Vec<Action>) {
        if self.paused_until_ms.is_some_and(|t| ctx.now_ms >= t) {
            self.paused_until_ms = None;
            out.push(Action::Record(self.record(ctx, "resume")));
            out.push(Action::StateChanged);
        }
    }

    fn update_presence(&mut self, ctx: &Context, out: &mut Vec<Action>) {
        let threshold = self.config.presence.away_after.ms();
        let away = threshold > 0 && ctx.idle_ms >= threshold;
        match (self.away, away) {
            (false, true) => {
                self.away = true;
                self.digest = Digest::default();
                out.push(Action::StateChanged);
            }
            (true, false) => {
                self.away = false;
                out.push(Action::StateChanged);
                self.welcome_back(ctx, out);
            }
            _ => {}
        }
    }

    /// On return, one flash in the colour of the most urgent signal among the waits
    /// still open and the signals that arrived while the user was gone.
    fn welcome_back(&mut self, ctx: &Context, out: &mut Vec<Action>) {
        let digest = std::mem::take(&mut self.digest);
        if !self.config.presence.digest_on_return
            || !self.enabled
            || self.paused(ctx)
            || self.quiet(ctx)
            || self.focused_skip(ctx)
        {
            return;
        }
        let pending = self.waiting(ctx).into_iter().next();
        let Some(kind) = pending.as_ref().map(|w| w.kind).max(digest.strongest()) else { return };
        let mut record = self.record(ctx, "digest");
        record.kind = Some(kind);
        let mut detail = Vec::new();
        if !digest.is_empty() {
            detail.push(format!("while away: {}", digest.summary()));
        }
        if let Some(w) = &pending {
            detail.push(format!("still waiting: {} in {} for {}", w.kind, w.project, duration::format(w.for_ms)));
        }
        record.detail = Some(detail.join("; "));
        let spec = self.flash_spec(kind, self.config.signal(kind));
        if self.request_flash(spec, ctx, out) != Requested::Covered {
            record.delivered.push(Channel::Flash);
        }
        out.push(Action::Record(record));
    }

    fn remind(&mut self, ctx: &Context, out: &mut Vec<Action>) {
        let every = self.config.flash.remind_after.ms();
        // Returning early leaves each wait's reminder due, so it flashes once nothing
        // stands in the way.
        if every == 0 || self.away || !self.enabled || self.paused(ctx) || self.quiet(ctx) || self.focused_skip(ctx) {
            return;
        }
        let now = ctx.now_ms;
        let eligible: Vec<String> = self
            .sessions
            .iter()
            .filter(|(id, s)| self.visible(id) && !self.muted(s))
            .map(|(id, _)| id.clone())
            .collect();
        let mut due: Option<Attention> = None;
        for id in eligible {
            let Some(session) = self.sessions.get_mut(&id) else { continue };
            for wait in &mut session.waits {
                if self.config.signal(wait.kind).enabled
                    && now.saturating_sub(wait.reminded_ms.unwrap_or(wait.since_ms)) >= every
                {
                    wait.reminded_ms = Some(now);
                    due = due.max(Some(wait.kind));
                }
            }
        }
        let Some(kind) = due else { return };
        let mut record = self.record(ctx, "reminder");
        record.kind = Some(kind);
        let spec = self.flash_spec(kind, self.config.signal(kind));
        if self.request_flash(spec, ctx, out) != Requested::Covered {
            record.delivered.push(Channel::Flash);
        }
        out.push(Action::Record(record));
    }

    fn forget_stale(&mut self, ctx: &Context) {
        let now = ctx.now_ms;
        for session in self.sessions.values_mut() {
            session.waits.retain(|w| now.saturating_sub(w.since_ms) < WAIT_TTL_MS);
        }
        self.sessions.retain(|_, s| !s.waits.is_empty() || now.saturating_sub(s.last_seen_ms) < SESSION_TTL_MS);
        let cutoff = ctx.unix_ms.saturating_sub(INTERACTIVE_TTL_MS);
        self.interactive.retain(|_, seen| *seen >= cutoff);
    }

    fn touch(&mut self, ev: &HookEvent, ctx: &Context) {
        let session = self.sessions.entry(ev.session_id.clone()).or_insert_with(|| Session {
            project: project_name(ev.cwd.as_deref()),
            cwd: ev.cwd.clone(),
            waits: Vec::new(),
            last_seen_ms: ctx.now_ms,
        });
        session.last_seen_ms = ctx.now_ms;
        if let Some(cwd) = &ev.cwd
            && session.cwd.as_deref() != Some(cwd.as_str())
        {
            session.project = project_name(Some(cwd));
            session.cwd = Some(cwd.clone());
        }
    }

    /// Ends the waits in the event's session that `matches` selects, recording how
    /// long each one lasted.
    fn resolve(&mut self, ev: &HookEvent, ctx: &Context, out: &mut Vec<Action>, matches: impl Fn(&Wait) -> bool) {
        let Some(session) = self.sessions.get_mut(&ev.session_id) else { return };
        let mut ended = Vec::new();
        session.waits.retain(|w| {
            let hit = matches(w);
            if hit {
                ended.push(w.clone());
            }
            !hit
        });
        if ended.is_empty() {
            return;
        }
        for wait in ended {
            let mut record = self.session_record(ctx, &ev.name, &ev.session_id);
            record.kind = Some(wait.kind);
            record.tool = wait.tool;
            record.waited_ms = Some(ctx.now_ms.saturating_sub(wait.since_ms));
            out.push(Action::Record(record));
        }
        out.push(Action::StateChanged);
    }

    fn waiting(&self, ctx: &Context) -> Vec<Waiting> {
        let mut items: Vec<Waiting> = self
            .sessions
            .iter()
            .filter(|(id, s)| self.visible(id) && !self.muted(s))
            .flat_map(|(id, s)| {
                s.waits.iter().filter(|w| self.config.signal(w.kind).enabled).map(move |w| Waiting {
                    kind: w.kind,
                    project: s.project.clone(),
                    session: event::short_id(id).to_owned(),
                    tool: w.tool.clone(),
                    for_ms: ctx.now_ms.saturating_sub(w.since_ms),
                })
            })
            .collect();
        items.sort_by(|a, b| b.kind.cmp(&a.kind).then(b.for_ms.cmp(&a.for_ms)).then_with(|| a.session.cmp(&b.session)));
        items
    }

    fn visible(&self, session_id: &str) -> bool {
        !self.config.sessions.ignore_background
            || self.interactive.is_empty()
            || self.interactive.contains_key(session_id)
    }

    fn muted(&self, session: &Session) -> bool {
        self.config.rule_for(session.cwd.as_deref(), &session.project).is_some_and(|r| r.mute)
    }

    fn paused(&self, ctx: &Context) -> bool {
        self.paused_until_ms.is_some_and(|t| ctx.now_ms < t)
    }

    fn quiet(&self, ctx: &Context) -> bool {
        self.config.quiet_hours.window().is_some_and(|(start, end)| time::in_window(ctx.local_clock(), start, end))
    }

    fn focused_skip(&self, ctx: &Context) -> bool {
        let Some(app) = ctx.focused_app.as_deref() else { return false };
        let app = strip_exe(app.trim());
        self.config.flash.skip_when_focused.iter().any(|name| strip_exe(name.trim()).eq_ignore_ascii_case(app))
    }

    fn flash_spec(&self, kind: Attention, style: SignalStyle) -> FlashSpec {
        let f = &self.config.flash;
        FlashSpec {
            kind,
            color: style.color,
            opacity: style.opacity as f32,
            style: f.style,
            vignette: f.vignette as f32,
            timing: f.timing(),
            respect_reduce_motion: f.respect_reduce_motion,
            sound: style.sound,
        }
    }

    fn record(&self, ctx: &Context, event: &str) -> Record {
        Record { ts: time::rfc3339(ctx.unix_ms), tz: ctx.utc_offset_min, event: event.to_owned(), ..Record::default() }
    }

    fn session_record(&self, ctx: &Context, event: &str, session_id: &str) -> Record {
        let mut record = self.record(ctx, event);
        record.session = Some(event::short_id(session_id).to_owned());
        if let Some(session) = self.sessions.get(session_id) {
            record.project = Some(session.project.clone());
            if self.config.journal.store_paths {
                record.path.clone_from(&session.cwd);
            }
        }
        record
    }
}

fn project_name(cwd: Option<&str>) -> String {
    cwd.map(glob::basename).filter(|b| !b.is_empty()).unwrap_or("unknown").to_owned()
}

fn strip_exe(name: &str) -> &str {
    match name.len().checked_sub(4).and_then(|i| name.get(i..).map(|tail| (i, tail))) {
        Some((i, tail)) if tail.eq_ignore_ascii_case(".exe") => &name[..i],
        _ => name,
    }
}

pub fn default_title(kind: Attention) -> &'static str {
    match kind {
        Attention::Done => "Claude finished",
        Attention::Error => "Claude stopped with an error",
        Attention::Question => "Claude has a question",
        Attention::Approval => "Claude needs approval",
    }
}

fn hook_notice(kind: Attention, project: &str, tool: Option<&str>) -> Notice {
    let body = match (kind, tool) {
        (Attention::Approval, Some(tool)) => format!("{tool} in {project}"),
        _ => project.to_owned(),
    };
    Notice { kind, title: default_title(kind).to_owned(), body }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-09-10T12:00:00Z
    const BASE_UNIX: u64 = 1_789_041_600_000;

    fn at(now_ms: u64) -> Context {
        Context { now_ms, unix_ms: BASE_UNIX + now_ms, ..Context::default() }
    }

    fn hook(name: &str, session: &str) -> HookEvent {
        HookEvent {
            name: name.into(),
            session_id: session.into(),
            cwd: Some("/work/app".into()),
            permission_mode: Some("default".into()),
            tool_name: None,
            tool_use_id: None,
            notification_type: None,
            agent_id: None,
        }
    }

    fn tool(mut ev: HookEvent, name: &str, id: &str) -> HookEvent {
        ev.tool_name = Some(name.into());
        ev.tool_use_id = Some(id.into());
        ev
    }

    fn run(engine: &mut Engine, input: Input, ctx: &Context) -> Vec<String> {
        engine.handle(input, ctx).iter().filter_map(describe).collect()
    }

    fn prompt(engine: &mut Engine, session: &str, now: u64) {
        run(engine, Input::Hook(hook("UserPromptSubmit", session)), &at(now));
    }

    #[test]
    fn finished_turn_in_an_interactive_session_flashes() {
        let mut e = Engine::new(Config::default());
        prompt(&mut e, "s1", 0);
        assert_eq!(run(&mut e, Input::Hook(hook("Stop", "s1")), &at(5_000)), ["flash done"]);
    }

    #[test]
    fn background_sessions_are_ignored_once_any_session_is_known() {
        let mut e = Engine::new(Config::default());
        assert_eq!(
            run(&mut e, Input::Hook(hook("Stop", "agent")), &at(0)),
            ["flash done"],
            "fails open with no history"
        );
        prompt(&mut e, "s1", 10_000);
        assert_eq!(run(&mut e, Input::Hook(hook("Stop", "agent")), &at(20_000)), ["suppressed background"]);
    }

    #[test]
    fn approvals_record_how_long_claude_waited() {
        let mut e = Engine::new(Config::default());
        prompt(&mut e, "s1", 0);
        assert_eq!(
            run(&mut e, Input::Hook(tool(hook("PermissionRequest", "s1"), "Bash", "t1")), &at(1_000)),
            ["flash approval"]
        );
        assert_eq!(e.snapshot(&at(2_000)).waiting.len(), 1);
        let actions = e.handle(Input::Hook(tool(hook("PostToolUse", "s1"), "Bash", "t1")), &at(15_000));
        let waited: Vec<u64> =
            actions.iter().filter_map(|a| if let Action::Record(r) = a { r.waited_ms } else { None }).collect();
        assert_eq!(waited, [14_000]);
        assert!(e.snapshot(&at(16_000)).waiting.is_empty());
    }

    #[test]
    fn parallel_waits_resolve_independently() {
        let mut e = Engine::new(Config::default());
        prompt(&mut e, "s1", 0);
        run(&mut e, Input::Hook(tool(hook("PermissionRequest", "s1"), "Bash", "a")), &at(1_000));
        run(&mut e, Input::Hook(tool(hook("PermissionRequest", "s1"), "Write", "b")), &at(1_100));
        run(&mut e, Input::Hook(tool(hook("PostToolUse", "s1"), "Bash", "a")), &at(3_000));
        let waiting = e.snapshot(&at(3_000)).waiting;
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].tool.as_deref(), Some("Write"));
    }

    #[test]
    fn a_permission_notification_duplicates_its_request() {
        let mut e = Engine::new(Config::default());
        prompt(&mut e, "s1", 0);
        run(&mut e, Input::Hook(tool(hook("PermissionRequest", "s1"), "Bash", "t1")), &at(1_000));
        let mut note = hook("Notification", "s1");
        note.notification_type = Some("permission_prompt".into());
        assert_eq!(run(&mut e, Input::Hook(note), &at(1_200)), ["suppressed duplicate"]);
    }

    #[test]
    fn bursts_never_exceed_the_flash_interval() {
        let mut e = Engine::new(Config::default());
        let flashes: usize = (0..30)
            .map(|i| {
                run(&mut e, Input::Hook(hook("Stop", &format!("s{i}"))), &at(i * 10))
                    .iter()
                    .filter(|d| d.starts_with("flash"))
                    .count()
            })
            .sum();
        assert_eq!(flashes, 1);
        // A stronger signal inside the window is held until the window ends.
        assert!(run(&mut e, Input::Hook(tool(hook("PermissionRequest", "s1"), "Bash", "t")), &at(400)).is_empty());
        assert_eq!(e.next_deadline(), Some(1_000));
        assert!(run(&mut e, Input::Tick, &at(999)).is_empty());
        assert_eq!(run(&mut e, Input::Tick, &at(1_000)), ["flash approval"]);
    }

    #[test]
    fn pause_suppresses_until_it_expires() {
        let mut e = Engine::new(Config::default());
        run(&mut e, Input::Control(Control::Pause { for_ms: 60_000 }), &at(0));
        assert_eq!(run(&mut e, Input::Hook(hook("Stop", "s1")), &at(1_000)), ["suppressed paused"]);
        assert_eq!(e.next_deadline(), Some(60_000));
        run(&mut e, Input::Tick, &at(60_000));
        assert_eq!(e.pause_remaining(&at(60_000)), None);
        assert_eq!(run(&mut e, Input::Hook(hook("Stop", "s1")), &at(61_000)), ["flash done"]);
    }

    #[test]
    fn away_signals_become_notifications_and_a_flash_on_return() {
        let config = Config::parse("[push]\nurl = \"https://ntfy.sh/t\"\nafter = \"10m\"\n").unwrap();
        let mut e = Engine::new(config);
        prompt(&mut e, "s1", 0);
        let away = |now_ms, idle_ms| Context { idle_ms, ..at(now_ms) };
        assert_eq!(run(&mut e, Input::Hook(hook("Stop", "s1")), &away(400_000, 360_000)), ["notify done"]);
        let request = tool(hook("PermissionRequest", "s1"), "Bash", "t1");
        assert_eq!(run(&mut e, Input::Hook(request), &away(700_000, 660_000)), ["notify approval", "push approval"]);
        assert_eq!(
            run(&mut e, Input::Tick, &at(710_000)),
            ["flash approval"],
            "the pending approval outranks the finished turn"
        );
    }

    #[test]
    fn quiet_hours_suppress_flashes() {
        let mut e = Engine::new(Config::parse("[quiet_hours]\nstart = \"11:00\"\nend = \"13:00\"\n").unwrap());
        assert_eq!(run(&mut e, Input::Hook(hook("Stop", "s1")), &at(0)), ["suppressed quiet_hours"]);
        assert_eq!(run(&mut e, Input::Hook(hook("Stop", "s1")), &at(2 * 3_600_000)), ["flash done"]);
    }

    #[test]
    fn reminders_repeat_while_claude_stays_blocked() {
        let mut e = Engine::new(Config::parse("[flash]\nremind_after = \"10m\"\n").unwrap());
        prompt(&mut e, "s1", 0);
        run(&mut e, Input::Hook(tool(hook("PermissionRequest", "s1"), "Bash", "t1")), &at(1_000));
        assert_eq!(e.next_deadline(), Some(601_000));
        assert!(run(&mut e, Input::Tick, &at(600_000)).is_empty());
        assert_eq!(run(&mut e, Input::Tick, &at(601_000)), ["flash approval"]);
        assert_eq!(e.next_deadline(), Some(1_201_000));
    }

    #[test]
    fn tests_bypass_pause_but_not_the_flash_interval() {
        let mut e = Engine::new(Config::default());
        run(&mut e, Input::Control(Control::Pause { for_ms: 3_600_000 }), &at(0));
        assert_eq!(run(&mut e, Input::Control(Control::Test(Attention::Question)), &at(1)), ["flash question"]);
        assert!(run(&mut e, Input::Control(Control::Test(Attention::Done)), &at(2)).is_empty());
    }

    #[test]
    fn records_carry_folder_names_but_no_paths_unless_asked() {
        let mut e = Engine::new(Config::default());
        let actions = e.handle(Input::Hook(hook("Stop", "3aee9676-df62-4e22")), &at(0));
        let record =
            actions.iter().find_map(|a| if let Action::Record(r) = a { Some(r.clone()) } else { None }).unwrap();
        assert_eq!(record.project.as_deref(), Some("app"));
        assert_eq!(record.session.as_deref(), Some("3aee9676"));
        assert_eq!(record.path, None);
        assert_eq!(record.ts, "2026-09-10T12:00:00.000Z");
    }

    #[test]
    fn restored_state_applies_after_a_restart() {
        let mut e = Engine::new(Config::default());
        e.restore(false, None, &at(0));
        assert_eq!(run(&mut e, Input::Hook(hook("Stop", "s1")), &at(1)), ["suppressed disabled"]);
        let sessions = BTreeMap::from([
            ("old".to_owned(), BASE_UNIX - INTERACTIVE_TTL_MS - 1),
            ("recent".to_owned(), BASE_UNIX - 1_000),
        ]);
        e.restore_interactive(sessions, BASE_UNIX);
        assert_eq!(e.interactive_sessions().keys().collect::<Vec<_>>(), ["recent"]);
    }

    #[test]
    fn strip_exe_is_char_safe() {
        assert_eq!(strip_exe("WindowsTerminal.EXE"), "WindowsTerminal");
        assert_eq!(strip_exe("Code"), "Code");
        assert_eq!(strip_exe("é.exé"), "é.exé");
    }
}
