//! Bodies of the agent's local HTTP API, shared by the agent and the CLI.
//!
//! | Method | Path                    | Token | Purpose                                  |
//! |--------|-------------------------|-------|------------------------------------------|
//! | GET    | `/v1/health`            | no    | Is an agent listening, and which version |
//! | POST   | `/v1/hooks/claude-code` | no    | Claude Code hook events                  |
//! | GET    | `/v1/status`            | yes   | Switches, waits and today's counts       |
//! | POST   | `/v1/control`           | yes   | On, off, pause, test, reload, quit       |
//! | POST   | `/v1/signal`            | yes   | Raise a signal from another tool         |
//! | GET    | `/v1/events`            | yes   | Follow the journal as it is written      |

use flash_core::engine::Waiting;
use flash_core::event::Attention;
use flash_core::journal::Record;
use serde::{Deserialize, Serialize};

pub const NAME: &str = "claude-flash";
pub const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    pub name: String,
    pub version: String,
    pub api: u32,
    pub pid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub version: String,
    pub pid: u32,
    pub port: u16,
    /// When the agent started, RFC 3339.
    pub started: String,
    /// `windows`, `macos` or `headless`.
    pub display: String,
    pub enabled: bool,
    pub paused_for_ms: Option<u64>,
    pub away: bool,
    /// Everything Claude is blocked on, most urgent first.
    pub waiting: Vec<Waiting>,
    pub today: Today,
    pub config_path: String,
    /// Why the configuration file was not applied, if it was not.
    pub config_error: Option<String>,
    /// Where the journal is written, when it is enabled.
    pub journal_path: Option<String>,
    pub last_event: Option<LastEvent>,
    /// Hook events ignored because their session ran with `CLAUDE_FLASH=off`.
    pub opted_out: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Today {
    pub done: u32,
    pub question: u32,
    pub approval: u32,
    pub error: u32,
    pub flashes: u32,
    pub suppressed: u32,
}

impl Today {
    pub fn count(&mut self, kind: Attention) {
        match kind {
            Attention::Done => self.done += 1,
            Attention::Question => self.question += 1,
            Attention::Approval => self.approval += 1,
            Attention::Error => self.error += 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastEvent {
    pub event: String,
    pub project: Option<String>,
    pub ts: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Control {
    On,
    Off,
    /// With `confirm`, the change is acknowledged on screen, for a switch pressed
    /// without a terminal to print to.
    Toggle {
        #[serde(default)]
        confirm: bool,
    },
    Pause {
        #[serde(rename = "for")]
        duration: String,
    },
    Resume,
    Test {
        kind: Attention,
    },
    Reload,
    Quit,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Events {
    /// Pass back as `since` to continue.
    pub next: u64,
    pub records: Vec<Record>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_use_a_readable_wire_format() {
        let pause: Control = serde_json::from_str(r#"{"action": "pause", "for": "15m"}"#).unwrap();
        assert_eq!(pause, Control::Pause { duration: "15m".into() });
        let test: Control = serde_json::from_str(r#"{"action": "test", "kind": "perm"}"#).unwrap();
        assert_eq!(test, Control::Test { kind: Attention::Approval });
        assert_eq!(
            serde_json::to_string(&Control::Toggle { confirm: false }).unwrap(),
            r#"{"action":"toggle","confirm":false}"#
        );
        let bare: Control = serde_json::from_str(r#"{"action": "toggle"}"#).unwrap();
        assert_eq!(bare, Control::Toggle { confirm: false });
    }
}
