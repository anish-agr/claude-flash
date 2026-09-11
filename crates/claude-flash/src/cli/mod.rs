//! The `flash` command line.

mod agent;
mod config;
mod doctor;
mod hooks;
mod install;
mod report;
mod style;

use std::fs;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use flash_core::config::{Config, MIN_FLASH_INTERVAL_MS};
use flash_core::duration;
use flash_core::event::Attention;
use serde_json::json;

use crate::api::Control;
use crate::client::{Client, ClientError};
use crate::paths::Paths;
use crate::store::{self, State};
use crate::system;

#[derive(Parser)]
#[command(
    name = "flash",
    version,
    about = "Screen-wide flashes when Claude Code finishes, asks a question, needs approval or fails",
    after_help = "Run `flash install` once to set everything up, then `flash test` to see each signal."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Show the switch, what Claude is waiting on, and today's counts
    Status {
        /// Print the agent's full status as JSON
        #[arg(long)]
        json: bool,
    },
    /// Turn flashes on
    On,
    /// Turn flashes off; the hooks stay installed and the agent keeps running
    Off,
    /// Turn flashes off if they are on, and on if they are off
    Toggle,
    /// Hold every signal for a while
    Pause {
        /// How long, such as 90s, 15m or 1h30m
        duration: String,
    },
    /// End a pause early
    Resume,
    /// Show a test flash for each signal, or for the ones named
    Test {
        #[arg(value_enum)]
        kinds: Vec<Kind>,
    },
    /// Raise a signal from a script, a build or another tool
    Signal {
        #[arg(value_enum)]
        kind: Kind,
        /// Notification title, used when you are away
        #[arg(long)]
        title: Option<String>,
        /// Notification text
        #[arg(long)]
        body: Option<String>,
        /// What raised it, such as `make` or `ci`
        #[arg(long)]
        source: Option<String>,
        /// Project name, matched against [[project]] rules
        #[arg(long)]
        project: Option<String>,
    },
    /// Follow signals as they happen
    Watch {
        /// One JSON record per line
        #[arg(long)]
        json: bool,
    },
    /// Show what the journal recorded
    Log {
        /// How far back to look, such as 30m, 6h or 7d
        #[arg(long, default_value = "1d")]
        since: String,
        /// Only this signal
        #[arg(long, value_enum)]
        kind: Option<Kind>,
        /// Only this project
        #[arg(long)]
        project: Option<String>,
        /// Include prompts and session starts and ends
        #[arg(long)]
        all: bool,
        /// Show at most this many entries, the most recent ones
        #[arg(short = 'n', long, default_value_t = 50)]
        limit: usize,
        /// One JSON record per line
        #[arg(long)]
        json: bool,
    },
    /// Summarise the journal: signals, time Claude spent waiting, busiest hours
    Stats {
        /// How far back to look
        #[arg(long, default_value = "7d")]
        since: String,
        #[arg(long)]
        json: bool,
    },
    /// Read and change settings
    Config {
        #[command(subcommand)]
        action: Option<config::ConfigAction>,
    },
    /// Add, remove or check the hooks in Claude Code's settings
    Hooks {
        #[command(subcommand)]
        action: hooks::HooksAction,
    },
    /// Start, stop or run the background agent
    Agent {
        #[command(subcommand)]
        action: agent::AgentAction,
    },
    /// Set up for this user: binaries, hooks, start at login, and the agent
    Install(install::InstallArgs),
    /// Remove the hooks and start at login, and stop the agent
    Uninstall(install::UninstallArgs),
    /// Check each part of the setup and say how to fix what is wrong
    Doctor,
    /// Print a shell completion script
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Kind {
    /// Claude finished its turn
    Done,
    /// Claude asked you something
    #[value(alias = "ask")]
    Question,
    /// Claude needs permission to use a tool
    #[value(alias = "perm", alias = "permission")]
    Approval,
    /// The turn ended in an API error
    Error,
}

impl From<Kind> for Attention {
    fn from(kind: Kind) -> Attention {
        match kind {
            Kind::Done => Attention::Done,
            Kind::Question => Attention::Question,
            Kind::Approval => Attention::Approval,
            Kind::Error => Attention::Error,
        }
    }
}

type Outcome = Result<(), String>;

/// What every command needs: where the files are, and how to reach the agent.
struct Env {
    paths: Paths,
    config: Config,
}

impl Env {
    fn load() -> Env {
        let paths = Paths::resolve();
        let config =
            fs::read_to_string(paths.config_file()).ok().and_then(|text| Config::parse(&text).ok()).unwrap_or_default();
        Env { paths, config }
    }

    fn port(&self) -> u16 {
        self.config.agent.port
    }

    /// A client carrying the token as it is on disk now. The agent creates the token
    /// on first start, so build a fresh client after starting one.
    fn client(&self) -> Client {
        Client::new(self.port(), store::read_token(&self.paths.token_file()))
    }
}

fn not_running() -> String {
    "the agent is not running; start it with `flash agent start`".to_owned()
}

fn agent_error(error: ClientError) -> String {
    match error {
        ClientError::NotRunning => not_running(),
        other => other.to_string(),
    }
}

pub fn main() -> ExitCode {
    if let Some(verb) = std::env::args().nth(1)
        && agent::is_legacy_verb(&verb)
    {
        return agent::legacy(&verb);
    }
    let cli = Cli::parse();
    style::init();
    let env = Env::load();
    let outcome = match cli.command {
        None | Some(Command::Status { json: false }) => report::status(&env, false),
        Some(Command::Status { json: true }) => report::status(&env, true),
        Some(Command::On) => switch(&env, Control::On),
        Some(Command::Off) => switch(&env, Control::Off),
        Some(Command::Toggle) => switch(&env, Control::Toggle { confirm: false }),
        Some(Command::Pause { duration }) => switch(&env, Control::Pause { duration }),
        Some(Command::Resume) => switch(&env, Control::Resume),
        Some(Command::Test { kinds }) => test(&env, &kinds),
        Some(Command::Signal { kind, title, body, source, project }) => {
            let signal = json!({
                "kind": Attention::from(kind),
                "title": title,
                "body": body,
                "source": source.unwrap_or_else(|| "cli".to_owned()),
                "project": project,
            });
            env.client().signal(&signal).map_err(agent_error)
        }
        Some(Command::Watch { json }) => report::watch(&env, json),
        Some(Command::Log { since, kind, project, all, limit, json }) => {
            let filter = report::LogFilter { kind: kind.map(Into::into), project, all, limit };
            report::log(&env, &since, &filter, json)
        }
        Some(Command::Stats { since, json }) => report::stats(&env, &since, json),
        Some(Command::Config { action }) => config::run(&env, action),
        Some(Command::Hooks { action }) => hooks::run(&env, action),
        Some(Command::Agent { action: agent::AgentAction::Run { headless, port } }) => {
            return crate::agent::run(crate::agent::Options { headless, port, echo: true, toggle: false });
        }
        Some(Command::Agent { action }) => agent::run(&env, action),
        Some(Command::Install(args)) => install::install(&env, &args),
        Some(Command::Uninstall(args)) => install::uninstall(&env, &args),
        Some(Command::Doctor) => doctor::run(&env),
        Some(Command::Completions { shell }) => {
            clap_complete::generate(shell, &mut Cli::command(), "flash", &mut std::io::stdout());
            Ok(())
        }
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{} {message}", style::red("error:"));
            ExitCode::FAILURE
        }
    }
}

/// Changes a switch through the agent, or in the saved state when it is not running,
/// so the next start picks it up.
fn switch(env: &Env, control: Control) -> Outcome {
    if let Control::Pause { duration } = &control {
        duration::parse(duration).map_err(|e| e.to_string())?;
    }
    match env.client().control(&control) {
        Ok(status) => {
            println!("{}", report::switch_line(status.enabled, status.paused_for_ms, status.away));
            Ok(())
        }
        Err(ClientError::NotRunning) => {
            let path = env.paths.state_file();
            let mut state = State::load(&path);
            match &control {
                Control::On => state.enabled = true,
                Control::Off => state.enabled = false,
                Control::Toggle { .. } => state.enabled = !state.enabled,
                Control::Resume => state.paused_until = None,
                Control::Pause { duration } => {
                    let ms = duration::parse(duration).map_err(|e| e.to_string())?.ms();
                    state.paused_until = (ms > 0).then(|| system::unix_ms() + ms);
                }
                _ => return Err(not_running()),
            }
            state.save(&path).map_err(|e| format!("could not save {}: {e}", path.display()))?;
            let paused = state.paused_until.map(|until| until.saturating_sub(system::unix_ms())).filter(|ms| *ms > 0);
            println!(
                "{}  {}",
                report::switch_line(state.enabled, paused, false),
                style::dim("saved for when the agent starts")
            );
            Ok(())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// The order in which to show the signals when none are named.
const TOUR: [Attention; 4] = [Attention::Done, Attention::Question, Attention::Approval, Attention::Error];

fn test(env: &Env, kinds: &[Kind]) -> Outcome {
    let kinds: Vec<Attention> =
        if kinds.is_empty() { TOUR.to_vec() } else { kinds.iter().map(|k| Attention::from(*k)).collect() };
    let flash = &env.config.flash;
    // Let each flash finish before the next starts; the agent would hold a weaker
    // one back anyway if it arrived inside the minimum interval.
    let length = flash.fade_in_ms + flash.hold_ms + flash.fade_out_ms;
    let gap = Duration::from_millis(u64::from(length.max(flash.min_interval_ms).max(MIN_FLASH_INTERVAL_MS)) + 150);
    let client = env.client();
    for (i, kind) in kinds.iter().enumerate() {
        if i > 0 {
            thread::sleep(gap);
        }
        client.control(&Control::Test { kind: *kind }).map_err(agent_error)?;
        println!("{}", style::kind(*kind, kind.as_str()));
    }
    Ok(())
}
