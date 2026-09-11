//! Small files shared by the agent and the CLI: the persisted switches and the API
//! token.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::system;

/// What survives an agent restart.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub enabled: bool,
    /// Unix milliseconds at which a pause ends.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused_until: Option<u64>,
    /// Sessions the user has typed into, with when they last did.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub interactive: BTreeMap<String, u64>,
}

impl Default for State {
    fn default() -> Self {
        State { enabled: true, paused_until: None, interactive: BTreeMap::new() }
    }
}

impl State {
    /// Reads the state file. A missing or unreadable file means the defaults: a
    /// damaged file must not be able to silence Claude Flash.
    pub fn load(path: &Path) -> State {
        fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut text = serde_json::to_string_pretty(self).expect("state always serialises");
        text.push('\n');
        write_atomic(path, text.as_bytes())
    }
}

/// Writes a file so that a reader sees either the old contents or the new ones,
/// never a mixture.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_replacing(path, bytes, false)
}

/// [`write_atomic`] for secrets. On Unix the file is readable only by its owner from
/// the moment it exists; on Windows the per-user profile folder already is.
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_replacing(path, bytes, true)
}

fn write_replacing(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if private { 0o600 } else { 0o644 });
    }
    #[cfg(not(unix))]
    let _ = private;
    let result = options.open(&tmp).and_then(|mut file| file.write_all(bytes)).and_then(|()| rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Renames over an existing file. Windows refuses while another process has the
/// target open, which the CLI reading the state file can briefly do, so retry.
fn rename(from: &Path, to: &Path) -> io::Result<()> {
    let mut attempts = 0;
    loop {
        match fs::rename(from, to) {
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied && attempts < 10 => {
                attempts += 1;
                thread::sleep(Duration::from_millis(15));
            }
            result => return result,
        }
    }
}

/// The bearer token that guards the agent's control API, created on first use.
pub fn load_or_create_token(path: &Path) -> io::Result<String> {
    if let Some(token) = read_token(path) {
        return Ok(token);
    }
    let token: String = system::random_bytes(32)?.iter().map(|b| format!("{b:02x}")).collect();
    write_private(path, token.as_bytes())?;
    Ok(token)
}

pub fn read_token(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let token = text.trim();
    (token.len() >= 32 && token.bytes().all(|b| b.is_ascii_hexdigit())).then(|| token.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("claude-flash-{}", std::process::id())).join("store").join(name);
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn state_round_trips_and_defaults_to_enabled() {
        let dir = scratch("state");
        let path = dir.join("state.json");
        assert_eq!(State::load(&path), State::default());
        assert!(State::default().enabled);
        let state = State { enabled: false, paused_until: Some(42), interactive: BTreeMap::from([("s".into(), 7)]) };
        state.save(&path).unwrap();
        assert_eq!(State::load(&path), state);
        fs::write(&path, "{ damaged").unwrap();
        assert!(State::load(&path).enabled, "a damaged file falls back to enabled");
    }

    #[test]
    fn token_is_created_once_and_reused() {
        let dir = scratch("token");
        let path = dir.join("token");
        let first = load_or_create_token(&path).unwrap();
        assert_eq!(first.len(), 64);
        assert_eq!(load_or_create_token(&path).unwrap(), first);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn atomic_writes_replace_existing_files() {
        let dir = scratch("atomic");
        let path = dir.join("f.txt");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1, "no temporary files are left behind");
    }
}
