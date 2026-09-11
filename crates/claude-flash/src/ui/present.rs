//! What the tray icon and the menu bar item say about the agent, shared by both
//! platforms.

use flash_core::color::Rgb;
use flash_core::config;
use flash_core::duration;
use flash_core::engine::Waiting;
use flash_core::event::Attention;

use crate::api::Status;

/// The signals, in the order menus list them.
pub const KINDS: [Attention; 4] = [Attention::Done, Attention::Question, Attention::Approval, Attention::Error];

/// Pause lengths offered in the menu: the duration, and its label.
pub const PAUSES: [(&str, &str); 3] = [("15m", "For 15 minutes"), ("1h", "For 1 hour"), ("3h", "For 3 hours")];

pub const QUIT_DETAIL: &str = "Until Claude Flash runs again, Claude Code sessions that are already open will \
    report hook errors. It starts again when you log in, or when a new Claude Code session starts.\n\n\
    To stop flashes but keep the agent running, uncheck Flashes on or use Pause.";

const OFF: Rgb = Rgb::new(0x8A, 0x8F, 0x98);
const PAUSED: Rgb = Rgb::new(0xFF, 0xAA, 0x00);

pub fn title(kind: Attention) -> &'static str {
    match kind {
        Attention::Done => "Done",
        Attention::Question => "Question",
        Attention::Approval => "Approval",
        Attention::Error => "Error",
    }
}

/// The icon colour for a status, with a tooltip to match: grey when off, amber when
/// paused, the colour of the most urgent wait, or green.
pub fn indicator(status: &Status) -> (Rgb, String) {
    if !status.enabled {
        return (OFF, "Claude Flash is off".to_owned());
    }
    if let Some(ms) = status.paused_for_ms {
        return (PAUSED, format!("Claude Flash is paused, {} left", duration::format(ms)));
    }
    match status.waiting.first() {
        Some(wait) => {
            let tip = format!("Claude Flash: {} waiting for you", status.waiting.len());
            (config::builtin(wait.kind).color, tip)
        }
        None => (config::builtin(Attention::Done).color, "Claude Flash is on".to_owned()),
    }
}

/// The menu's first line. `elapsed_ms` is how long ago the status arrived.
pub fn headline(status: Option<&Status>, elapsed_ms: u64) -> String {
    match status {
        None => "Claude Flash".to_owned(),
        Some(status) if !status.enabled => "Claude Flash is off".to_owned(),
        Some(status) => match status.paused_for_ms {
            Some(ms) => format!("Claude Flash is paused, {} left", duration::format(ms.saturating_sub(elapsed_ms))),
            None if status.away => "Claude Flash is on, and you are away".to_owned(),
            None => "Claude Flash is on".to_owned(),
        },
    }
}

/// One waiting item for the menu, with its age brought up to date.
pub fn waiting(wait: &Waiting, elapsed_ms: u64) -> String {
    let what = match (&wait.tool, wait.kind) {
        (Some(tool), Attention::Approval) => format!("{tool} in {}", wait.project),
        _ => wait.project.clone(),
    };
    format!("{} · {what} · {}", title(wait.kind), duration::format(wait.for_ms + elapsed_ms))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Today;

    fn status() -> Status {
        Status {
            version: flash_core::VERSION.to_owned(),
            pid: 1,
            port: 47_823,
            started: String::new(),
            display: "test".to_owned(),
            enabled: true,
            paused_for_ms: None,
            away: false,
            waiting: Vec::new(),
            today: Today::default(),
            config_path: String::new(),
            config_error: None,
            journal_path: None,
            last_event: None,
            opted_out: 0,
        }
    }

    fn approval() -> Waiting {
        Waiting {
            kind: Attention::Approval,
            project: "app".into(),
            session: "3aee9676".into(),
            tool: Some("Bash".into()),
            for_ms: 65_000,
        }
    }

    #[test]
    fn the_indicator_follows_the_most_urgent_state() {
        assert_eq!(indicator(&status()).0, config::builtin(Attention::Done).color);
        let waiting = Status { waiting: vec![approval()], ..status() };
        assert_eq!(
            indicator(&waiting),
            (config::builtin(Attention::Approval).color, "Claude Flash: 1 waiting for you".into())
        );
        let paused = Status { paused_for_ms: Some(60_000), ..waiting.clone() };
        assert_eq!(indicator(&paused).0, PAUSED);
        assert_eq!(indicator(&Status { enabled: false, ..paused }).0, OFF);
    }

    #[test]
    fn menu_lines_age_with_the_status() {
        let paused = Status { paused_for_ms: Some(10 * 60_000), ..status() };
        assert_eq!(
            headline(Some(&paused), 60_000),
            format!("Claude Flash is paused, {} left", duration::format(9 * 60_000))
        );
        assert_eq!(waiting(&approval(), 5_000), format!("Approval · Bash in app · {}", duration::format(70_000)));
    }
}
