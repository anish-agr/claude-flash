//! `flash install` and `flash uninstall`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use clap::Args;
use flash_core::config;

use super::{Env, Outcome, agent, hooks, style};
use crate::store::{self, State};
use crate::{autostart, paths};

#[derive(Args)]
pub struct InstallArgs {
    /// Where to put flash and flash-agent [default: a folder already on PATH]
    #[arg(long)]
    bin_dir: Option<PathBuf>,
    /// Leave Claude Code's settings.json alone
    #[arg(long)]
    no_hooks: bool,
    /// Do not start the agent at login
    #[arg(long)]
    no_autostart: bool,
    /// Also add a desktop shortcut that turns flashes on and off (Windows)
    #[arg(long)]
    desktop_toggle: bool,
}

#[derive(Args)]
pub struct UninstallArgs {
    /// Also delete the settings, the journal and the saved state
    #[arg(long)]
    purge: bool,
}

pub fn install(env: &Env, args: &InstallArgs) -> Outcome {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate this executable: {e}"))?;
    let source = exe.parent().ok_or("this executable is not in a folder")?;
    let bin_dir = args.bin_dir.clone().unwrap_or_else(default_bin_dir);

    // A running agent holds its executable open on Windows, and an older one would
    // keep answering after the upgrade, so it stops first in every case.
    agent::stop(env)?;
    let (flash, flash_agent) = place_binaries(source, &bin_dir)?;
    step("programs", &paths::display(&bin_dir));

    for note in migrate_v1(env) {
        step("upgrade", &note);
    }

    let config_path = env.paths.config_file();
    if !config_path.exists() {
        store::write_atomic(&config_path, config::DEFAULT_TOML.as_bytes())
            .map_err(|e| format!("could not write {}: {e}", config_path.display()))?;
    }
    step("settings", &paths::display(&config_path));

    if !args.no_hooks {
        let settings = paths::claude_settings();
        let change = hooks::install(&settings, &flash, env.port(), false)?;
        let detail = if change.changed {
            format!("{} events in {}", change.added, paths::display(&settings))
        } else {
            format!("already current in {}", paths::display(&settings))
        };
        step("hooks", &detail);
    }

    if args.no_autostart {
        if autostart::registered().is_some() {
            autostart::disable().map_err(|e| format!("could not stop the agent starting at login: {e}"))?;
            step("at login", "the agent no longer starts when you log in");
        }
    } else {
        autostart::enable(&flash_agent).map_err(|e| format!("could not set the agent to start at login: {e}"))?;
        step("at login", "the agent starts when you log in");
    }

    let health = agent::start_program(env, &flash_agent)?;
    step("agent", &format!("{} running · pid {}", health.version, health.pid));

    if args.desktop_toggle {
        step("shortcut", &desktop_toggle(&flash_agent)?);
    }

    println!();
    println!("Claude Flash {} is installed. Run `flash test` to see each signal.", flash_core::VERSION);
    if !args.no_hooks {
        println!("{}", style::dim("Claude Code sessions that are already open use the new hooks once restarted."));
    }
    if !on_path(&bin_dir) {
        let hint = format!("{} is not on PATH; add it to run flash from any terminal.", paths::display(&bin_dir));
        println!("{}", style::yellow(&hint));
    }
    Ok(())
}

pub fn uninstall(env: &Env, args: &UninstallArgs) -> Outcome {
    if agent::stop(env)? {
        step("agent", "stopped");
    }
    let settings = paths::claude_settings();
    match hooks::uninstall(&settings, false)? {
        Some(change) if change.changed => {
            step("hooks", &format!("removed {} from {}", change.removed, paths::display(&settings)));
        }
        _ => step("hooks", "none were installed"),
    }
    autostart::disable().map_err(|e| format!("could not stop the agent starting at login: {e}"))?;
    step("at login", "no longer starts");
    if remove_desktop_toggle() {
        step("shortcut", "removed from the desktop");
    }
    if args.purge {
        for dir in [&env.paths.data_dir, &env.paths.config_dir] {
            match fs::remove_dir_all(dir) {
                Ok(()) => step("data", &format!("deleted {}", paths::display(dir))),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("could not delete {}: {e}", dir.display())),
            }
        }
    }
    println!();
    println!("Claude Flash is uninstalled.");
    if let Some(dir) = std::env::current_exe().ok().as_deref().and_then(Path::parent) {
        let hint = format!("Delete flash and flash-agent from {} to remove the programs as well.", paths::display(dir));
        println!("{}", style::dim(&hint));
    }
    Ok(())
}

fn step(what: &str, detail: &str) {
    println!("{} {what:<10}{detail}", style::green("✓"));
}

fn default_bin_dir() -> PathBuf {
    #[cfg(windows)]
    {
        // On PATH for every Windows 10 and 11 user, and where version 1 installed.
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| paths::home().join("AppData").join("Local"))
            .join("Microsoft")
            .join("WindowsApps")
    }
    #[cfg(not(windows))]
    {
        paths::home().join(".local").join("bin")
    }
}

/// Copies `flash` and `flash-agent` from `source` into `bin_dir`, unless that is
/// where they already are.
fn place_binaries(source: &Path, bin_dir: &Path) -> Result<(PathBuf, PathBuf), String> {
    let names =
        [format!("flash{}", std::env::consts::EXE_SUFFIX), format!("flash-agent{}", std::env::consts::EXE_SUFFIX)];
    let placed = (bin_dir.join(&names[0]), bin_dir.join(&names[1]));
    if !same_dir(source, bin_dir) {
        fs::create_dir_all(bin_dir).map_err(|e| format!("could not create {}: {e}", bin_dir.display()))?;
        for name in &names {
            replace(&source.join(name), &bin_dir.join(name))?;
        }
    }
    if !placed.1.is_file() {
        return Err(format!("{} is missing", placed.1.display()));
    }
    Ok(placed)
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn on_path(dir: &Path) -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|entry| same_dir(&entry, dir)))
}

/// Puts a copy of `from` at `to`. Windows will not overwrite a running executable
/// but will rename one, so the old file steps aside when it has to.
fn replace(from: &Path, to: &Path) -> Result<(), String> {
    if !from.is_file() {
        return Err(format!("{} is missing; flash and flash-agent must be in the same folder", from.display()));
    }
    let staged = to.with_extension("new");
    fs::copy(from, &staged).map_err(|e| format!("could not copy to {}: {e}", staged.display()))?;
    if fs::rename(&staged, to).is_ok() {
        return Ok(());
    }
    let old = to.with_extension("old");
    let _ = fs::remove_file(&old);
    fs::rename(to, &old).and_then(|()| fs::rename(&staged, to)).map_err(|e| {
        let _ = fs::remove_file(&staged);
        format!("could not replace {}: {e}", to.display())
    })
}

/// Carries Claude Flash 1's settings forward and removes the files it kept for its
/// own bookkeeping.
fn migrate_v1(env: &Env) -> Vec<String> {
    let dir = &env.paths.data_dir;
    let mut notes = Vec::new();
    let ini = dir.join("config.ini");
    let toml = env.paths.config_file();
    if ini.is_file()
        && !toml.exists()
        && let Ok(text) = fs::read_to_string(&ini)
    {
        let migration = config::migrate_v1(&text);
        if store::write_atomic(&toml, migration.text.as_bytes()).is_ok() {
            notes.push("converted config.ini to config.toml".to_owned());
            notes.extend(migration.notes);
            let _ = fs::rename(&ini, dir.join("config.v1.ini"));
        }
    }
    let marker = dir.join("disabled");
    if marker.is_file() {
        let path = env.paths.state_file();
        let mut state = State::load(&path);
        state.enabled = false;
        if state.save(&path).is_ok() && fs::remove_file(&marker).is_ok() {
            notes.push("flashes stay off, as they were before the upgrade".to_owned());
        }
    }
    let mut cleared = false;
    for name in ["pending", "sessions"] {
        cleared |= fs::remove_dir_all(dir.join(name)).is_ok();
    }
    for name in ["lastflash", "relaunch.log"] {
        cleared |= fs::remove_file(dir.join(name)).is_ok();
    }
    if cleared {
        notes.push("removed version 1's bookkeeping files".to_owned());
    }
    notes
}

#[cfg(windows)]
const SHORTCUT: &str = "Claude Flash toggle.lnk";

#[cfg(windows)]
fn powershell(script: &str, agent: Option<&Path>) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", script]).creation_flags(CREATE_NO_WINDOW);
    command.env("CLAUDE_FLASH_SHORTCUT", SHORTCUT);
    if let Some(agent) = agent {
        command.env("CLAUDE_FLASH_AGENT", agent);
    }
    let output = command.output().map_err(|e| format!("could not run PowerShell: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// A desktop shortcut that runs `flash-agent toggle`, with the agent's own icon.
#[cfg(windows)]
fn desktop_toggle(agent: &Path) -> Result<String, String> {
    const SCRIPT: &str = "$ErrorActionPreference = 'Stop'; \
        $path = Join-Path ([Environment]::GetFolderPath('Desktop')) $env:CLAUDE_FLASH_SHORTCUT; \
        $link = (New-Object -ComObject WScript.Shell).CreateShortcut($path); \
        $link.TargetPath = $env:CLAUDE_FLASH_AGENT; \
        $link.Arguments = 'toggle'; \
        $link.IconLocation = $env:CLAUDE_FLASH_AGENT + ',0'; \
        $link.Description = 'Turn Claude Flash on or off'; \
        $link.Save(); \
        $path";
    powershell(SCRIPT, Some(agent)).map_err(|e| format!("could not create the desktop shortcut: {e}"))
}

#[cfg(not(windows))]
fn desktop_toggle(_agent: &Path) -> Result<String, String> {
    Err("--desktop-toggle is for Windows; on macOS the menu bar item has the switch".to_owned())
}

#[cfg(windows)]
fn remove_desktop_toggle() -> bool {
    const SCRIPT: &str = "$path = Join-Path ([Environment]::GetFolderPath('Desktop')) $env:CLAUDE_FLASH_SHORTCUT; \
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path; 'removed' }";
    powershell(SCRIPT, None).is_ok_and(|out| out == "removed")
}

#[cfg(not(windows))]
fn remove_desktop_toggle() -> bool {
    false
}
