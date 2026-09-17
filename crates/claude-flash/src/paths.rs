//! Where Claude Flash keeps its files.

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
    /// The platform's usual locations, or everything under `CLAUDE_FLASH_HOME` when
    /// that is set.
    pub fn resolve() -> Paths {
        match env::var_os("CLAUDE_FLASH_HOME").filter(|v| !v.is_empty()) {
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
    // every process sees one Claude Flash. `relocate_legacy_data` carries an older
    // install's files here.
    Paths::at(home().join(".claude-flash"))
}

/// Where Windows builds before this one kept their data.
#[cfg(windows)]
fn legacy_windows_data_dir() -> PathBuf {
    let local = env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData").join("Local"));
    local.join("ClaudeFlash")
}

#[cfg(target_os = "macos")]
fn platform_paths() -> Paths {
    Paths::at(home().join("Library").join("Application Support").join("claude-flash"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_paths() -> Paths {
    let xdg = |var: &str, fallback: &str| {
        env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(fallback))
    };
    Paths {
        config_dir: xdg("XDG_CONFIG_HOME", ".config").join("claude-flash"),
        data_dir: xdg("XDG_STATE_HOME", ".local/state").join("claude-flash"),
    }
}

pub fn home() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// Carries a Windows install's data from the old `%LOCALAPPDATA%\ClaudeFlash` folder
/// into the profile root the first time a build that uses the new location runs, so
/// nobody loses a token, settings or the journal in the move. It copies rather than
/// deletes, and only fills gaps, so running it twice is safe and the old folder stays
/// as a backup. It does nothing when a custom `CLAUDE_FLASH_HOME` is in force, when
/// the new home already holds settings, or on macOS and Linux, which are not
/// redirected.
pub fn relocate_legacy_data(active: &Paths) {
    #[cfg(windows)]
    {
        let default_home = home().join(".claude-flash");
        // A custom CLAUDE_FLASH_HOME points somewhere the user chose; leave it alone.
        if active.data_dir != default_home {
            return;
        }
        let old = legacy_windows_data_dir();
        if old == default_home || active.config_file().exists() {
            return;
        }
        // Only move a real install, not an empty leftover folder.
        if old.join("config.toml").exists() || old.join("token").exists() {
            let _ = copy_tree_missing(&old, &default_home);
        }
    }
    #[cfg(not(windows))]
    let _ = active;
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
