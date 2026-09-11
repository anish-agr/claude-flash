//! Where Claude Flash keeps its files.

use std::env;
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
    let local = env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData").join("Local"));
    // The same folder v1 used, so an upgrade finds its settings where it left them.
    Paths::at(local.join("ClaudeFlash"))
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
}
