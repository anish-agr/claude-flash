//! What the CLI prints: the status summary, journal lines and statistics.

use std::thread;
use std::time::Duration;

use flash_core::engine::Waiting;
use flash_core::event::Attention;
use flash_core::journal::{Channel, Record};
use flash_core::{duration, stats, time};

use super::{Env, Outcome, agent_error, hooks, style};
use crate::api::Today;
use crate::client::ClientError;
use crate::store::State;
use crate::{journal, paths, system};

const DAY_MS: u64 = 86_400_000;

/// A dimmed, fixed-width label for the start of a line.
fn label(text: &str) -> String {
    style::dim(&format!("{text:<10}"))
}

pub fn status(env: &Env, json: bool) -> Outcome {
    let status = match env.client().status() {
        Ok(status) => Some(status),
        Err(ClientError::NotRunning) => None,
        Err(e) => return Err(e.to_string()),
    };
    if json {
        let status = status.ok_or_else(super::not_running)?;
        println!("{}", serde_json::to_string_pretty(&status).expect("status serialises"));
        return Ok(());
    }
    match &status {
        Some(s) => {
            let uptime = time::parse_rfc3339(&s.started)
                .map(|t| format!(" · up {}", duration::format(system::unix_ms().saturating_sub(t))))
                .unwrap_or_default();
            println!(
                "{}  {}",
                switch_line(s.enabled, s.paused_for_ms, s.away),
                style::dim(&format!("agent {} · pid {}{uptime}", s.version, s.pid))
            );
            for (i, wait) in s.waiting.iter().enumerate() {
                println!("{}{}", label(if i == 0 { "waiting" } else { "" }), wait_line(wait));
            }
            println!("{}{}", label("today"), today_line(&s.today));
            if s.opted_out > 0 {
                let line = format!("{} hook events ignored from sessions run with CLAUDE_FLASH=off", s.opted_out);
                println!("{}{}", label(""), style::dim(&line));
            }
            if let Some(error) = &s.config_error {
                println!("{}{}", label("config"), style::red(error));
            }
        }
        None => {
            let state = State::load(&env.paths.state_file());
            println!("{}  {}", style::dim("○ agent not running"), style::dim("start it with `flash agent start`"));
            println!("{}{}", label("switch"), if state.enabled { "on" } else { "off" });
        }
    }
    println!("{}{}", label("hooks"), hooks::summary(env));
    println!("{}{}", label("config"), paths::display(&env.paths.config_file()));
    Ok(())
}

pub fn switch_line(enabled: bool, paused_for_ms: Option<u64>, away: bool) -> String {
    let mut line = match (enabled, paused_for_ms) {
        (false, _) => style::red("○ off"),
        (true, Some(ms)) => style::yellow(&format!("◐ paused for {}", duration::format(ms))),
        (true, None) => style::green("● on"),
    };
    if away {
        line.push_str(&style::dim(" · away"));
    }
    line
}

fn wait_line(wait: &Waiting) -> String {
    let what = match &wait.tool {
        Some(tool) if wait.kind == Attention::Approval => format!("{tool} in {}", wait.project),
        _ => wait.project.clone(),
    };
    format!(
        "{} {what}  {}",
        style::kind(wait.kind, &format!("{:<9}", wait.kind.as_str())),
        style::dim(&format!("{} · session {}", duration::format(wait.for_ms), wait.session))
    )
}

fn today_line(today: &Today) -> String {
    let mut parts = vec![
        count(today.done, "done", "done"),
        count(today.question, "question", "questions"),
        count(today.approval, "approval", "approvals"),
        count(today.error, "error", "errors"),
        count(today.flashes, "flash", "flashes"),
    ];
    if today.suppressed > 0 {
        parts.push(count(today.suppressed, "held back", "held back"));
    }
    parts.join(" · ")
}

fn count(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub struct LogFilter {
    pub kind: Option<Attention>,
    pub project: Option<String>,
    pub all: bool,
    pub limit: usize,
}

pub fn log(env: &Env, since: &str, filter: &LogFilter, json: bool) -> Outcome {
    let window = duration::parse(since).map_err(|e| e.to_string())?.ms();
    let records = journal::read(&env.paths.journal_dir(), system::unix_ms().saturating_sub(window));
    let selected: Vec<&Record> = records
        .iter()
        .filter(|r| filter.all || !is_bookkeeping(r))
        .filter(|r| filter.kind.is_none_or(|k| r.kind == Some(k)))
        .filter(|r| {
            filter.project.as_deref().is_none_or(|p| r.project.as_deref().is_some_and(|rp| rp.eq_ignore_ascii_case(p)))
        })
        .collect();
    let shown = &selected[selected.len().saturating_sub(filter.limit)..];
    if json {
        for record in shown {
            println!("{}", serde_json::to_string(record).expect("records serialise"));
        }
        return Ok(());
    }
    if shown.is_empty() {
        println!("{}", nothing_recorded(env, since));
        return Ok(());
    }
    let with_date = window > DAY_MS;
    if selected.len() > shown.len() {
        let hidden = selected.len() - shown.len();
        println!("{}", style::dim(&format!("{hidden} earlier entries not shown; raise -n to see them")));
    }
    for record in shown {
        println!("{}", record_line(record, with_date));
    }
    Ok(())
}

/// Prompts and session boundaries: useful with `--all`, noise otherwise.
fn is_bookkeeping(record: &Record) -> bool {
    record.waited_ms.is_none() && matches!(record.event.as_str(), "UserPromptSubmit" | "SessionStart" | "SessionEnd")
}

fn nothing_recorded(env: &Env, since: &str) -> String {
    if env.config.journal.enabled {
        format!("Nothing recorded in the last {since}.")
    } else {
        "The journal is off. Turn it on with `flash config set journal.enabled true`.".to_owned()
    }
}

pub fn watch(env: &Env, json: bool) -> Outcome {
    let client = env.client();
    let recent = client.events(None, Duration::ZERO).map_err(agent_error)?;
    for record in &recent.records {
        print_record(record, json);
    }
    if !json {
        eprintln!("{}", style::dim("Watching for signals. Press Ctrl+C to stop."));
    }
    let mut since = recent.next;
    let mut lost = false;
    loop {
        match client.events(Some(since), Duration::from_secs(25)) {
            Ok(events) => {
                if lost {
                    eprintln!("{}", style::dim("The agent is back."));
                    lost = false;
                }
                for record in &events.records {
                    print_record(record, json);
                }
                // A restarted agent numbers its records from zero again.
                since = if events.next < since { 0 } else { events.next };
            }
            Err(ClientError::NotRunning) => {
                if !lost {
                    eprintln!("{}", style::dim("The agent stopped; waiting for it to start again."));
                    lost = true;
                }
                since = 0;
                thread::sleep(Duration::from_secs(2));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn print_record(record: &Record, json: bool) {
    if json {
        println!("{}", serde_json::to_string(record).expect("records serialise"));
    } else if !is_bookkeeping(record) {
        println!("{}", record_line(record, false));
    }
}

pub fn record_line(record: &Record, with_date: bool) -> String {
    let when = record.unix_ms().map_or_else(|| record.ts.clone(), |t| clock(t, record.tz, with_date));
    let what = match record.kind {
        Some(kind) => style::kind(kind, &format!("{:<9}", kind.as_str())),
        None => format!("{:<9}", event_label(&record.event)),
    };
    let mut line = format!("{}  {what} {:<30} ", style::dim(&when), outcome(record));
    let context: Vec<&str> = [record.project.as_deref(), record.tool.as_deref(), record.detail.as_deref()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    if !context.is_empty() {
        line.push_str(&style::dim(&context.join("  ")));
    }
    line.trim_end().to_owned()
}

fn event_label(event: &str) -> String {
    match event {
        "pause" | "resume" | "enable" | "disable" => "switch".to_owned(),
        "UserPromptSubmit" => "prompt".to_owned(),
        "SessionStart" | "SessionEnd" => "session".to_owned(),
        other => other.to_ascii_lowercase(),
    }
}

fn outcome(record: &Record) -> String {
    if let Some(ms) = record.waited_ms {
        let verb = match record.event.as_str() {
            "PermissionDenied" => "denied",
            "UserPromptSubmit" | "Stop" | "StopFailure" => "moved on",
            "SessionEnd" => "session ended",
            _ => "answered",
        };
        return format!("{verb} after {}", duration::format(ms));
    }
    if let Some(reason) = record.suppressed {
        return format!("held back: {}", reason.describe());
    }
    if record.kind.is_some() && record.delivered.is_empty() {
        return "covered by the flash before it".to_owned();
    }
    match record.event.as_str() {
        "pause" => "paused".to_owned(),
        "resume" => "resumed".to_owned(),
        "enable" => "turned on".to_owned(),
        "disable" => "turned off".to_owned(),
        "UserPromptSubmit" => "prompt submitted".to_owned(),
        "SessionStart" => "started".to_owned(),
        "SessionEnd" => "ended".to_owned(),
        event if !record.delivered.is_empty() => {
            let channels: Vec<&str> =
                record.delivered.iter().filter(|c| **c != Channel::Digest).map(|c| c.as_str()).collect();
            let away = record.delivered.contains(&Channel::Digest);
            let shown = match (channels.is_empty(), away) {
                (true, _) => "saved for your return".to_owned(),
                (false, true) => format!("{} while away", channels.join(" + ")),
                (false, false) => channels.join(" + "),
            };
            match event {
                "test" => format!("test {shown}"),
                "reminder" => format!("reminder {shown}"),
                "digest" => format!("welcome back {shown}"),
                _ => shown,
            }
        }
        event => event.to_owned(),
    }
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn clock(unix_ms: u64, utc_offset_min: i32, with_date: bool) -> String {
    let seconds = (unix_ms / 1000) as i64 + i64::from(utc_offset_min) * 60;
    let day = seconds.rem_euclid(86_400);
    let hms = format!("{:02}:{:02}:{:02}", day / 3600, day / 60 % 60, day % 60);
    if !with_date {
        return hms;
    }
    let (_, month, date) = time::local_date(unix_ms, utc_offset_min);
    format!("{} {date:>2} {hms}", MONTHS[(month as usize + 11) % 12])
}

pub fn stats(env: &Env, since: &str, json: bool) -> Outcome {
    let window = duration::parse(since).map_err(|e| e.to_string())?.ms();
    let records = journal::read(&env.paths.journal_dir(), system::unix_ms().saturating_sub(window));
    let summary = stats::summarize(&records);
    if json {
        println!("{}", serde_json::to_string_pretty(&summary).expect("summaries serialise"));
        return Ok(());
    }
    if summary.records == 0 {
        println!("{}", nothing_recorded(env, since));
        return Ok(());
    }
    println!("{}", style::bold(&format!("Claude Flash · the last {since}")));
    println!();

    let signals: Vec<String> = super::TOUR
        .iter()
        .map(|kind| {
            let n = summary.signals.get(kind).map_or(0, |c| c.total);
            let name = match (kind, n) {
                (Attention::Done, _) => "done",
                (Attention::Question, 1) => "question",
                (Attention::Question, _) => "questions",
                (Attention::Approval, 1) => "approval",
                (Attention::Approval, _) => "approvals",
                (Attention::Error, 1) => "error",
                (Attention::Error, _) => "errors",
            };
            style::kind(*kind, &format!("{n} {name}"))
        })
        .collect();
    println!("{}{}", label("signals"), signals.join("   "));

    let channel = |c: Channel| summary.channels.get(&c).copied().unwrap_or(0);
    let held: u32 = summary.suppressed.values().sum();
    println!(
        "{}{} · {} · {} · {}",
        label("shown as"),
        count(channel(Channel::Flash), "flash", "flashes"),
        count(channel(Channel::Notify), "notification", "notifications"),
        count(channel(Channel::Push), "push", "pushes"),
        count(held, "held back", "held back"),
    );
    if held > 0 {
        let reasons: Vec<String> =
            summary.suppressed.iter().map(|(reason, n)| format!("{n} {}", reason.describe())).collect();
        println!("{}{}", label(""), style::dim(&reasons.join(" · ")));
    }
    let waits = &summary.waits;
    if waits.count > 0 {
        println!(
            "{}{} · median {} · p90 {} · longest {} · {} in all",
            label("waiting"),
            count(waits.count, "wait", "waits"),
            duration::format(waits.median_ms),
            duration::format(waits.p90_ms),
            duration::format(waits.max_ms),
            duration::format(waits.total_ms),
        );
    }
    println!(
        "{}{} · {}",
        label("sessions"),
        count(summary.sessions, "session", "sessions"),
        count(summary.prompts, "prompt", "prompts")
    );
    println!();
    println!("{}{}", label("by hour"), sparkline(&summary.hours));
    println!("{}{}", label(""), style::dim("0     6     12    18   23"));

    if !summary.projects.is_empty() {
        println!();
        let top = &summary.projects[..summary.projects.len().min(8)];
        let width = top.iter().map(|p| p.name.chars().count()).max().unwrap_or(0).min(28);
        for (i, project) in top.iter().enumerate() {
            let name: String = project.name.chars().take(width).collect();
            let waited = if project.waited_ms > 0 {
                style::dim(&format!("  {} waiting", duration::format(project.waited_ms)))
            } else {
                String::new()
            };
            println!(
                "{}{name:<width$}  {}{waited}",
                label(if i == 0 { "projects" } else { "" }),
                count(project.signals, "signal", "signals")
            );
        }
    }
    Ok(())
}

fn sparkline(values: &[u32]) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max = values.iter().copied().max().unwrap_or(0).max(1);
    values.iter().map(|&v| if v == 0 { '·' } else { BARS[(v as usize * (BARS.len() - 1)) / max as usize] }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flash_core::journal::Reason;

    fn record(event: &str) -> Record {
        Record { ts: time::rfc3339(1_789_041_600_000), event: event.into(), ..Record::default() }
    }

    #[test]
    fn outcomes_read_as_plain_statements() {
        let mut answered = record("PostToolUse");
        answered.kind = Some(Attention::Approval);
        answered.waited_ms = Some(9_000);
        assert_eq!(outcome(&answered), "answered after 9s");

        let mut held = record("Stop");
        held.kind = Some(Attention::Done);
        held.suppressed = Some(Reason::Background);
        assert_eq!(outcome(&held), "held back: background session");

        let mut away = record("PermissionRequest");
        away.delivered = vec![Channel::Digest, Channel::Notify, Channel::Push];
        assert_eq!(outcome(&away), "notify + push while away");
    }

    #[test]
    fn clock_uses_the_recorded_offset() {
        // 12:00 UTC is 05:00 in UTC-7.
        assert_eq!(clock(1_789_041_600_000, -420, false), "05:00:00");
        assert_eq!(clock(1_789_041_600_000, 0, true), "Sep 10 12:00:00");
    }

    #[test]
    fn sparkline_scales_to_the_busiest_hour() {
        assert_eq!(sparkline(&[0, 1, 4, 8]), "·▁▄█");
    }
}
