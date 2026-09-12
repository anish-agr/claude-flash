//! `flash hooks`: Claude Flash's entries in Claude Code's `settings.json`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use clap::Subcommand;
use flash_core::settings::{self, Change, Install, RemoteAgent};
use serde_json::Value;

use super::{Env, Outcome, style};
use crate::{paths, store};

#[derive(Subcommand)]
pub enum HooksAction {
    /// Add Claude Flash's hooks, replacing any older ones
    Install {
        /// A settings file other than ~/.claude/settings.json
        #[arg(long)]
        settings: Option<PathBuf>,
        /// Print the result instead of writing it
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove Claude Flash's hooks and nothing else
    Uninstall {
        #[arg(long)]
        settings: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Show whether the hooks are installed and current
    Status {
        #[arg(long)]
        settings: Option<PathBuf>,
    },
    /// Print hooks for another machine, such as WSL, that point at this agent
    Remote {
        /// This machine, as the other one reaches it: an address or a host name
        host: String,
    },
}

pub fn run(env: &Env, action: HooksAction) -> Outcome {
    match action {
        HooksAction::Install { settings, dry_run } => {
            let path = settings.unwrap_or_else(paths::claude_settings);
            let program = std::env::current_exe().map_err(|e| format!("cannot locate this executable: {e}"))?;
            let change = install(&path, &program, env.port(), dry_run)?;
            describe(&path, &change, dry_run);
        }
        HooksAction::Uninstall { settings, dry_run } => {
            let path = settings.unwrap_or_else(paths::claude_settings);
            match uninstall(&path, dry_run)? {
                Some(change) => describe(&path, &change, dry_run),
                None => println!("{} does not exist; nothing to remove", paths::display(&path)),
            }
        }
        HooksAction::Status { settings } => {
            let path = settings.unwrap_or_else(paths::claude_settings);
            println!("{}  {}", paths::display(&path), inspect(&path, env.port()));
        }
        HooksAction::Remote { host } => remote(env, &host)?,
    }
    Ok(())
}

/// Prints a `settings.json` for Claude Code on another machine, with hooks that
/// reach this agent and carry its token. Claude Flash itself is not needed there.
fn remote(env: &Env, host: &str) -> Outcome {
    let token = store::read_token(&env.paths.token_file())
        .ok_or("there is no token yet; start the agent once with `flash agent start`")?;
    let url = format!("http://{host}:{}", env.port());
    let spec = Install { port: env.port(), program: String::new(), remote: Some(RemoteAgent { url, token }) };
    let change = settings::install(None, &spec).map_err(|e| e.to_string())?;
    // The settings go to stdout so they can be redirected to a file; everything the
    // reader has to do about it goes to stderr.
    println!("{}", change.text);
    if !env.config.agent.remote {
        eprintln!(
            "{} this agent still listens on loopback only. Run `flash config set agent.remote true` and restart it,",
            style::yellow("note:")
        );
        eprintln!("      or the other machine cannot reach it.");
    }
    eprintln!("Merge the hooks above into ~/.claude/settings.json on the other machine.");
    eprintln!("They carry this agent's token, so treat that file as a secret.");
    Ok(())
}

/// Installs the hooks in `path`, keeping a copy of the previous file.
pub fn install(path: &Path, program: &Path, port: u16, dry_run: bool) -> Result<Change, String> {
    let original = read(path)?;
    let spec = Install { port, program: program.display().to_string(), remote: None };
    let change = settings::install(original.as_deref(), &spec).map_err(|e| e.to_string())?;
    if change.changed && !dry_run {
        write(path, original.as_deref(), &change.text)?;
    }
    Ok(change)
}

pub fn uninstall(path: &Path, dry_run: bool) -> Result<Option<Change>, String> {
    let Some(original) = read(path)? else { return Ok(None) };
    let change = settings::uninstall(&original).map_err(|e| e.to_string())?;
    if change.changed && !dry_run {
        write(path, Some(&original), &change.text)?;
    }
    Ok(Some(change))
}

fn read(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("could not read {}: {e}", path.display())),
    }
}

fn write(path: &Path, original: Option<&str>, text: &str) -> Result<(), String> {
    if let Some(original) = original {
        let name = path.file_name().map_or_else(|| "settings.json".into(), |n| n.to_string_lossy().into_owned());
        let backup = path.with_file_name(format!("{name}.claude-flash-backup"));
        store::write_atomic(&backup, original.as_bytes())
            .map_err(|e| format!("could not back up {}: {e}", path.display()))?;
    }
    store::write_atomic(path, text.as_bytes()).map_err(|e| format!("could not write {}: {e}", path.display()))
}

fn describe(path: &Path, change: &Change, dry_run: bool) {
    let shown = paths::display(path);
    if !change.changed {
        println!("{shown} is already up to date");
        return;
    }
    if dry_run {
        print!("{}", change.text);
        println!("{}", style::dim(&format!("(dry run: {shown} was not changed)")));
        return;
    }
    if change.added > 0 {
        let replaced =
            if change.removed > 0 { format!(", replacing {} older ones", change.removed) } else { String::new() };
        println!("added {} hooks to {shown}{replaced}", change.added);
        println!("{}", style::dim("Claude Code reads hooks when a session starts; restart open sessions to use them."));
    } else {
        println!("removed {} hooks from {shown}", change.removed);
    }
}

/// The `flash` executable that the installed SessionStart hook runs.
pub fn installed_program(settings_text: &str) -> Option<String> {
    let root: Value = serde_json::from_str(settings_text.trim_start_matches('\u{feff}')).ok()?;
    root.get("hooks")?
        .get("SessionStart")?
        .as_array()?
        .iter()
        .filter_map(|group| group.get("hooks")?.as_array())
        .flatten()
        .find(|hook| hook.get("args").is_some() && settings::is_flash_hook(hook))
        .and_then(|hook| Some(hook.get("command")?.as_str()?.to_owned()))
}

/// One line on the state of the hooks in `path`.
pub fn inspect(path: &Path, port: u16) -> String {
    let text = match read(path) {
        Ok(Some(text)) => text,
        Ok(None) => return style::yellow("not installed · run `flash hooks install`"),
        Err(e) => return style::red(&e),
    };
    let program = installed_program(&text);
    let spec = Install { port, program: program.clone().unwrap_or_default(), remote: None };
    match settings::inspect(&text, &spec) {
        Err(e) => style::red(&e.to_string()),
        Ok(_) if settings::hooks_disabled(&text) => style::red("installed, but disableAllHooks turns every hook off"),
        Ok(report) if report.current == 0 && report.outdated == 0 => {
            style::yellow("not installed · run `flash hooks install`")
        }
        Ok(report) if report.up_to_date() => match program {
            Some(program) if !Path::new(&program).is_file() => {
                style::red(&format!("installed, but {program} is missing · run `flash hooks install`"))
            }
            _ => style::green("installed"),
        },
        Ok(_) => style::yellow("out of date · run `flash hooks install`"),
    }
}

pub fn summary(env: &Env) -> String {
    inspect(&paths::claude_settings(), env.port())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_program_the_session_start_hook_runs() {
        let spec = Install { port: 47_823, program: "/opt/flash/bin/flash".into(), remote: None };
        let text = settings::install(None, &spec).unwrap().text;
        assert_eq!(installed_program(&text).as_deref(), Some("/opt/flash/bin/flash"));
        assert_eq!(installed_program("{}"), None);
    }
}
