//! The activity journal: one JSON object per line, one file per local day.
//!
//! Records hold what happened and when, never what was said: event names, signal
//! kinds, project folder names, tool names and durations.

use serde::{Deserialize, Serialize};

use crate::event::Attention;
use crate::time;

/// How a signal reached the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Flash,
    Notify,
    Push,
    /// Held while the user was away, to be summarised on return.
    Digest,
}

/// Why a signal was not shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Disabled,
    Paused,
    Muted,
    SignalOff,
    Background,
    Duplicate,
    QuietHours,
    Focused,
}

impl Channel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Channel::Flash => "flash",
            Channel::Notify => "notify",
            Channel::Push => "push",
            Channel::Digest => "digest",
        }
    }
}

impl Reason {
    /// The name used in journal files.
    pub const fn as_str(self) -> &'static str {
        match self {
            Reason::Disabled => "disabled",
            Reason::Paused => "paused",
            Reason::Muted => "muted",
            Reason::SignalOff => "signal_off",
            Reason::Background => "background",
            Reason::Duplicate => "duplicate",
            Reason::QuietHours => "quiet_hours",
            Reason::Focused => "focused",
        }
    }

    pub const fn describe(self) -> &'static str {
        match self {
            Reason::Disabled => "switched off",
            Reason::Paused => "paused",
            Reason::Muted => "project muted",
            Reason::SignalOff => "signal turned off",
            Reason::Background => "background session",
            Reason::Duplicate => "duplicate of a wait already signalled",
            Reason::QuietHours => "quiet hours",
            Reason::Focused => "focused application",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// RFC 3339 UTC timestamp.
    pub ts: String,
    /// The local UTC offset in minutes when the record was written.
    #[serde(default)]
    pub tz: i32,
    /// A hook event name, or `signal`, `digest`, `reminder`, `pause` and so on.
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Attention>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delivered: Vec<Channel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suppressed: Option<Reason>,
    /// Set on the record that ends a wait: how long Claude was blocked on the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waited_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Record {
    pub fn to_line(&self) -> String {
        let mut line = serde_json::to_string(self).expect("records always serialise");
        line.push('\n');
        line
    }

    /// Parses one line, skipping anything malformed rather than failing the whole
    /// file: a journal truncated by a crash is still worth reading.
    pub fn parse_line(line: &str) -> Option<Record> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        serde_json::from_str(line).ok()
    }

    pub fn unix_ms(&self) -> Option<u64> {
        time::parse_rfc3339(&self.ts)
    }

    /// A raised signal, as opposed to a wait resolving or a lifecycle event.
    pub fn is_signal(&self) -> bool {
        self.kind.is_some() && self.waited_ms.is_none()
    }
}

/// `2026-09-10.jsonl`, named for the local date.
pub fn file_name(unix_ms: u64, utc_offset_min: i32) -> String {
    let (y, m, d) = time::local_date(unix_ms, utc_offset_min);
    format!("{y:04}-{m:02}-{d:02}.jsonl")
}

/// The local date encoded in a journal file name.
pub fn parse_file_name(name: &str) -> Option<(i64, u32, u32)> {
    let stem = name.strip_suffix(".jsonl")?;
    let mut parts = stem.splitn(3, '-');
    let (y, m, d) = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_omits_empty_fields() {
        let r = Record {
            ts: time::rfc3339(1_789_064_521_123),
            tz: -420,
            event: "PermissionRequest".into(),
            kind: Some(Attention::Approval),
            session: Some("3aee9676".into()),
            project: Some("claude-flash".into()),
            tool: Some("Bash".into()),
            delivered: vec![Channel::Flash],
            ..Record::default()
        };
        let line = r.to_line();
        assert_eq!(
            line,
            "{\"ts\":\"2026-09-10T18:22:01.123Z\",\"tz\":-420,\"event\":\"PermissionRequest\",\"kind\":\"approval\",\"session\":\"3aee9676\",\"project\":\"claude-flash\",\"tool\":\"Bash\",\"delivered\":[\"flash\"]}\n"
        );
        assert_eq!(Record::parse_line(&line), Some(r));
    }

    #[test]
    fn skips_blank_and_broken_lines() {
        assert_eq!(Record::parse_line("   "), None);
        assert_eq!(Record::parse_line("{\"ts\":\"2026-09"), None);
    }

    #[test]
    fn distinguishes_signals_from_resolutions() {
        let signal = Record { kind: Some(Attention::Question), ..Record::default() };
        let resolution = Record { kind: Some(Attention::Question), waited_ms: Some(5), ..Record::default() };
        assert!(signal.is_signal() && !resolution.is_signal());
    }

    #[test]
    fn file_names_follow_the_local_date() {
        let t = time::parse_rfc3339("2026-09-10T02:30:00Z").unwrap();
        assert_eq!(file_name(t, -420), "2026-09-09.jsonl");
        assert_eq!(file_name(t, 0), "2026-09-10.jsonl");
        assert_eq!(parse_file_name("2026-09-09.jsonl"), Some((2026, 9, 9)));
        assert_eq!(parse_file_name("notes.txt"), None);
        assert_eq!(parse_file_name("2026-13-01.jsonl"), None);
    }
}
