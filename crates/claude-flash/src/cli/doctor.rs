//! `flash doctor`: checks each part of the setup and says how to fix what is wrong.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use flash_core::config::Config;
use flash_core::settings::{self, Install};
use flash_core::{duration, time};

use super::{Env, Outcome, hooks, style};
use crate::client::ClientError;
use crate::{autostart, journal, paths, system};

enum Level {
    Pass,
    Warn,
    Fail,
}

#[derive(Default)]
struct Report {
    warnings: usize,
    failures: usize,
}

impl Report {
    fn line(&mut self, level: Level, what: &str, detail: impl AsRef<str>) {
        let mark = match level {
            Level::Pass => style::green("✓"),
            Level::Warn => {
                self.warnings += 1;
                style::yellow("!")
            }
            Level::Fail => {
                self.failures += 1;
                style::red("✗")
            }
        };
        println!("{mark} {what:<10}{}", detail.as_ref());
    }
}

pub fn run(env: &Env) -> Outcome {
    let mut report = Report::default();
    println!("{}", style::bold(&format!("Claude Flash {}", flash_core::VERSION)));

    let client = env.client();
    let health = client.health();
    match &health {
        Ok(h) if h.version != flash_core::VERSION => report.line(
            Level::Warn,
            "agent",
            format!("version {} is running, but this is {}; run `flash agent restart`", h.version, flash_core::VERSION),
        ),
        Ok(h) if env.config.agent.remote => report.line(
            Level::Pass,
            "agent",
            format!("running · pid {} · port {} on every interface · token required", h.pid, env.port()),
        ),
        Ok(h) => report.line(Level::Pass, "agent", format!("running · pid {} · 127.0.0.1:{}", h.pid, env.port())),
        Err(ClientError::NotRunning) => {
            report.line(Level::Fail, "agent", "not running; start it with `flash agent start`")
        }
        Err(e) => report.line(Level::Fail, "agent", format!("{e}; choose a free port with agent.port")),
    }
    let status = match (&health, client.status()) {
        (Ok(_), Ok(status)) => Some(status),
        (Ok(_), Err(ClientError::Api { status: 401, .. })) => {
            report.line(Level::Fail, "token", "the agent does not accept this token; run `flash agent restart`");
            None
        }
        (Ok(_), Err(e)) => {
            report.line(Level::Fail, "token", e.to_string());
            None
        }
        _ => None,
    };

    let config_path = env.paths.config_file();
    match fs::read_to_string(&config_path) {
        Ok(text) => match Config::parse(&text) {
            Ok(_) => report.line(Level::Pass, "settings", paths::display(&config_path)),
            Err(e) => report.line(Level::Fail, "settings", e.to_string()),
        },
        Err(_) => report.line(Level::Pass, "settings", "defaults; no config.toml yet"),
    }

    check_hooks(env, &mut report);

    match autostart::registered() {
        Some(program) if program.is_file() => report.line(Level::Pass, "at login", paths::display(&program)),
        Some(program) => report.line(
            Level::Fail,
            "at login",
            format!("starts {}, which does not exist; run `flash install`", program.display()),
        ),
        None => report.line(Level::Warn, "at login", "the agent does not start at login; run `flash install`"),
    }

    if let Some(status) = &status {
        match &status.last_event {
            Some(event) => {
                let ago = time::parse_rfc3339(&event.ts)
                    .map(|t| duration::format(system::unix_ms().saturating_sub(t)))
                    .unwrap_or_default();
                report.line(Level::Pass, "activity", format!("last hook event {} {ago} ago", event.event));
            }
            None => report.line(
                Level::Warn,
                "activity",
                "no hook events since the agent started; start a new Claude Code session to check",
            ),
        }
        if let Some(error) = &status.config_error {
            report.line(Level::Fail, "settings", format!("the agent could not apply them: {error}"));
        }
    }

    if env.config.journal.enabled {
        let dir = env.paths.journal_dir();
        let detail = format!(
            "{} · {} · kept {} days",
            paths::display(&dir),
            size(journal::size(&dir)),
            env.config.journal.retain_days
        );
        report.line(Level::Pass, "journal", detail);
    } else {
        report.line(Level::Pass, "journal", "off");
    }

    if env.config.push.enabled() {
        let curl = if cfg!(windows) { "curl.exe" } else { "curl" };
        let found = Command::new(curl).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status();
        match found {
            Ok(status) if status.success() => report.line(Level::Pass, "push", format!("sent with {curl}")),
            _ => report.line(Level::Fail, "push", format!("push is configured, but {curl} could not be run")),
        }
    }

    if env.paths.data_dir.join("config.ini").is_file() {
        report.line(Level::Warn, "upgrade", "config.ini from version 1 has not been converted; run `flash install`");
    }

    println!();
    match (report.failures, report.warnings) {
        (0, 0) => {
            println!("{}", style::green("Everything checks out."));
            Ok(())
        }
        (0, warnings) => {
            println!("{}", style::yellow(&format!("{warnings} warning{}.", if warnings == 1 { "" } else { "s" })));
            Ok(())
        }
        (failures, _) => Err(format!("{failures} problem{} found", if failures == 1 { "" } else { "s" })),
    }
}

fn check_hooks(env: &Env, report: &mut Report) {
    let path = paths::claude_settings();
    let Ok(text) = fs::read_to_string(&path) else {
        report.line(Level::Fail, "hooks", format!("{} not found; run `flash hooks install`", paths::display(&path)));
        return;
    };
    let program = hooks::installed_program(&text);
    let spec = Install { port: env.port(), program: program.clone().unwrap_or_default(), remote: None };
    match settings::inspect(&text, &spec) {
        Err(e) => report.line(Level::Fail, "hooks", e.to_string()),
        Ok(_) if settings::hooks_disabled(&text) => {
            report.line(Level::Fail, "hooks", "disableAllHooks is set in settings.json, so no hooks run")
        }
        Ok(found) if found.current == 0 && found.outdated == 0 => {
            report.line(Level::Fail, "hooks", "not installed; run `flash hooks install`")
        }
        Ok(found) if found.up_to_date() => match program {
            Some(program) if !Path::new(&program).is_file() => report.line(
                Level::Fail,
                "hooks",
                format!("sessions start {program}, which does not exist; run `flash hooks install`"),
            ),
            _ => report.line(
                Level::Pass,
                "hooks",
                format!("{} events · {}", settings::SUBSCRIPTIONS.len(), paths::display(&path)),
            ),
        },
        Ok(found) => report.line(
            Level::Warn,
            "hooks",
            format!(
                "{} current, {} outdated, {} missing; run `flash hooks install`",
                found.current,
                found.outdated,
                found.missing.len()
            ),
        ),
    }
}

fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / 1_048_576.0),
        b if b >= 1 << 10 => format!("{:.0} KB", b as f64 / 1024.0),
        b => format!("{b} bytes"),
    }
}
