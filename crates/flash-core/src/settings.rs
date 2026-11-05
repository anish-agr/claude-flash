//! Hook registration in Claude Code's `settings.json`.
//!
//! That file belongs to the user and to every other tool they run, so each edit here
//! is surgical: only entries Claude Flash owns are removed or added, everything else
//! keeps its content and its position, and a file that does not parse is refused
//! rather than overwritten.

use std::fmt;

use serde_json::{Map, Value, json};

/// Header carried by every HTTP hook Claude Flash installs. Claude Code sends it to
/// the agent, and in `settings.json` it marks the entry as ours.
pub const MARKER_HEADER: &str = "X-Claude-Flash";
pub const HOOK_VERSION: &str = "2";
/// Carries the session's `CLAUDE_FLASH` environment variable to the agent, so a
/// script can start Claude Code with `CLAUDE_FLASH=off` and keep its sessions out.
pub const SESSION_HEADER: &str = "X-Claude-Flash-Session";
pub const SESSION_ENV_VAR: &str = "CLAUDE_FLASH";
pub const INGRESS_PATH: &str = "/v1/hooks/claude-code";

/// Every event Claude Flash listens to, with the matcher each one needs.
pub const SUBSCRIPTIONS: &[(&str, Option<&str>)] = &[
    ("SessionStart", None),
    ("UserPromptSubmit", None),
    ("Stop", None),
    ("StopFailure", None),
    ("PermissionRequest", Some("*")),
    ("PreToolUse", Some("AskUserQuestion")),
    ("PostToolUse", Some("*")),
    ("PostToolUseFailure", Some("*")),
    ("PermissionDenied", Some("*")),
    ("Elicitation", Some("*")),
    ("ElicitationResult", Some("*")),
    ("Notification", Some("*")),
    ("SessionEnd", None),
];

/// Every Gemini CLI event Claude Flash listens to, under Gemini's own names. Gemini
/// publishes the mapping itself in `gemini hooks migrate`: `BeforeAgent` is Claude
/// Code's `UserPromptSubmit`, `AfterAgent` its `Stop`, and so on. Only the tool
/// events take a matcher, as Gemini's own examples show.
pub const GEMINI_SUBSCRIPTIONS: &[(&str, Option<&str>)] = &[
    ("SessionStart", None),
    ("BeforeAgent", None),
    ("BeforeTool", Some("*")),
    ("AfterTool", Some("*")),
    ("Notification", None),
    ("AfterAgent", None),
    ("SessionEnd", None),
];

/// Which agent's settings file is being written. They share the shape of the `hooks`
/// object, so the merging is the same, but they name events differently and only
/// Claude Code accepts an HTTP hook.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Host {
    #[default]
    ClaudeCode,
    Gemini,
}

impl Host {
    fn subscriptions(self) -> &'static [(&'static str, Option<&'static str>)] {
        match self {
            Host::ClaudeCode => SUBSCRIPTIONS,
            Host::Gemini => GEMINI_SUBSCRIPTIONS,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Install {
    pub port: u16,
    /// Absolute path of the `flash` executable, which every session runs once at
    /// start as `flash agent ensure`.
    pub program: String,
    /// Set when the hooks point at an agent on another machine, which has nothing
    /// local to start and wants its token on every event.
    pub remote: Option<RemoteAgent>,
    /// Whose settings file this is. Claude Code by default.
    pub host: Host,
}

/// An agent reachable over the network, and the token that admits a request to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteAgent {
    /// Base URL, such as `http://192.168.1.5:47823`.
    pub url: String,
    pub token: String,
}

impl Install {
    /// Where hook events are sent.
    fn url(&self) -> String {
        match &self.remote {
            Some(remote) => format!("{}{INGRESS_PATH}", remote.url.trim_end_matches('/')),
            None => ingress_url(self.port),
        }
    }
}

pub fn ingress_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}{INGRESS_PATH}")
}

/// Whether a session's `CLAUDE_FLASH` value keeps it out: `off`, `0`, `false` or `no`.
pub fn session_opted_out(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "off" | "0" | "false" | "no")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsError {
    Json(String),
    NotAnObject,
    Malformed(String),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::Json(e) => write!(f, "settings.json is not valid JSON ({e}); left unchanged"),
            SettingsError::NotAnObject => f.write_str("settings.json is not a JSON object; left unchanged"),
            SettingsError::Malformed(what) => write!(f, "settings.json: {what}; left unchanged"),
        }
    }
}

impl std::error::Error for SettingsError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub text: String,
    pub changed: bool,
    pub removed: usize,
    pub added: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inspection {
    /// Installed hooks that match what `install` would write.
    pub current: usize,
    /// Hooks of ours that are not current: v1 command hooks, an old port.
    pub outdated: usize,
    /// Subscribed events with no current hook.
    pub missing: Vec<&'static str>,
}

impl Inspection {
    pub fn up_to_date(&self) -> bool {
        self.missing.is_empty() && self.outdated == 0
    }
}

pub fn install(original: Option<&str>, spec: &Install) -> Result<Change, SettingsError> {
    let (mut root, before) = load(original)?;
    let hooks = hooks_mut(&mut root)?;
    let removed = strip(hooks)?;
    let mut added = 0;
    for (event, matcher) in spec.host.subscriptions() {
        let group = desired_group(event, *matcher, spec);
        added += group["hooks"].as_array().map_or(0, Vec::len);
        let slot = hooks.entry((*event).to_owned()).or_insert_with(|| Value::Array(Vec::new()));
        slot.as_array_mut()
            .ok_or_else(|| SettingsError::Malformed(format!("hooks.{event} is not an array")))?
            .push(group);
    }
    Ok(finish(original, before, root, removed, added))
}

pub fn uninstall(original: &str) -> Result<Change, SettingsError> {
    let (mut root, before) = load(Some(original))?;
    let removed = match root.get_mut("hooks") {
        None => 0,
        Some(hooks) => {
            strip(hooks.as_object_mut().ok_or_else(|| SettingsError::Malformed("hooks is not an object".into()))?)?
        }
    };
    Ok(finish(Some(original), before, root, removed, 0))
}

pub fn inspect(original: &str, spec: &Install) -> Result<Inspection, SettingsError> {
    let (root, _) = load(Some(original))?;
    let empty = Map::new();
    let hooks = match root.get("hooks") {
        None => &empty,
        Some(v) => v.as_object().ok_or_else(|| SettingsError::Malformed("hooks is not an object".into()))?,
    };
    let ours_total: usize = hooks
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|g| g.get("hooks").and_then(Value::as_array))
        .flatten()
        .filter(|h| is_flash_hook(h))
        .count();
    let mut report = Inspection::default();
    for (event, matcher) in spec.host.subscriptions() {
        let want = desired_group(event, *matcher, spec);
        let want_hooks = want["hooks"].as_array().cloned().unwrap_or_default();
        let found = hooks.get(*event).and_then(Value::as_array).is_some_and(|groups| {
            groups.iter().any(|g| {
                g.get("matcher").and_then(Value::as_str) == *matcher
                    && g.get("hooks")
                        .and_then(Value::as_array)
                        .is_some_and(|hs| want_hooks.iter().all(|w| hs.contains(w)))
            })
        });
        if found {
            report.current += want_hooks.len();
        } else {
            report.missing.push(event);
        }
    }
    report.outdated = ours_total.saturating_sub(report.current);
    Ok(report)
}

/// Whether `settings.json` switches every hook off.
pub fn hooks_disabled(original: &str) -> bool {
    load(Some(original)).is_ok_and(|(root, _)| root.get("disableAllHooks").and_then(Value::as_bool) == Some(true))
}

/// Whether a hook entry was written by Claude Flash, in this version or v1.
pub fn is_flash_hook(hook: &Value) -> bool {
    if hook
        .get("headers")
        .and_then(Value::as_object)
        .is_some_and(|h| h.keys().any(|k| k.eq_ignore_ascii_case(MARKER_HEADER)))
    {
        return true;
    }
    let Some(command) = hook.get("command").and_then(Value::as_str) else { return false };
    let lower = command.to_ascii_lowercase();
    // v1's diagnostic logger wrote here through cmd.exe.
    if lower.contains(r"claudeflash\hook.log") || lower.contains("claudeflash/hook.log") {
        return true;
    }
    // In exec form `command` is the executable itself, spaces and all.
    let stem = if hook.get("args").is_some() { Some(file_stem(command)) } else { program_stem(command) };
    matches!(stem.as_deref(), Some("flash" | "flash-agent"))
}

/// The file name, without extension, of the program a shell-form hook command runs.
/// Handles quoting and PowerShell's `&` call operator.
pub fn program_stem(command: &str) -> Option<String> {
    let s = command.trim();
    let s = s.strip_prefix('&').map_or(s, str::trim_start);
    let first = s.chars().next()?;
    let token = if first == '"' || first == '\'' {
        let rest = &s[1..];
        &rest[..rest.find(first)?]
    } else {
        s.split_whitespace().next()?
    };
    Some(file_stem(token))
}

fn file_stem(path: &str) -> String {
    let name = path.trim().rsplit(['/', '\\']).next().unwrap_or_default().to_ascii_lowercase();
    name.strip_suffix(".exe").map(str::to_owned).unwrap_or(name)
}

fn desired_group(event: &str, matcher: Option<&str>, spec: &Install) -> Value {
    // SessionStart runs a command rather than an HTTP hook. The agent may not be up
    // when a session starts, and the hooks for one event run in parallel, so an HTTP
    // hook here would race the command that starts it. The command forwards the
    // event itself once the agent is listening.
    // A remote agent has nothing here to start, so it takes SessionStart over HTTP
    // like every other event.
    // Gemini runs a command per event and drops anything else, so every one of its
    // hooks runs `flash hook`, which starts the agent if it is down and hands the
    // event over. Its timeout is in milliseconds, unlike Claude Code's seconds.
    let hook = if spec.host == Host::Gemini {
        json!({"type": "command", "command": spec.program, "args": ["hook"], "name": "claude-flash", "timeout": 10_000})
    } else if event == "SessionStart" && spec.remote.is_none() {
        json!({"type": "command", "command": spec.program, "args": ["agent", "ensure"], "timeout": 30})
    } else {
        let mut headers = Map::new();
        headers.insert(MARKER_HEADER.into(), HOOK_VERSION.into());
        headers.insert(SESSION_HEADER.into(), format!("${{{SESSION_ENV_VAR}}}").into());
        if let Some(remote) = &spec.remote {
            headers.insert("Authorization".into(), format!("Bearer {}", remote.token).into());
        }
        json!({
            "type": "http",
            "url": spec.url(),
            "timeout": 5,
            "headers": headers,
            "allowedEnvVars": [SESSION_ENV_VAR],
        })
    };
    let mut group = Map::new();
    if let Some(m) = matcher {
        group.insert("matcher".into(), m.into());
    }
    group.insert("hooks".into(), json!([hook]));
    Value::Object(group)
}

fn load(original: Option<&str>) -> Result<(Map<String, Value>, Value), SettingsError> {
    let text = original.unwrap_or_default().trim_start_matches('\u{feff}');
    if text.trim().is_empty() {
        return Ok((Map::new(), Value::Null));
    }
    match serde_json::from_str::<Value>(text).map_err(|e| SettingsError::Json(e.to_string()))? {
        Value::Object(map) => {
            let before = Value::Object(map.clone());
            Ok((map, before))
        }
        _ => Err(SettingsError::NotAnObject),
    }
}

fn hooks_mut(root: &mut Map<String, Value>) -> Result<&mut Map<String, Value>, SettingsError> {
    root.entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| SettingsError::Malformed("hooks is not an object".into()))
}

/// Removes every Claude Flash hook, dropping groups and events that only existed to
/// hold them. Returns how many hooks were removed.
fn strip(hooks: &mut Map<String, Value>) -> Result<usize, SettingsError> {
    let mut removed = 0;
    let mut emptied = Vec::new();
    for (event, groups) in hooks.iter_mut() {
        let groups =
            groups.as_array_mut().ok_or_else(|| SettingsError::Malformed(format!("hooks.{event} is not an array")))?;
        let mut touched = false;
        groups.retain_mut(|group| {
            let Some(list) = group.get_mut("hooks").and_then(Value::as_array_mut) else { return true };
            let before = list.len();
            list.retain(|h| !is_flash_hook(h));
            let gone = before - list.len();
            removed += gone;
            touched |= gone > 0;
            !(gone > 0 && list.is_empty())
        });
        if touched && groups.is_empty() {
            emptied.push(event.clone());
        }
    }
    for event in emptied {
        hooks.shift_remove(&event);
    }
    Ok(removed)
}

fn finish(original: Option<&str>, before: Value, root: Map<String, Value>, removed: usize, added: usize) -> Change {
    let after = Value::Object(root);
    // Semantically unchanged means untouched: never reformat a file for nothing.
    if let Some(text) = original
        && after == before
    {
        return Change { text: text.to_owned(), changed: false, removed, added };
    }
    let mut text = serde_json::to_string_pretty(&after).expect("a JSON value always serialises");
    if original.is_none_or(|o| o.is_empty() || o.ends_with('\n')) {
        text.push('\n');
    }
    Change { text, changed: true, removed, added }
}

/// Every Codex event Claude Flash listens to. Codex names them as Claude Code does,
/// which is how its own bundled plugins write them, even though it spells the same
/// events in kebab case inside a payload. It has no equivalent of Claude Code's
/// `AskUserQuestion` tool or of `StopFailure`, so Codex drives the done and approval
/// signals and the waits, but not question or error.
pub const CODEX_EVENTS: &[&str] =
    &["SessionStart", "UserPromptSubmit", "PreToolUse", "PermissionRequest", "PostToolUse", "Stop", "SessionEnd"];

/// A `hooks.json` for Codex, which runs a command per event rather than posting to
/// an address: every event runs `flash hook`, which starts the agent if it is not up
/// and hands the event over on standard input.
///
/// Codex's parser is strict about the shape, and it rejects the file outright over
/// one bad entry: `command` is a string, not a list of arguments, and a byte order
/// mark at the start counts as a parse error, so this is written as plain UTF-8.
pub fn codex_hooks(program: &str) -> Value {
    let mut events = Map::new();
    for event in CODEX_EVENTS {
        let hook = json!({"type": "command", "command": program, "args": ["hook"], "timeout": 30});
        events.insert((*event).to_owned(), json!([{ "hooks": [hook] }]));
    }
    json!({ "hooks": Value::Object(events) })
}

/// Whether a `hooks.json` holds nothing but Claude Flash's own hooks, so replacing it
/// outright loses nobody else's work.
pub fn codex_hooks_are_only_ours(original: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(original.trim_start_matches('\u{feff}')) else {
        return original.trim().is_empty();
    };
    let Some(events) = value.get("hooks").and_then(Value::as_object) else {
        return value.as_object().is_some_and(Map::is_empty);
    };
    events.values().all(|groups| {
        groups.as_array().is_some_and(|groups| {
            groups.iter().all(|group| {
                group.get("hooks").and_then(Value::as_array).is_some_and(|hooks| hooks.iter().all(is_flash_hook))
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> Install {
        Install {
            port: 47_823,
            program: r"C:\Program Files\Claude Flash\flash.exe".into(),
            remote: None,
            host: Host::default(),
        }
    }

    /// Shaped like a real file: another tool's hooks around a v1 Claude Flash hook.
    const EXISTING: &str = r#"{
  "permissions": {
    "allow": [
      "Bash(git push:*)"
    ]
  },
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "*",
        "hooks": [
          {
            "type": "command",
            "command": "C:\\tools\\sentinel-hook.exe pre"
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "\"C:\\Users\\a\\AppData\\Local\\Microsoft\\WindowsApps\\flash.exe\" done --bg --require_session"
          }
        ]
      },
      {
        "matcher": "*",
        "hooks": [
          {
            "type": "command",
            "command": "C:\\tools\\sentinel-hook.exe stop"
          }
        ]
      }
    ]
  },
  "theme": "dark"
}
"#;

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn hooks_for_another_machine_carry_the_token_and_start_nothing_locally() {
        let remote = RemoteAgent { url: "http://192.168.1.5:47823/".into(), token: "9f3a".into() };
        let spec = Install { remote: Some(remote), ..spec() };
        let v = parse(&install(None, &spec).unwrap().text);
        let session_start = &v["hooks"]["SessionStart"][0]["hooks"][0];
        assert_eq!(session_start["type"], "http", "there is no agent on that machine to start");
        for hook in [session_start, &v["hooks"]["Stop"][0]["hooks"][0]] {
            assert_eq!(hook["url"], "http://192.168.1.5:47823/v1/hooks/claude-code", "one slash, not two");
            assert_eq!(hook["headers"]["Authorization"], "Bearer 9f3a");
            assert_eq!(hook["headers"][MARKER_HEADER], HOOK_VERSION);
        }
    }

    #[test]
    fn gemini_hooks_use_its_own_event_names_and_keep_other_tools() {
        // Shaped like a real Gemini settings.json: another tool's guard on BeforeTool.
        let existing = r#"{
  "hooks": {
    "BeforeTool": [
      {
        "hooks": [{ "command": "C:\\tools\\other-guard.exe", "name": "other-guard", "timeout": 10000, "type": "command" }],
        "matcher": "*"
      }
    ]
  }
}"#;
        let spec = Install { host: Host::Gemini, ..spec() };
        let change = install(Some(existing), &spec).unwrap();
        let v = parse(&change.text);
        let events: Vec<&String> = v["hooks"].as_object().unwrap().keys().collect();
        for want in ["SessionStart", "BeforeAgent", "BeforeTool", "AfterTool", "AfterAgent", "SessionEnd"] {
            assert!(events.iter().any(|e| *e == want), "{want} missing from {events:?}");
        }
        // Claude Code's names must not leak into a Gemini file.
        assert!(!events.iter().any(|e| e.as_str() == "Stop" || e.as_str() == "UserPromptSubmit"));

        // The other tool's guard is still there, alongside ours.
        let before_tool = v["hooks"]["BeforeTool"].as_array().unwrap();
        let names: Vec<&str> = before_tool
            .iter()
            .flat_map(|g| g["hooks"].as_array().unwrap())
            .map(|h| h["name"].as_str().unwrap_or("?"))
            .collect();
        assert!(names.contains(&"other-guard"), "another tool's hook was lost: {names:?}");
        assert!(names.contains(&"claude-flash"));

        // Every hook of ours runs `flash hook`, and Gemini counts its timeout in ms.
        let ours = &v["hooks"]["AfterAgent"][0]["hooks"][0];
        assert_eq!(ours["type"], "command");
        assert_eq!(ours["args"], json!(["hook"]));
        assert_eq!(ours["timeout"], 10_000);
        assert!(ours["url"].is_null(), "Gemini drops HTTP hooks, so it must not get one");

        // And taking them out again leaves the other tool untouched.
        let removed = uninstall(&change.text).unwrap();
        let after = parse(&removed.text);
        assert_eq!(removed.removed, GEMINI_SUBSCRIPTIONS.len());
        assert_eq!(after["hooks"]["BeforeTool"][0]["hooks"][0]["name"], "other-guard");
    }

    #[test]
    fn codex_hooks_match_the_shape_codex_accepts() {
        let v = codex_hooks(r"C:\bin\flash.exe");
        let events = v["hooks"].as_object().expect("an object of events");
        assert_eq!(events.len(), CODEX_EVENTS.len());
        // Codex names events as Claude Code does, not in the kebab case it uses in a
        // payload, and its own bundled plugins group hooks the same way.
        // Stop is the finished-turn signal, and PermissionRequest the approval.
        assert!(events.contains_key("Stop") && events.contains_key("PermissionRequest"));
        let hook = &events["SessionStart"][0]["hooks"][0];
        assert_eq!(hook["type"], "command");
        // A list here is rejected outright: "invalid type: sequence, expected a string".
        assert!(hook["command"].is_string());
        assert_eq!(hook["args"], json!(["hook"]));

        // Ours alone can be replaced; anything else is the user's to merge.
        let ours = serde_json::to_string(&v).unwrap();
        assert!(codex_hooks_are_only_ours(&ours));
        assert!(codex_hooks_are_only_ours(""));
        assert!(!codex_hooks_are_only_ours(
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"notify-send done"}]}]}}"#
        ));
    }

    #[test]
    fn install_replaces_v1_and_leaves_other_tools_in_place() {
        let change = install(Some(EXISTING), &spec()).unwrap();
        assert!(change.changed);
        assert_eq!(change.removed, 1);
        assert_eq!(change.added, SUBSCRIPTIONS.len());
        let v = parse(&change.text);
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["permissions", "hooks", "theme"], "top-level order is preserved");
        // The foreign PreToolUse group stays first; ours is appended after it.
        assert_eq!(v["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "C:\\tools\\sentinel-hook.exe pre");
        assert_eq!(v["hooks"]["PreToolUse"][1]["matcher"], "AskUserQuestion");
        // The v1 group is gone entirely; the foreign Stop group now leads.
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "C:\\tools\\sentinel-hook.exe stop");
        assert_eq!(v["hooks"]["Stop"][1]["hooks"][0]["url"], "http://127.0.0.1:47823/v1/hooks/claude-code");
        assert!(change.text.ends_with("}\n"));
    }

    #[test]
    fn session_start_runs_the_agent_ensure_command_in_exec_form() {
        let v = parse(&install(None, &spec()).unwrap().text);
        assert_eq!(
            v["hooks"]["SessionStart"][0]["hooks"],
            json!([{"type": "command", "command": r"C:\Program Files\Claude Flash\flash.exe", "args": ["agent", "ensure"], "timeout": 30}])
        );
    }

    #[test]
    fn http_hooks_carry_the_marker_and_the_session_opt_out() {
        let v = parse(&install(None, &spec()).unwrap().text);
        let hook = &v["hooks"]["PermissionRequest"][0]["hooks"][0];
        assert_eq!(hook["type"], "http");
        assert_eq!(hook["headers"][MARKER_HEADER], HOOK_VERSION);
        assert_eq!(hook["headers"][SESSION_HEADER], "${CLAUDE_FLASH}");
        assert_eq!(hook["allowedEnvVars"], json!(["CLAUDE_FLASH"]));
    }

    #[test]
    fn install_is_idempotent_and_never_reformats_needlessly() {
        let once = install(Some(EXISTING), &spec()).unwrap();
        let twice = install(Some(&once.text), &spec()).unwrap();
        assert!(!twice.changed);
        assert_eq!(twice.text, once.text);
        let compact = serde_json::to_string(&parse(&once.text)).unwrap();
        let again = install(Some(&compact), &spec()).unwrap();
        assert!(!again.changed, "semantically identical input is returned byte for byte");
        assert_eq!(again.text, compact);
    }

    #[test]
    fn uninstall_removes_only_ours() {
        let installed = install(Some(EXISTING), &spec()).unwrap();
        let removed = uninstall(&installed.text).unwrap();
        assert_eq!(removed.removed, SUBSCRIPTIONS.len());
        let v = parse(&removed.text);
        assert_eq!(v["hooks"].as_object().unwrap().keys().collect::<Vec<_>>(), ["PreToolUse", "Stop"]);
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert_eq!(v["theme"], "dark");
    }

    #[test]
    fn mixed_groups_lose_only_our_entry() {
        let text = json!({"hooks": {"Stop": [{"hooks": [
            {"type": "command", "command": "/usr/local/bin/flash done"},
            {"type": "command", "command": "notify-send done"}
        ]}]}})
        .to_string();
        let v = parse(&uninstall(&text).unwrap().text);
        assert_eq!(v["hooks"]["Stop"][0]["hooks"], json!([{"type": "command", "command": "notify-send done"}]));
    }

    #[test]
    fn inspection_reports_legacy_stale_and_current() {
        let before = inspect(EXISTING, &spec()).unwrap();
        assert_eq!(before.outdated, 1);
        assert_eq!(before.missing.len(), SUBSCRIPTIONS.len());
        let installed = install(Some(EXISTING), &spec()).unwrap().text;
        assert!(inspect(&installed, &spec()).unwrap().up_to_date());
        // Only the HTTP hooks name the port; the SessionStart command is still current.
        let other_port = Install { port: 50_000, ..spec() };
        let stale = inspect(&installed, &other_port).unwrap();
        assert!(!stale.up_to_date());
        assert_eq!(stale.outdated, SUBSCRIPTIONS.len() - 1);
        assert_eq!(stale.missing.len(), SUBSCRIPTIONS.len() - 1);
    }

    #[test]
    fn refuses_files_it_cannot_safely_edit() {
        assert!(matches!(install(Some("{ not json"), &spec()), Err(SettingsError::Json(_))));
        assert_eq!(install(Some("[1, 2]"), &spec()), Err(SettingsError::NotAnObject));
        assert!(matches!(install(Some(r#"{"hooks": []}"#), &spec()), Err(SettingsError::Malformed(_))));
        assert!(matches!(install(Some(r#"{"hooks": {"Stop": {}}}"#), &spec()), Err(SettingsError::Malformed(_))));
    }

    #[test]
    fn creates_a_file_from_nothing_and_tolerates_a_bom() {
        let fresh = install(None, &spec()).unwrap();
        assert!(fresh.changed && fresh.text.starts_with("{\n  \"hooks\": {"));
        assert!(install(Some("\u{feff}{}"), &spec()).is_ok());
    }

    #[test]
    fn recognises_session_opt_out_values() {
        for value in ["off", "OFF", " 0 ", "false", "no"] {
            assert!(session_opted_out(value), "{value:?}");
        }
        // An unset variable interpolates to nothing, or stays literal in older clients.
        for value in ["", "on", "1", "${CLAUDE_FLASH}", "quiet"] {
            assert!(!session_opted_out(value), "{value:?}");
        }
    }

    #[test]
    fn notices_hooks_switched_off_globally() {
        assert!(hooks_disabled(r#"{"disableAllHooks": true}"#));
        assert!(!hooks_disabled(r#"{"disableAllHooks": false}"#));
        assert!(!hooks_disabled("{}"));
    }

    #[test]
    fn recognises_program_names_across_quoting_styles() {
        let stem = |c| program_stem(c);
        assert_eq!(stem(r#""C:\x\flash.exe" done --bg"#).as_deref(), Some("flash"));
        assert_eq!(stem(r"& 'C:\x\FLASH.EXE' agent ensure").as_deref(), Some("flash"));
        assert_eq!(stem("/Users/a/.local/bin/flash agent ensure").as_deref(), Some("flash"));
        assert_eq!(stem(r"C:\tools\sentinel-hook.exe pre").as_deref(), Some("sentinel-hook"));
        assert_eq!(stem("\"unterminated").as_deref(), None);
        assert!(!is_flash_hook(&json!({"type": "command", "command": "flashy --go"})));
        assert!(is_flash_hook(
            &json!({"type": "command", "command": r"C:\Program Files\x\flash.exe", "args": ["agent", "ensure"]})
        ));
        assert!(is_flash_hook(
            &json!({"type": "command", "command": r#"cmd /c echo x >> "%LOCALAPPDATA%\ClaudeFlash\hook.log""#})
        ));
    }
}
