//! Where Agent Flash keeps its files.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// Holds `config.toml`.
    pub config_dir: PathBuf,
    /// Holds the persisted state, the API token, the agent log and the journal.
    pub data_dir: PathBuf,
}

impl Paths {
    /// The platform's usual locations, or everything under `AGENT_FLASH_HOME` when
    /// that is set. `CLAUDE_FLASH_HOME` still works: it is what this program was
    /// called before it covered more than one agent.
    pub fn resolve() -> Paths {
        let chosen =
            ["AGENT_FLASH_HOME", "CLAUDE_FLASH_HOME"].iter().find_map(|var| env::var_os(var).filter(|v| !v.is_empty()));
        match chosen {
            Some(dir) => Paths::at(dir),
            None => platform_paths(),
        }
    }

    /// Everything in one directory.
    pub fn at(dir: impl Into<PathBuf>) -> Paths {
        let dir = dir.into();
        Paths { config_dir: dir.clone(), data_dir: dir }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn state_file(&self) -> PathBuf {
        self.data_dir.join("state.json")
    }

    pub fn token_file(&self) -> PathBuf {
        self.data_dir.join("token")
    }

    pub fn journal_dir(&self) -> PathBuf {
        self.data_dir.join("journal")
    }

    pub fn log_file(&self) -> PathBuf {
        self.data_dir.join("agent.log")
    }
}

#[cfg(windows)]
fn platform_paths() -> Paths {
    // The profile root, next to Claude Code's own `~/.claude`, not a folder under
    // `AppData`. A packaged host such as the Claude desktop app redirects the writes
    // its child processes make under `AppData` into a private per-app copy, so a
    // token or settings file written there from inside the app is invisible to the
    // user's own terminals, and the reverse. The profile root is not redirected, so
    // every process sees one Agent Flash. `relocate_legacy_data` carries an older
    // install's files here.
    Paths::at(home().join(".agent-flash"))
}

/// Where Windows builds before this one kept their data.
#[cfg(windows)]
fn legacy_windows_data_dir() -> PathBuf {
    let local = env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData").join("Local"));
    local.join("ClaudeFlash")
}

#[cfg(target_os = "macos")]
fn platform_paths() -> Paths {
    Paths::at(home().join("Library").join("Application Support").join("agent-flash"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_paths() -> Paths {
    let xdg = |var: &str, fallback: &str| {
        env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(fallback))
    };
    Paths {
        config_dir: xdg("XDG_CONFIG_HOME", ".config").join("agent-flash"),
        data_dir: xdg("XDG_STATE_HOME", ".local/state").join("agent-flash"),
    }
}

pub fn home() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// Where older versions kept their files, newest first. This program was called
/// Claude Flash until it covered more than one agent, and on Windows version 2.2 and
/// earlier kept everything under `%LOCALAPPDATA%`.
fn legacy_paths() -> Vec<Paths> {
    #[cfg(windows)]
    let all = vec![Paths::at(home().join(".claude-flash")), Paths::at(legacy_windows_data_dir())];
    #[cfg(target_os = "macos")]
    let all = vec![Paths::at(home().join("Library").join("Application Support").join("claude-flash"))];
    #[cfg(all(unix, not(target_os = "macos")))]
    let all = {
        let xdg = |var: &str, fallback: &str| {
            env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(fallback))
        };
        vec![Paths {
            config_dir: xdg("XDG_CONFIG_HOME", ".config").join("claude-flash"),
            data_dir: xdg("XDG_STATE_HOME", ".local/state").join("claude-flash"),
        }]
    };
    all
}

/// Carries an older version's files into the current location the first time a build
/// that uses it runs, so nobody loses a token, settings or the journal when the
/// program is renamed or moves house. It copies rather than moves and only fills
/// gaps, so running it twice is safe, and the old folder goes only once its settings
/// and token are known to have arrived. It does nothing when a home was chosen by
/// hand, or when the current location already holds settings.
pub fn relocate_legacy_data(active: &Paths) {
    // A home from the environment points somewhere the user chose; leave it alone.
    if *active != platform_paths() || active.config_file().exists() {
        return;
    }
    for old in legacy_paths() {
        if old == *active || !(old.config_file().exists() || old.token_file().exists()) {
            continue;
        }
        let copied = copy_tree_missing(&old.config_dir, &active.config_dir).is_ok()
            && (old.data_dir == old.config_dir || copy_tree_missing(&old.data_dir, &active.data_dir).is_ok());
        if copied && migrated(&old, active) {
            let _ = fs::remove_dir_all(&old.data_dir);
            if old.config_dir != old.data_dir {
                let _ = fs::remove_dir_all(&old.config_dir);
            }
            return;
        }
    }
}

/// Every folder an older version used, so `uninstall --purge` can clear them too.
pub fn legacy_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for old in legacy_paths() {
        for dir in [old.data_dir, old.config_dir] {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    dirs
}

/// Whether the settings and, if there was one, the token reached `new`.
fn migrated(old: &Paths, new: &Paths) -> bool {
    if !new.config_file().exists() {
        return false;
    }
    let old_token = old.token_file();
    !old_token.exists() || matches!((fs::read(&old_token), fs::read(new.token_file())), (Ok(a), Ok(b)) if a == b)
}

/// Copies every file under `from` into `to` that `to` does not already have,
/// recursing into subfolders. An existing file in `to` is kept as it is.
fn copy_tree_missing(from: &Path, to: &Path) -> io::Result<()> {
    if !from.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if entry.file_type()?.is_dir() {
            copy_tree_missing(&src, &dst)?;
        } else if !dst.exists() {
            fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// Gemini CLI's user-level `settings.json`, which holds its hooks.
pub fn gemini_settings() -> PathBuf {
    match env::var_os("GEMINI_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("settings.json"),
        None => home().join(".gemini").join("settings.json"),
    }
}

/// Codex's `hooks.json`, where its own hooks live.
pub fn codex_hooks() -> PathBuf {
    match env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("hooks.json"),
        None => home().join(".codex").join("hooks.json"),
    }
}

/// Claude Code's user-level `settings.json`.
pub fn claude_settings() -> PathBuf {
    match env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("settings.json"),
        None => home().join(".claude").join("settings.json"),
    }
}

/// A path for display, with the home directory shortened to `~`.
pub fn display(path: &Path) -> String {
    let home = home();
    match path.strip_prefix(&home) {
        Ok(rest) if home != Path::new(".") => {
            let sep = std::path::MAIN_SEPARATOR;
            format!("~{sep}{}", rest.display())
        }
        _ => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_lives_under_an_explicit_home() {
        let p = Paths::at("/tmp/cf");
        assert_eq!(p.config_file(), Path::new("/tmp/cf/config.toml"));
        assert_eq!(p.journal_dir(), Path::new("/tmp/cf/journal"));
        assert_eq!(p.token_file(), Path::new("/tmp/cf/token"));
    }

    #[test]
    fn display_shortens_the_home_directory() {
        let inside = home().join("notes").join("a.txt");
        assert!(display(&inside).starts_with('~'));
        assert!(!display(Path::new("/definitely/elsewhere")).starts_with('~'));
    }

    #[cfg(windows)]
    #[test]
    fn relocation_fills_gaps_and_keeps_what_is_already_there() {
        let root = env::temp_dir().join(format!("cf-relocate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (old, new) = (root.join("old"), root.join("new"));
        fs::create_dir_all(old.join("journal")).unwrap();
        fs::write(old.join("token"), "T").unwrap();
        fs::write(old.join("config.toml"), "OLD").unwrap();
        fs::write(old.join("journal").join("a.log"), "A").unwrap();
        fs::create_dir_all(&new).unwrap();
        // Something the new home already has must not be overwritten.
        fs::write(new.join("config.toml"), "NEW").unwrap();

        copy_tree_missing(&old, &new).unwrap();

        assert_eq!(fs::read_to_string(new.join("config.toml")).unwrap(), "NEW");
        assert_eq!(fs::read_to_string(new.join("token")).unwrap(), "T");
        assert_eq!(fs::read_to_string(new.join("journal").join("a.log")).unwrap(), "A");
        let _ = fs::remove_dir_all(&root);
    }
}
