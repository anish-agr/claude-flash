//! Runs every scenario in `spec/scenarios` against the policy engine.
//!
//! The scenarios are Claude Flash's behavioural specification. Each feeds a sequence
//! of hook events, API signals, control commands and clock movements to the engine
//! and states exactly which flashes, notifications and suppressions must follow.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use flash_core::config::Config;
use flash_core::duration;
use flash_core::engine::{Action, Context, Control, Engine, Input, describe};
use flash_core::event::{Attention, HookEvent, Signal};
use flash_core::time;
use serde::Deserialize;
use serde_json::Value;

/// 2026-09-10T12:00:00Z, the default start of every scenario.
const DEFAULT_START: u64 = 1_789_041_600_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    title: String,
    #[serde(default)]
    config: Option<String>,
    #[serde(default)]
    start: Option<String>,
    #[serde(default)]
    utc_offset_min: i32,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    at: String,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    idle: Option<String>,
    #[serde(default)]
    focused: Option<String>,
    #[serde(default)]
    hook: Option<Value>,
    #[serde(default)]
    signal: Option<Value>,
    #[serde(default)]
    control: Option<Value>,
    #[serde(default)]
    tick: bool,
    /// Exact, ordered descriptions of what the step must produce.
    #[serde(default)]
    expect: Option<Vec<String>>,
    /// Colours of the flashes the step must start, in order.
    #[serde(default)]
    flash_colors: Option<Vec<String>>,
    /// How many waits must be outstanding afterwards.
    #[serde(default)]
    waiting: Option<usize>,
}

#[test]
fn every_scenario_holds() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec/scenarios");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no scenarios in {}", dir.display());

    let mut seen = BTreeSet::new();
    let failures: Vec<String> = paths.iter().filter_map(|p| run(p, &mut seen).err()).collect();
    assert!(
        failures.is_empty(),
        "{} of {} scenarios failed:\n\n{}",
        failures.len(),
        paths.len(),
        failures.join("\n\n")
    );

    // The specification must exercise every outcome the engine can produce.
    let mut required: Vec<String> = Attention::ALL.iter().map(|k| format!("flash {k}")).collect();
    required.extend(
        ["disabled", "paused", "muted", "signal_off", "background", "duplicate", "quiet_hours", "focused"]
            .map(|r| format!("suppressed {r}")),
    );
    required.extend(["notify approval".to_owned(), "push approval".to_owned()]);
    let missing: Vec<&String> = required.iter().filter(|r| !seen.contains(*r)).collect();
    assert!(missing.is_empty(), "no scenario covers: {missing:?}");
}

fn run(path: &Path, seen: &mut BTreeSet<String>) -> Result<(), String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let text = fs::read_to_string(path).map_err(|e| format!("{name}: {e}"))?;
    let scenario: Scenario = serde_json::from_str(&text).map_err(|e| format!("{name}: not a valid scenario: {e}"))?;
    let config = match &scenario.config {
        Some(toml) => Config::parse(toml).map_err(|e| format!("{name}: config: {e}"))?,
        None => Config::default(),
    };
    let start = match &scenario.start {
        Some(s) => time::parse_rfc3339(s).ok_or_else(|| format!("{name}: start is not RFC 3339: {s:?}"))?,
        None => DEFAULT_START,
    };

    let mut engine = Engine::new(config);
    let mut previous = 0;
    for (i, step) in scenario.steps.iter().enumerate() {
        let label = format!("{name}, step {} — {}", i + 1, step.note.as_deref().unwrap_or(&scenario.title));
        let at = duration::parse(&step.at).map_err(|e| format!("{label}: at: {e}"))?.ms();
        if at < previous {
            return Err(format!("{label}: time runs backwards ({} after {})", step.at, duration::format(previous)));
        }
        previous = at;
        let idle = match &step.idle {
            Some(s) => duration::parse(s).map_err(|e| format!("{label}: idle: {e}"))?.ms(),
            None => 0,
        };
        let ctx = Context {
            now_ms: at,
            unix_ms: start + at,
            utc_offset_min: scenario.utc_offset_min,
            idle_ms: idle,
            focused_app: step.focused.clone(),
        };
        let input = input_for(step).map_err(|e| format!("{label}: {e}"))?;
        let actions = engine.handle(input, &ctx);

        let described: Vec<String> = actions.iter().filter_map(describe).collect();
        seen.extend(described.iter().cloned());
        if let Some(expected) = &step.expect
            && &described != expected
        {
            return Err(format!("{label}\n  expected: {expected:?}\n  actual:   {described:?}"));
        }
        if let Some(expected) = &step.flash_colors {
            let actual: Vec<String> = actions
                .iter()
                .filter_map(|a| if let Action::Flash(f) = a { Some(f.color.to_hex()) } else { None })
                .collect();
            let expected: Vec<String> = expected.iter().map(|c| c.to_ascii_uppercase()).collect();
            if actual != expected {
                return Err(format!(
                    "{label}\n  expected flash colours: {expected:?}\n  actual:                 {actual:?}"
                ));
            }
        }
        if let Some(expected) = step.waiting {
            let actual = engine.snapshot(&ctx).waiting.len();
            if actual != expected {
                return Err(format!("{label}\n  expected {expected} outstanding waits, found {actual}"));
            }
        }
    }
    Ok(())
}

fn input_for(step: &Step) -> Result<Input, String> {
    let given = [step.hook.is_some(), step.signal.is_some(), step.control.is_some(), step.tick]
        .into_iter()
        .filter(|&b| b)
        .count();
    if given != 1 {
        return Err("each step needs exactly one of hook, signal, control or tick".to_owned());
    }
    if let Some(v) = &step.hook {
        return HookEvent::from_value(v).map(Input::Hook).map_err(|e| e.to_string());
    }
    if let Some(v) = &step.signal {
        return serde_json::from_value::<Signal>(v.clone()).map(Input::Signal).map_err(|e| format!("signal: {e}"));
    }
    if let Some(v) = &step.control {
        return control(v).map(Input::Control);
    }
    Ok(Input::Tick)
}

fn control(v: &Value) -> Result<Control, String> {
    match v {
        Value::String(s) => match s.as_str() {
            "enable" => Ok(Control::Enable),
            "disable" => Ok(Control::Disable),
            "resume" => Ok(Control::Resume),
            other => Err(format!("unknown control {other:?}")),
        },
        Value::Object(map) if map.len() == 1 => {
            let (key, arg) = map.iter().next().expect("one entry");
            let arg = arg.as_str().ok_or("a control's argument must be a string")?;
            match key.as_str() {
                "pause" => duration::parse(arg).map(|s| Control::Pause { for_ms: s.ms() }).map_err(|e| e.to_string()),
                "test" => {
                    Attention::parse(arg).map(Control::Test).ok_or_else(|| format!("unknown signal kind {arg:?}"))
                }
                other => Err(format!("unknown control {other:?}")),
            }
        }
        _ => Err("control must be a string or a single-key object".to_owned()),
    }
}
