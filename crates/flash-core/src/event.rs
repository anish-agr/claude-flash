//! Claude Code hook payloads, and what each one means for the person at the keyboard.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// Something that deserves the user's attention. Declaration order is priority
/// order: when signals coalesce, the highest one wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Attention {
    /// Claude finished responding.
    Done,
    /// The turn ended because of an API error.
    Error,
    /// Claude asked a question and is waiting for the answer.
    Question,
    /// A tool call is waiting on a permission decision.
    Approval,
}

impl Attention {
    pub const ALL: [Attention; 4] = [Attention::Done, Attention::Question, Attention::Approval, Attention::Error];

    pub const fn as_str(self) -> &'static str {
        match self {
            Attention::Done => "done",
            Attention::Error => "error",
            Attention::Question => "question",
            Attention::Approval => "approval",
        }
    }

    /// Accepts canonical names plus the verbs v1 used on its command line.
    pub fn parse(s: &str) -> Option<Attention> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "done" | "finished" | "stop" => Attention::Done,
            "question" | "ask" | "input" => Attention::Question,
            "approval" | "approve" | "perm" | "permission" => Attention::Approval,
            "error" | "failure" | "fail" => Attention::Error,
            _ => return None,
        })
    }

    /// Whether Claude is stuck until the user acts.
    pub const fn blocks_claude(self) -> bool {
        matches!(self, Attention::Question | Attention::Approval)
    }
}

impl fmt::Display for Attention {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Attention {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Attention::parse(&s).ok_or_else(|| {
            serde::de::Error::custom(format!("unknown signal kind {s:?} (done, question, approval, error)"))
        })
    }
}

/// The handful of hook fields Claude Flash uses. Prompts, tool inputs and assistant
/// messages are never read, so they cannot end up in a log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookEvent {
    pub name: String,
    pub session_id: String,
    pub cwd: Option<String>,
    pub permission_mode: Option<String>,
    pub tool_name: Option<String>,
    pub tool_use_id: Option<String>,
    pub notification_type: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventError {
    NotJson(String),
    NotAnObject,
    MissingEventName,
}

impl fmt::Display for EventError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventError::NotJson(e) => write!(f, "hook payload is not valid JSON: {e}"),
            EventError::NotAnObject => f.write_str("hook payload is not a JSON object"),
            EventError::MissingEventName => f.write_str("hook payload has no hook_event_name"),
        }
    }
}

impl std::error::Error for EventError {}

impl HookEvent {
    pub fn from_slice(body: &[u8]) -> Result<Self, EventError> {
        let value: Value = serde_json::from_slice(body).map_err(|e| EventError::NotJson(e.to_string()))?;
        Self::from_value(&value)
    }

    pub fn from_value(value: &Value) -> Result<Self, EventError> {
        let obj = value.as_object().ok_or(EventError::NotAnObject)?;
        let text = |key: &str| obj.get(key).and_then(Value::as_str).map(str::to_owned).filter(|s| !s.is_empty());
        Ok(HookEvent {
            name: text("hook_event_name").ok_or(EventError::MissingEventName)?,
            session_id: text("session_id").unwrap_or_else(|| "unknown".to_owned()),
            cwd: text("cwd"),
            permission_mode: text("permission_mode"),
            tool_name: text("tool_name"),
            tool_use_id: text("tool_use_id"),
            notification_type: text("notification_type"),
            agent_id: text("agent_id"),
        })
    }

    /// First eight characters of the session id: enough to tell sessions apart in a
    /// log, short enough to read.
    pub fn short_session(&self) -> &str {
        short_id(&self.session_id)
    }
}

pub fn short_id(id: &str) -> &str {
    let end = id.char_indices().nth(8).map_or(id.len(), |(i, _)| i);
    &id[..end]
}

/// What a hook event implies, independent of any user preference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Raise a signal. Blocking signals carry the id of the call they wait on, when
    /// there is one, so the wait can be matched to its resolution.
    Signal {
        kind: Attention,
        wait: Option<String>,
    },
    /// The user submitted a prompt: this is an interactive session, and anything it
    /// was waiting on is over.
    Prompted,
    /// Claude moved on. With an id, the wait for that call ends; without one, waits
    /// that had no id end (a notification-derived question, say).
    Progress {
        tool_use_id: Option<String>,
    },
    SessionStarted,
    SessionEnded,
    Ignored,
}

pub fn classify(ev: &HookEvent) -> Effect {
    let id = || ev.tool_use_id.clone();
    match ev.name.as_str() {
        "Stop" => Effect::Signal { kind: Attention::Done, wait: None },
        "StopFailure" => Effect::Signal { kind: Attention::Error, wait: None },
        "PermissionRequest" => Effect::Signal { kind: Attention::Approval, wait: id() },
        "PreToolUse" if ev.tool_name.as_deref() == Some("AskUserQuestion") => {
            Effect::Signal { kind: Attention::Question, wait: id() }
        }
        "PreToolUse" => Effect::Progress { tool_use_id: None },
        "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" => Effect::Progress { tool_use_id: id() },
        "Elicitation" => Effect::Signal { kind: Attention::Question, wait: None },
        "ElicitationResult" => Effect::Progress { tool_use_id: None },
        "Notification" => match ev.notification_type.as_deref() {
            Some("permission_prompt") => Effect::Signal { kind: Attention::Approval, wait: None },
            Some("elicitation_dialog" | "elicitation_url_dialog" | "agent_needs_input") => {
                Effect::Signal { kind: Attention::Question, wait: None }
            }
            _ => Effect::Ignored,
        },
        "UserPromptSubmit" => Effect::Prompted,
        "SessionStart" => Effect::SessionStarted,
        "SessionEnd" => Effect::SessionEnded,
        _ => Effect::Ignored,
    }
}

/// A signal sent through the local API by something other than Claude Code: a build
/// script, a test runner, another agent.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signal {
    pub kind: Attention,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
}

impl Signal {
    pub const MAX_TITLE: usize = 120;
    pub const MAX_BODY: usize = 500;

    /// Trims every text field to a bounded length, so a misbehaving client cannot
    /// push megabytes into a notification.
    pub fn bounded(mut self) -> Self {
        fn clip(s: &mut Option<String>, max: usize) {
            if let Some(v) = s
                && let Some((i, _)) = v.char_indices().nth(max)
            {
                v.truncate(i);
            }
        }
        clip(&mut self.title, Self::MAX_TITLE);
        clip(&mut self.body, Self::MAX_BODY);
        clip(&mut self.source, 40);
        clip(&mut self.project, 80);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(v: Value) -> HookEvent {
        HookEvent::from_value(&v).unwrap()
    }

    #[test]
    fn reads_only_the_fields_it_needs() {
        let e = ev(json!({
            "hook_event_name": "PermissionRequest",
            "session_id": "3aee9676-df62-4e22-9928-fdeee50375dd",
            "cwd": "/Users/a/claude-flash",
            "permission_mode": "default",
            "tool_name": "Bash",
            "tool_input": {"command": "rm -rf build"},
            "tool_use_id": "toolu_01",
            "last_assistant_message": "secret"
        }));
        assert_eq!(e.name, "PermissionRequest");
        assert_eq!(e.tool_name.as_deref(), Some("Bash"));
        assert_eq!(e.short_session(), "3aee9676");
        // No field of HookEvent can hold the tool input or the assistant message.
        assert!(!format!("{e:?}").contains("secret") && !format!("{e:?}").contains("rm -rf"));
    }

    #[test]
    fn tolerates_missing_session_but_not_missing_event_name() {
        assert_eq!(ev(json!({"hook_event_name": "Stop"})).session_id, "unknown");
        assert_eq!(HookEvent::from_value(&json!({"session_id": "x"})), Err(EventError::MissingEventName));
        assert_eq!(HookEvent::from_value(&json!([1, 2])), Err(EventError::NotAnObject));
        assert!(matches!(HookEvent::from_slice(b"{nope"), Err(EventError::NotJson(_))));
    }

    #[test]
    fn classifies_every_signal_source() {
        let c = |v| classify(&ev(v));
        let signal = |kind, wait: Option<&str>| Effect::Signal { kind, wait: wait.map(str::to_owned) };
        assert_eq!(c(json!({"hook_event_name": "Stop", "session_id": "s"})), signal(Attention::Done, None));
        assert_eq!(c(json!({"hook_event_name": "StopFailure", "session_id": "s"})), signal(Attention::Error, None));
        assert_eq!(
            c(json!({"hook_event_name": "PermissionRequest", "session_id": "s", "tool_use_id": "t1"})),
            signal(Attention::Approval, Some("t1"))
        );
        assert_eq!(
            c(
                json!({"hook_event_name": "PreToolUse", "session_id": "s", "tool_name": "AskUserQuestion", "tool_use_id": "t2"})
            ),
            signal(Attention::Question, Some("t2"))
        );
        assert_eq!(c(json!({"hook_event_name": "Elicitation", "session_id": "s"})), signal(Attention::Question, None));
        assert_eq!(
            c(json!({"hook_event_name": "Notification", "session_id": "s", "notification_type": "permission_prompt"})),
            signal(Attention::Approval, None)
        );
    }

    #[test]
    fn classifies_progress_and_lifecycle() {
        let c = |v| classify(&ev(v));
        assert_eq!(
            c(json!({"hook_event_name": "PreToolUse", "session_id": "s", "tool_name": "Bash"})),
            Effect::Progress { tool_use_id: None }
        );
        assert_eq!(
            c(json!({"hook_event_name": "PostToolUse", "session_id": "s", "tool_use_id": "t1"})),
            Effect::Progress { tool_use_id: Some("t1".into()) }
        );
        assert_eq!(c(json!({"hook_event_name": "UserPromptSubmit", "session_id": "s"})), Effect::Prompted);
        assert_eq!(c(json!({"hook_event_name": "SessionEnd", "session_id": "s"})), Effect::SessionEnded);
        assert_eq!(
            c(json!({"hook_event_name": "Notification", "session_id": "s", "notification_type": "idle_prompt"})),
            Effect::Ignored
        );
        assert_eq!(c(json!({"hook_event_name": "PreCompact", "session_id": "s"})), Effect::Ignored);
    }

    #[test]
    fn priority_follows_declaration_order() {
        assert!(Attention::Approval > Attention::Question);
        assert!(Attention::Question > Attention::Error);
        assert!(Attention::Error > Attention::Done);
    }

    #[test]
    fn parses_v1_verbs() {
        assert_eq!(Attention::parse("perm"), Some(Attention::Approval));
        assert_eq!(Attention::parse("ask"), Some(Attention::Question));
        assert_eq!(Attention::parse("DONE"), Some(Attention::Done));
        assert_eq!(Attention::parse("maybe"), None);
    }

    #[test]
    fn signals_are_bounded() {
        let s: Signal = serde_json::from_value(json!({"kind": "done", "title": "x".repeat(10_000)})).unwrap();
        assert_eq!(s.bounded().title.unwrap().chars().count(), Signal::MAX_TITLE);
    }

    #[test]
    fn signals_reject_unknown_fields_and_kinds() {
        assert!(serde_json::from_value::<Signal>(json!({"kind": "done", "colour": "red"})).is_err());
        assert!(serde_json::from_value::<Signal>(json!({"kind": "celebrate"})).is_err());
    }

    #[test]
    fn short_id_is_char_safe() {
        assert_eq!(short_id("ééééééééééé"), "éééééééé");
        assert_eq!(short_id("abc"), "abc");
    }
}
