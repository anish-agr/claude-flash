//! `flash agent`, and the entry points Claude Code's hooks call.

use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use clap::Subcommand;
use flash_core::event::Attention;
use flash_core::settings::{self, SESSION_ENV_VAR};

use super::{Env, Outcome, agent_error};
use crate::api::{Control, Health};
use crate::{paths, system};

#[derive(Subcommand)]
pub enum AgentAction {
    /// Start the agent in the background, unless it is already running
    Start,
    /// Ask the agent to quit
    Stop,
    /// Stop the agent and start it again, for example after an upgrade
    Restart,
    /// Run the agent in this terminal, printing its log here
    Run {
        /// No overlays, tray or menu bar; flashes are only logged
        #[arg(long)]
        headless: bool,
        /// Listen on this port instead of agent.port
        #[arg(long)]
        port: Option<u16>,
    },
    /// Print the end of the agent's log
    Logs {
        #[arg(short = 'n', long, default_value_t = 40)]
        lines: usize,
    },
    /// Start the agent if needed, then pass on the hook event read from standard
    /// input. Claude Code runs this when a session starts.
    #[command(hide = true)]
    Ensure,
}

pub fn run(env: &Env, action: AgentAction) -> Outcome {
    match action {
        AgentAction::Start => {
            let health = start(env)?;
            println!("agent {} running · pid {} · 127.0.0.1:{}", health.version, health.pid, env.port());
        }
        AgentAction::Stop => {
            println!("{}", if stop(env)? { "agent stopped" } else { "the agent was not running" });
        }
        AgentAction::Restart => {
            stop(env)?;
            let health = start(env)?;
            println!("agent {} running · pid {} · 127.0.0.1:{}", health.version, health.pid, env.port());
        }
        AgentAction::Run { .. } => unreachable!("the agent runs from main, which owns the exit code"),
        AgentAction::Logs { lines } => {
            let path = env.paths.log_file();
            let text = fs::read_to_string(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
            let all: Vec<&str> = text.lines().collect();
            for line in &all[all.len().saturating_sub(lines)..] {
                println!("{line}");
            }
        }
        AgentAction::Ensure => ensure(env),
    }
    Ok(())
}

/// `flash-agent`, which lives next to `flash`.
pub fn agent_program() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate this executable: {e}"))?;
    let agent = exe.with_file_name(format!("flash-agent{}", std::env::consts::EXE_SUFFIX));
    if agent.is_file() {
        Ok(agent)
    } else {
        Err(format!("{} not found; flash-agent belongs next to flash", agent.display()))
    }
}

/// Starts the agent unless one is running, and waits until it answers.
pub fn start(env: &Env) -> Result<Health, String> {
    if let Ok(health) = env.client().health() {
        return Ok(health);
    }
    start_program(env, &agent_program()?)
}

/// Starts `program` as the agent unless one is running, and waits until it answers.
pub fn start_program(env: &Env, program: &Path) -> Result<Health, String> {
    let client = env.client();
    if let Ok(health) = client.health() {
        return Ok(health);
    }
    system::spawn_detached(program, &[]).map_err(|e| format!("could not start {}: {e}", program.display()))?;
    wait_for(Duration::from_secs(6), || client.health().ok())
        .ok_or_else(|| format!("the agent did not answer; its log is {}", paths::display(&env.paths.log_file())))
}

/// Asks the agent to quit and waits until it has. Returns whether one was running.
pub fn stop(env: &Env) -> Result<bool, String> {
    let client = env.client();
    if client.health().is_err() {
        return Ok(false);
    }
    client.control(&Control::Quit).map_err(agent_error)?;
    wait_for(Duration::from_secs(6), || client.health().is_err().then_some(()))
        .ok_or("the agent did not stop within six seconds")?;
    Ok(true)
}

fn wait_for<T>(limit: Duration, mut probe: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(value) = probe() {
            return Some(value);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// The SessionStart hook. Whatever happens it prints nothing and succeeds: output
/// would become part of Claude's context, and a failure would appear in the
/// transcript.
fn ensure(env: &Env) {
    let payload = hook_payload();
    let session = std::env::var(SESSION_ENV_VAR).unwrap_or_default();
    if settings::session_opted_out(&session) {
        return;
    }
    if start(env).is_ok()
        && let Some(payload) = payload
    {
        let _ = env.client().forward_hook(&payload, &session);
    }
}

/// A hook's JSON input, when standard input carries one. Reading gives up after two
/// seconds, so a launcher that leaves standard input open cannot hang the hook.
fn hook_payload() -> Option<Vec<u8>> {
    if io::stdin().is_terminal() {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = io::stdin().lock().take(1024 * 1024).read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    rx.recv_timeout(Duration::from_secs(2)).ok().filter(|buf| buf.iter().any(|b| !b.is_ascii_whitespace()))
}

const LEGACY_VERBS: [&str; 8] = ["done", "ask", "question", "input", "perm", "permission", "mark", "seen"];

pub fn is_legacy_verb(verb: &str) -> bool {
    LEGACY_VERBS.contains(&verb)
}

/// Claude Flash 1 registered hooks that run `flash <verb> [flags]`. Sessions started
/// before an upgrade keep those hooks until they restart, so the verbs still work:
/// the event on standard input goes to the agent like any other, and without one
/// the verb shows its test flash as it used to.
pub fn legacy(verb: &str) -> ExitCode {
    let env = Env::load();
    let session = std::env::var(SESSION_ENV_VAR).unwrap_or_default();
    match hook_payload() {
        Some(payload) => {
            if !settings::session_opted_out(&session) && start(&env).is_ok() {
                let _ = env.client().forward_hook(&payload, &session);
            }
        }
        None => {
            let kind = match verb {
                "done" => Some(Attention::Done),
                "ask" | "question" | "input" => Some(Attention::Question),
                "perm" | "permission" => Some(Attention::Approval),
                _ => None,
            };
            if let Some(kind) = kind
                && start(&env).is_ok()
            {
                let _ = env.client().control(&Control::Test { kind });
            }
        }
    }
    ExitCode::SUCCESS
}
