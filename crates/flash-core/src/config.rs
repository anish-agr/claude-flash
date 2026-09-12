//! User preferences, stored as `config.toml`.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::color::Rgb;
use crate::duration::Span;
use crate::event::Attention;
use crate::glob;
use crate::render::{Style, Timing};
use crate::time::Clock;

/// Flashes never start closer together than this. Three flashes in any one second
/// is the WCAG 2.3.1 general flash threshold; a burst of hook events, however large,
/// must not be able to exceed it.
pub const MIN_FLASH_INTERVAL_MS: u32 = 334;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub flash: Flash,
    pub signals: Signals,
    pub sessions: Sessions,
    pub presence: Presence,
    pub push: Push,
    pub quiet_hours: QuietHours,
    #[serde(rename = "project", skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<ProjectRule>,
    pub agent: Agent,
    pub journal: Journal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Flash {
    pub style: Style,
    pub vignette: f64,
    pub fade_in_ms: u32,
    pub hold_ms: u32,
    pub fade_out_ms: u32,
    pub dismiss_fade_ms: u32,
    pub min_visible_ms: u32,
    pub min_interval_ms: u32,
    pub respect_reduce_motion: bool,
    pub skip_when_focused: Vec<String>,
    pub remind_after: Span,
}

impl Default for Flash {
    fn default() -> Self {
        Flash {
            style: Style::Wash,
            vignette: 0.32,
            fade_in_ms: 70,
            hold_ms: 420,
            fade_out_ms: 560,
            dismiss_fade_ms: 110,
            min_visible_ms: 120,
            min_interval_ms: 1_000,
            respect_reduce_motion: true,
            skip_when_focused: Vec::new(),
            remind_after: Span::ZERO,
        }
    }
}

impl Flash {
    pub fn timing(&self) -> Timing {
        Timing {
            fade_in_ms: self.fade_in_ms,
            hold_ms: self.hold_ms,
            fade_out_ms: self.fade_out_ms,
            dismiss_fade_ms: self.dismiss_fade_ms,
            min_visible_ms: self.min_visible_ms,
        }
    }
}

/// A signal's appearance with every default applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SignalStyle {
    pub enabled: bool,
    pub color: Rgb,
    pub opacity: f64,
}

pub const fn builtin(kind: Attention) -> SignalStyle {
    let (color, opacity) = match kind {
        Attention::Done => (Rgb::new(0x00, 0xFF, 0x5A), 0.28),
        Attention::Question => (Rgb::new(0x08, 0xA9, 0xFF), 0.20),
        Attention::Approval => (Rgb::new(0x8B, 0x2F, 0xCE), 0.20),
        Attention::Error => (Rgb::new(0xFF, 0x3B, 0x30), 0.24),
    };
    SignalStyle { enabled: true, color, opacity }
}

/// One signal's settings as written. Anything left out falls back to [`builtin`],
/// so `[signals.done] color = "#FFD400"` changes only the colour.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SignalOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Rgb>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Signals {
    pub done: SignalOverride,
    pub question: SignalOverride,
    pub approval: SignalOverride,
    pub error: SignalOverride,
}

impl Default for Signals {
    fn default() -> Self {
        let full = |kind| {
            let s = builtin(kind);
            SignalOverride { enabled: Some(s.enabled), color: Some(s.color), opacity: Some(s.opacity) }
        };
        Signals {
            done: full(Attention::Done),
            question: full(Attention::Question),
            approval: full(Attention::Approval),
            error: full(Attention::Error),
        }
    }
}

impl Signals {
    pub fn get(&self, kind: Attention) -> &SignalOverride {
        match kind {
            Attention::Done => &self.done,
            Attention::Question => &self.question,
            Attention::Approval => &self.approval,
            Attention::Error => &self.error,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Sessions {
    pub ignore_background: bool,
}

impl Default for Sessions {
    fn default() -> Self {
        Sessions { ignore_background: true }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Presence {
    pub away_after: Span,
    pub notify_when_away: bool,
    pub digest_on_return: bool,
}

impl Default for Presence {
    fn default() -> Self {
        Presence { away_after: Span(5 * 60_000), notify_when_away: true, digest_on_return: true }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PushFormat {
    #[default]
    Ntfy,
    Json,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Push {
    pub url: String,
    pub format: PushFormat,
    pub token: String,
    pub after: Span,
    pub kinds: Vec<Attention>,
}

impl Default for Push {
    fn default() -> Self {
        Push {
            url: String::new(),
            format: PushFormat::Ntfy,
            token: String::new(),
            after: Span(10 * 60_000),
            kinds: vec![Attention::Question, Attention::Approval, Attention::Error],
        }
    }
}

impl Push {
    pub fn enabled(&self) -> bool {
        !self.url.trim().is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QuietHours {
    pub start: String,
    pub end: String,
    pub notify: bool,
}

impl QuietHours {
    pub fn window(&self) -> Option<(Clock, Clock)> {
        Some((Clock::parse(&self.start)?, Clock::parse(&self.end)?))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Colors {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<Rgb>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<Rgb>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval: Option<Rgb>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Rgb>,
}

impl Colors {
    pub fn get(&self, kind: Attention) -> Option<Rgb> {
        match kind {
            Attention::Done => self.done,
            Attention::Question => self.question,
            Attention::Approval => self.approval,
            Attention::Error => self.error,
        }
    }

    pub fn is_empty(&self) -> bool {
        Attention::ALL.iter().all(|k| self.get(*k).is_none())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRule {
    #[serde(rename = "match")]
    pub pattern: String,
    #[serde(default)]
    pub mute: bool,
    #[serde(default, skip_serializing_if = "Colors::is_empty")]
    pub colors: Colors,
}

impl ProjectRule {
    /// A pattern containing a path separator is matched against the working
    /// directory; otherwise against the project's folder name.
    pub fn matches(&self, cwd: Option<&str>, project: &str) -> bool {
        let pattern = self.pattern.trim();
        if pattern.contains(['/', '\\']) {
            cwd.is_some_and(|c| glob::matches(&glob::normalize_path(pattern), &glob::normalize_path(c)))
        } else {
            glob::matches(pattern, project)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Agent {
    pub port: u16,
    /// Listen on every interface, and require the token on every request, so that
    /// Claude Code in WSL or on another machine can reach this agent.
    pub remote: bool,
}

pub const DEFAULT_PORT: u16 = 47_823;

impl Default for Agent {
    fn default() -> Self {
        Agent { port: DEFAULT_PORT, remote: false }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Journal {
    pub enabled: bool,
    pub retain_days: u32,
    pub store_paths: bool,
}

impl Default for Journal {
    fn default() -> Self {
        Journal { enabled: true, retain_days: 30, store_paths: false }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub key: String,
    pub problem: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    Syntax(String),
    Invalid(Vec<Issue>),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Syntax(msg) => write!(f, "{}", msg.trim_end()),
            ConfigError::Invalid(issues) => {
                for (i, issue) in issues.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{}: {}", issue.key, issue.problem)?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    pub fn parse(text: &str) -> Result<Config, ConfigError> {
        let config: Config = toml::from_str(text).map_err(|e| ConfigError::Syntax(e.to_string()))?;
        config.check()?;
        Ok(config)
    }

    /// Range and consistency checks that types alone cannot express.
    pub fn check(&self) -> Result<(), ConfigError> {
        let mut issues = Vec::new();
        let mut bad = |key: &str, problem: String| issues.push(Issue { key: key.to_owned(), problem });
        let f = &self.flash;
        if !(0.0..=0.9).contains(&f.vignette) {
            bad("flash.vignette", format!("must be between 0 and 0.9, not {}", f.vignette));
        }
        for (key, value, max) in [
            ("flash.fade_in_ms", f.fade_in_ms, 5_000),
            ("flash.hold_ms", f.hold_ms, 10_000),
            ("flash.fade_out_ms", f.fade_out_ms, 10_000),
            ("flash.dismiss_fade_ms", f.dismiss_fade_ms, 2_000),
            ("flash.min_visible_ms", f.min_visible_ms, 2_000),
            ("flash.min_interval_ms", f.min_interval_ms, 60_000),
        ] {
            if value > max {
                bad(key, format!("must be at most {max}, not {value}"));
            }
        }
        if f.min_interval_ms < MIN_FLASH_INTERVAL_MS {
            bad(
                "flash.min_interval_ms",
                format!("must be at least {MIN_FLASH_INTERVAL_MS}, which keeps bursts under three flashes per second"),
            );
        }
        for kind in Attention::ALL {
            if let Some(o) = self.signals.get(kind).opacity
                && !(0.02..=1.0).contains(&o)
            {
                bad(&format!("signals.{kind}.opacity"), format!("must be between 0.02 and 1, not {o}"));
            }
        }
        let q = &self.quiet_hours;
        match (q.start.trim().is_empty(), q.end.trim().is_empty()) {
            (true, true) => {}
            (false, false) => {
                for (key, value) in [("quiet_hours.start", &q.start), ("quiet_hours.end", &q.end)] {
                    if Clock::parse(value).is_none() {
                        bad(key, format!("must be a 24-hour time like \"22:00\", not {value:?}"));
                    }
                }
            }
            _ => bad("quiet_hours", "set both start and end, or neither".to_owned()),
        }
        let url = self.push.url.trim();
        if !url.is_empty() && !(url.starts_with("https://") || url.starts_with("http://")) {
            bad("push.url", format!("must start with https:// or http://, not {url:?}"));
        }
        if self.agent.port < 1_024 {
            bad("agent.port", format!("must be 1024 or higher, not {}", self.agent.port));
        }
        if !(1..=3_650).contains(&self.journal.retain_days) {
            bad("journal.retain_days", format!("must be between 1 and 3650, not {}", self.journal.retain_days));
        }
        for (i, rule) in self.projects.iter().enumerate() {
            if rule.pattern.trim().is_empty() {
                bad(&format!("project[{i}].match"), "must not be empty".to_owned());
            }
        }
        if issues.is_empty() { Ok(()) } else { Err(ConfigError::Invalid(issues)) }
    }

    pub fn signal(&self, kind: Attention) -> SignalStyle {
        let base = builtin(kind);
        let o = self.signals.get(kind);
        SignalStyle {
            enabled: o.enabled.unwrap_or(base.enabled),
            color: o.color.unwrap_or(base.color),
            opacity: o.opacity.unwrap_or(base.opacity),
        }
    }

    /// The signal's style after any matching project rule has recoloured it.
    pub fn signal_for(&self, kind: Attention, rule: Option<&ProjectRule>) -> SignalStyle {
        let mut style = self.signal(kind);
        if let Some(color) = rule.and_then(|r| r.colors.get(kind)) {
            style.color = color;
        }
        style
    }

    pub fn rule_for(&self, cwd: Option<&str>, project: &str) -> Option<&ProjectRule> {
        self.projects.iter().find(|r| r.matches(cwd, project))
    }

    /// Reads a single setting by dotted key, formatted as it would be written.
    pub fn get(&self, key: &str) -> Option<String> {
        let root = toml::Value::try_from(self).ok()?;
        let mut node = &root;
        for part in key.split('.') {
            node = node.get(part)?;
        }
        Some(match node {
            toml::Value::String(s) => s.clone(),
            toml::Value::Table(_) => return None,
            other => other.to_string(),
        })
    }
}

/// The file written on first run. It documents every setting and must parse to
/// exactly [`Config::default`].
pub const DEFAULT_TOML: &str = r##"# Claude Flash configuration.
#
# Changes apply to the next signal; nothing needs restarting. Check this file with
# `flash config check`, or change one value with `flash config set KEY VALUE`.

[flash]
# "wash" tints the whole screen; "edge" glows around its border.
style = "wash"
# 0 is a flat tint. Higher keeps the centre of the screen clearer than the edges.
vignette = 0.32
# Animation, in milliseconds.
fade_in_ms = 70
hold_ms = 420
fade_out_ms = 560
# Fade-out used when a key press or click dismisses the flash early.
dismiss_fade_ms = 110
# Input sooner than this after a flash starts does not dismiss it.
min_visible_ms = 120
# Flashes never start closer together than this. The floor is 334, which keeps any
# burst of events within three flashes per second (WCAG 2.3.1).
min_interval_ms = 1000
# Use short cross-fades when the operating system asks for reduced motion.
respect_reduce_motion = true
# Skip flashing while one of these applications is focused, e.g. ["WindowsTerminal"].
skip_when_focused = []
# Flash again while Claude is still blocked on you after this long. "0" is off.
remind_after = "0"

# Each signal can be recoloured, softened with opacity (0.02 to 1), or turned off.
# To soften a flash, lower its opacity: lightening the colour turns it white instead.
[signals.done]
enabled = true
color = "#00FF5A"
opacity = 0.28

[signals.question]
enabled = true
color = "#08A9FF"
opacity = 0.2

[signals.approval]
enabled = true
color = "#8B2FCE"
opacity = 0.2

[signals.error]
enabled = true
color = "#FF3B30"
opacity = 0.24

[sessions]
# Ignore sessions you never typed a prompt into, such as background agents.
ignore_background = true

[presence]
# With no keyboard or mouse input for this long, you count as away. "0" disables.
away_after = "5m"
# While away, show a desktop notification instead of flashing.
notify_when_away = true
# On your return, flash once for the most important thing that happened.
digest_on_return = true

[push]
# Send signals to a phone while you are away. An empty url disables this.
# Accepts an ntfy topic URL (https://ntfy.sh/your-topic) or any JSON webhook.
url = ""
format = "ntfy"
# Optional bearer token sent with each push.
token = ""
# Push only after being away at least this long.
after = "10m"
kinds = ["question", "approval", "error"]

[quiet_hours]
# Suppress signals between these local times, e.g. start = "22:00", end = "08:00".
start = ""
end = ""
# Still show desktop notifications during quiet hours.
notify = false

# Project rules. A pattern with a slash matches the working directory; one without
# matches the project folder name. The first rule that matches applies.
#
# [[project]]
# match = "*/scratch/*"
# mute = true
#
# [[project]]
# match = "pricetime"
# colors = { done = "#FFD400" }

[agent]
# Loopback port for the local agent. Run `flash hooks install` after changing it.
port = 47823
# Take events from other machines, such as Claude Code in WSL or over SSH. The agent
# then listens on every interface and the token is required on every request, hook
# events included. `flash hooks remote` prints what to run on the other machine.
remote = false

[journal]
enabled = true
retain_days = 30
# Record full working-directory paths instead of only project folder names.
store_paths = false
"##;

/// Sets one value in a config file's text, keeping every comment and all other
/// formatting. The result is parsed and validated before it is returned, so a bad
/// value never reaches disk.
pub fn set_value(text: &str, key: &str, value: &str) -> Result<String, ConfigError> {
    let key = key.trim();
    let invalid = |problem: String| ConfigError::Invalid(vec![Issue { key: key.to_owned(), problem }]);
    if key.split('.').next() == Some("project") {
        return Err(invalid("project rules are edited directly in config.toml".to_owned()));
    }
    let (table, leaf) = key.rsplit_once('.').ok_or_else(|| invalid("unknown setting".to_owned()))?;
    let literal = literal_for(key, value).map_err(invalid)?;
    let assignment = format!("{leaf} = {literal}");

    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let mut current: Option<String> = None;
    let mut header_at = None;
    let mut replaced = false;
    for (i, line) in lines.iter_mut().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("[[") {
            current = None;
            continue;
        }
        if let Some(name) = table_header(trimmed) {
            if name == table {
                header_at = Some(i);
            }
            current = Some(name.to_owned());
            continue;
        }
        if current.as_deref() == Some(table) && assigns(trimmed, leaf) {
            line.clone_from(&assignment);
            replaced = true;
            break;
        }
    }
    if !replaced {
        match header_at {
            Some(i) => lines.insert(i + 1, assignment),
            None => {
                if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                    lines.push(String::new());
                }
                lines.push(format!("[{table}]"));
                lines.push(assignment);
            }
        }
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') || text.is_empty() {
        out.push('\n');
    }
    Config::parse(&out)?;
    Ok(out)
}

fn table_header(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('[')?;
    let end = rest.find(']')?;
    Some(rest[..end].trim())
}

fn assigns(line: &str, leaf: &str) -> bool {
    line.strip_prefix(leaf).is_some_and(|rest| rest.trim_start().starts_with('='))
}

/// Renders a user-supplied value as the TOML literal the setting expects, using the
/// type of the setting's default to decide between string, number and array.
fn literal_for(key: &str, value: &str) -> Result<String, String> {
    let defaults = toml::Value::try_from(Config::default()).map_err(|e| e.to_string())?;
    let mut node = &defaults;
    for part in key.split('.') {
        node = node.get(part).ok_or_else(|| "unknown setting".to_owned())?;
    }
    let v = value.trim();
    Ok(match node {
        toml::Value::String(_) => quote(v),
        toml::Value::Boolean(_) => match v.to_ascii_lowercase().as_str() {
            "true" | "on" | "yes" => "true".to_owned(),
            "false" | "off" | "no" => "false".to_owned(),
            _ => return Err(format!("expected true or false, not {v:?}")),
        },
        toml::Value::Integer(_) => {
            v.parse::<i64>().map_err(|_| format!("expected a whole number, not {v:?}"))?;
            v.to_owned()
        }
        toml::Value::Float(_) => {
            let n: f64 = v.parse().map_err(|_| format!("expected a number, not {v:?}"))?;
            if v.contains(['.', 'e', 'E']) { v.to_owned() } else { format!("{n:.1}") }
        }
        toml::Value::Array(_) if v.starts_with('[') => v.to_owned(),
        toml::Value::Array(_) => {
            let items: Vec<String> = v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(quote).collect();
            format!("[{}]", items.join(", "))
        }
        _ => return Err("this setting is a table; set one of its keys instead".to_owned()),
    })
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The outcome of converting a v1 `config.ini`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Migration {
    pub text: String,
    pub notes: Vec<String>,
}

/// Converts the v1 `config.ini` into `config.toml` text, keeping the user's colours,
/// opacities and timings.
pub fn migrate_v1(ini: &str) -> Migration {
    let mut text = DEFAULT_TOML.to_owned();
    let mut notes = Vec::new();
    for raw in ini.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        let target = match k {
            "alpha" => Some(("signals.done.opacity", v.to_owned())),
            "alpha_ask" => Some(("signals.question.opacity", v.to_owned())),
            "alpha_perm" => Some(("signals.approval.opacity", v.to_owned())),
            "color_done" => Some(("signals.done.color", v.to_owned())),
            "color_ask" => Some(("signals.question.color", v.to_owned())),
            "color_perm" => Some(("signals.approval.color", v.to_owned())),
            "fade_in_ms" | "hold_ms" | "fade_out_ms" | "dismiss_fade_ms" | "min_visible_ms" | "vignette" => {
                Some((leak_flash_key(k), v.to_owned()))
            }
            "perm_flash" => Some(("signals.approval.enabled", v.to_owned())),
            "only_your_sessions" => Some(("sessions.ignore_background", v.to_owned())),
            "skip_if_focused" => Some(("flash.skip_when_focused", v.to_owned())),
            "perm_modes" | "prompt_wait_ms" => {
                notes.push(format!(
                    "{k} is no longer needed: approvals now come from Claude Code's PermissionRequest event, which is exact"
                ));
                None
            }
            _ => {
                notes.push(format!("ignored unknown v1 setting {k}"));
                None
            }
        };
        let Some((key, value)) = target else { continue };
        let value = if key.ends_with(".color") {
            match value.parse::<Rgb>() {
                Ok(c) => c.to_hex(),
                Err(e) => {
                    notes.push(format!("kept the default for {key}: {e}"));
                    continue;
                }
            }
        } else {
            value
        };
        match set_value(&text, key, &value) {
            Ok(updated) => text = updated,
            Err(e) => notes.push(format!("kept the default for {key}: {e}")),
        }
    }
    Migration { text, notes }
}

fn leak_flash_key(k: &str) -> &'static str {
    match k {
        "fade_in_ms" => "flash.fade_in_ms",
        "hold_ms" => "flash.hold_ms",
        "fade_out_ms" => "flash.fade_out_ms",
        "dismiss_fade_ms" => "flash.dismiss_fade_ms",
        "min_visible_ms" => "flash.min_visible_ms",
        _ => "flash.vignette",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_parses_to_defaults() {
        assert_eq!(Config::parse(DEFAULT_TOML).unwrap(), Config::default());
    }

    #[test]
    fn empty_file_means_defaults() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn partial_signal_tables_keep_builtin_values() {
        let c = Config::parse("[signals.done]\ncolor = \"#FFD400\"\n").unwrap();
        let done = c.signal(Attention::Done);
        assert_eq!(done.color, Rgb::new(0xFF, 0xD4, 0x00));
        assert!((done.opacity - 0.28).abs() < 1e-9);
        assert!(done.enabled);
    }

    #[test]
    fn unknown_keys_are_errors() {
        let err = Config::parse("[flash]\nopactiy = 0.3\n").unwrap_err();
        assert!(matches!(err, ConfigError::Syntax(ref m) if m.contains("opactiy")), "{err}");
    }

    #[test]
    fn range_checks_name_the_offending_key() {
        let err = Config::parse("[flash]\nmin_interval_ms = 100\nvignette = 2.0\n[signals.done]\nopacity = 1.5\n")
            .unwrap_err();
        let ConfigError::Invalid(issues) = err else { panic!("expected validation issues") };
        let keys: Vec<&str> = issues.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["flash.vignette", "flash.min_interval_ms", "signals.done.opacity"]);
    }

    #[test]
    fn quiet_hours_need_both_ends() {
        assert!(Config::parse("[quiet_hours]\nstart = \"22:00\"\n").is_err());
        assert!(Config::parse("[quiet_hours]\nstart = \"22:00\"\nend = \"25:00\"\n").is_err());
        let c = Config::parse("[quiet_hours]\nstart = \"22:00\"\nend = \"08:00\"\n").unwrap();
        assert_eq!(c.quiet_hours.window(), Some((Clock(1_320), Clock(480))));
    }

    #[test]
    fn project_rules_match_paths_or_names() {
        let c = Config::parse(
            "[[project]]\nmatch = \"*/scratch/*\"\nmute = true\n\n[[project]]\nmatch = \"price*\"\ncolors = { done = \"#FFD400\" }\n",
        )
        .unwrap();
        assert!(c.rule_for(Some(r"C:\Users\a\scratch\run-1"), "run-1").unwrap().mute);
        let rule = c.rule_for(Some("/Users/a/pricetime"), "pricetime").unwrap();
        assert_eq!(c.signal_for(Attention::Done, Some(rule)).color, Rgb::new(0xFF, 0xD4, 0x00));
        assert_eq!(c.signal_for(Attention::Error, Some(rule)).color, builtin(Attention::Error).color);
        assert!(c.rule_for(Some("/Users/a/other"), "other").is_none());
    }

    #[test]
    fn set_value_edits_in_place_and_keeps_comments() {
        let out = set_value(DEFAULT_TOML, "signals.approval.color", "#C13FFF").unwrap();
        assert!(out.contains("color = \"#C13FFF\""));
        assert!(out.contains("# Each signal can be recoloured"));
        assert_eq!(out.lines().count(), DEFAULT_TOML.lines().count());
        assert_eq!(Config::parse(&out).unwrap().signal(Attention::Approval).color, Rgb::new(0xC1, 0x3F, 0xFF));
    }

    #[test]
    fn set_value_infers_types_from_the_schema() {
        let c = |key, value| Config::parse(&set_value(DEFAULT_TOML, key, value).unwrap()).unwrap();
        assert_eq!(c("agent.port", "50000").agent.port, 50_000);
        assert_eq!(c("presence.away_after", "0").presence.away_after, Span::ZERO);
        assert!((c("signals.done.opacity", "1").signal(Attention::Done).opacity - 1.0).abs() < 1e-9);
        assert!(!c("sessions.ignore_background", "off").sessions.ignore_background);
        assert_eq!(
            c("flash.skip_when_focused", "WindowsTerminal, Code").flash.skip_when_focused,
            ["WindowsTerminal", "Code"]
        );
        assert_eq!(c("push.kinds", "approval").push.kinds, [Attention::Approval]);
    }

    #[test]
    fn set_value_rejects_bad_values_and_unknown_keys() {
        assert!(set_value(DEFAULT_TOML, "signals.done.color", "notacolour").is_err());
        assert!(set_value(DEFAULT_TOML, "flash.min_interval_ms", "10").is_err());
        assert!(set_value(DEFAULT_TOML, "flash.nonsense", "1").is_err());
        assert!(set_value(DEFAULT_TOML, "agent.port", "http").is_err());
        assert!(set_value(DEFAULT_TOML, "project.match", "x").is_err());
    }

    #[test]
    fn set_value_adds_missing_tables_and_keys() {
        let out = set_value("[flash]\nstyle = \"edge\"\n", "quiet_hours.notify", "true").unwrap();
        assert!(out.ends_with("[quiet_hours]\nnotify = true\n"), "{out}");
        let out = set_value("[flash]\nstyle = \"edge\"\n", "flash.hold_ms", "900").unwrap();
        assert_eq!(out, "[flash]\nhold_ms = 900\nstyle = \"edge\"\n");
    }

    #[test]
    fn get_reads_dotted_keys() {
        let c = Config::default();
        assert_eq!(c.get("signals.question.color").as_deref(), Some("#08A9FF"));
        assert_eq!(c.get("agent.port").as_deref(), Some("47823"));
        assert_eq!(c.get("presence.away_after").as_deref(), Some("5m"));
        assert_eq!(c.get("flash"), None);
        assert_eq!(c.get("nope.nope"), None);
    }

    #[test]
    fn migrates_the_v1_ini_users_actually_have() {
        let ini = "# ClaudeFlash settings\nalpha=0.28\nalpha_ask=0.20\ncolor_done=#00FF5A\ncolor_ask=#08A9FF\n\
                   color_perm=#8B2FCE\nalpha_perm=0.17\nhold_ms=420\nperm_flash=on\nperm_modes=default,plan\n\
                   prompt_wait_ms=3000\nonly_your_sessions=on\nskip_if_focused=\n";
        let m = migrate_v1(ini);
        let c = Config::parse(&m.text).unwrap();
        assert!((c.signal(Attention::Approval).opacity - 0.17).abs() < 1e-9);
        assert_eq!(c.signal(Attention::Approval).color, Rgb::new(0x8B, 0x2F, 0xCE));
        assert!(c.signal(Attention::Approval).enabled && c.sessions.ignore_background);
        assert!(c.flash.skip_when_focused.is_empty());
        assert_eq!(m.notes.iter().filter(|n| n.contains("no longer needed")).count(), 2);
    }

    #[test]
    fn migration_keeps_defaults_for_invalid_v1_values() {
        let m = migrate_v1("color_done=violet\nalpha=7\n");
        let c = Config::parse(&m.text).unwrap();
        assert_eq!(c.signal(Attention::Done).color, Rgb::new(0xA8, 0x55, 0xF7));
        assert!((c.signal(Attention::Done).opacity - 0.28).abs() < 1e-9);
        assert!(m.notes.iter().any(|n| n.contains("signals.done.opacity")));
    }
}
