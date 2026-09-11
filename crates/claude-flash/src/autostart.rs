//! Starting the agent at login: a `Run` registry value on Windows, a LaunchAgent on
//! macOS, and an XDG autostart entry elsewhere.

use std::io;
use std::path::{Path, PathBuf};

/// The LaunchAgent's label on macOS.
pub const LABEL: &str = "io.github.anish-agr.claude-flash";

/// The program registered to start at login, if there is one.
pub fn registered() -> Option<PathBuf> {
    platform::registered()
}

pub fn enable(agent: &Path) -> io::Result<()> {
    platform::enable(agent)
}

pub fn disable() -> io::Result<()> {
    platform::disable()
}

#[cfg(windows)]
mod platform {
    use std::io;
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SAM_FLAGS, REG_SZ, RegCloseKey, RegDeleteValueW,
        RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    };

    use crate::system::wide;

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE: &str = "Claude Flash";

    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the key came from RegOpenKeyExW and is closed exactly once.
            unsafe { RegCloseKey(self.0) };
        }
    }

    fn check(status: u32) -> io::Result<()> {
        if status == ERROR_SUCCESS { Ok(()) } else { Err(io::Error::from_raw_os_error(status as i32)) }
    }

    fn open(access: REG_SAM_FLAGS) -> io::Result<Key> {
        let subkey = wide(RUN);
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: `subkey` is NUL-terminated UTF-16 and `key` is a valid out-pointer.
        check(unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, access, &mut key) })?;
        Ok(Key(key))
    }

    pub fn registered() -> Option<PathBuf> {
        let key = open(KEY_QUERY_VALUE).ok()?;
        let name = wide(VALUE);
        let (mut kind, mut size) = (0u32, 0u32);
        // SAFETY: with a null data pointer the call reports only the type and size.
        let status = unsafe {
            RegQueryValueExW(key.0, name.as_ptr(), std::ptr::null(), &mut kind, std::ptr::null_mut(), &mut size)
        };
        if check(status).is_err() || kind != REG_SZ {
            return None;
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        // SAFETY: `buf` has room for `size` bytes.
        let status = unsafe {
            RegQueryValueExW(key.0, name.as_ptr(), std::ptr::null(), &mut kind, buf.as_mut_ptr().cast(), &mut size)
        };
        check(status).ok()?;
        let command = String::from_utf16_lossy(&buf);
        Some(PathBuf::from(program_of(command.trim_end_matches('\0'))))
    }

    pub fn enable(agent: &Path) -> io::Result<()> {
        let key = open(KEY_SET_VALUE)?;
        let name = wide(VALUE);
        let command = wide(format!("\"{}\"", agent.display()));
        let bytes = u32::try_from(command.len() * 2).map_err(|_| io::Error::other("path too long"))?;
        // SAFETY: `command` is `bytes` bytes of NUL-terminated UTF-16.
        check(unsafe { RegSetValueExW(key.0, name.as_ptr(), 0, REG_SZ, command.as_ptr().cast(), bytes) })
    }

    pub fn disable() -> io::Result<()> {
        let key = open(KEY_SET_VALUE)?;
        let name = wide(VALUE);
        // SAFETY: `name` is NUL-terminated UTF-16.
        match unsafe { RegDeleteValueW(key.0, name.as_ptr()) } {
            ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
            status => check(status),
        }
    }

    /// The program in a `Run` command line, without its quotes or arguments.
    fn program_of(command: &str) -> &str {
        let command = command.trim();
        match command.strip_prefix('"') {
            Some(rest) => rest.split('"').next().unwrap_or(rest),
            None => command.split(' ').next().unwrap_or(command),
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::LABEL;
    use crate::{paths, store};

    fn plist() -> PathBuf {
        paths::home().join("Library").join("LaunchAgents").join(format!("{LABEL}.plist"))
    }

    fn service() -> String {
        // SAFETY: getuid has no failure mode.
        format!("gui/{}", unsafe { libc::getuid() })
    }

    pub fn registered() -> Option<PathBuf> {
        let text = fs::read_to_string(plist()).ok()?;
        let arguments = text.split("<key>ProgramArguments</key>").nth(1)?;
        let start = arguments.find("<string>")? + "<string>".len();
        let end = start + arguments[start..].find("</string>")?;
        Some(PathBuf::from(unescape(&arguments[start..end])))
    }

    pub fn enable(agent: &Path) -> io::Result<()> {
        let path = plist();
        store::write_atomic(&path, launch_agent(agent).as_bytes())?;
        // Replace any loaded copy, so a moved or upgraded agent takes over now.
        let _ = Command::new("launchctl").args(["bootout", &format!("{}/{LABEL}", service())]).output();
        let output = Command::new("launchctl").arg("bootstrap").arg(service()).arg(&path).output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("launchctl bootstrap: {}", String::from_utf8_lossy(&output.stderr).trim())))
        }
    }

    pub fn disable() -> io::Result<()> {
        let _ = Command::new("launchctl").args(["bootout", &format!("{}/{LABEL}", service())]).output();
        match fs::remove_file(plist()) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Starts at login and again after a crash, but not after the user quits it.
    fn launch_agent(agent: &Path) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{}</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<dict>
		<key>SuccessfulExit</key>
		<false/>
	</dict>
	<key>ProcessType</key>
	<string>Interactive</string>
	<key>LimitLoadToSessionType</key>
	<string>Aqua</string>
</dict>
</plist>
"#,
            escape(&agent.display().to_string())
        )
    }

    fn escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    fn unescape(s: &str) -> String {
        s.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    use crate::{paths, store};

    fn entry() -> PathBuf {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| paths::home().join(".config"))
            .join("autostart")
            .join("claude-flash.desktop")
    }

    pub fn registered() -> Option<PathBuf> {
        let text = fs::read_to_string(entry()).ok()?;
        let exec = text.lines().find_map(|line| line.strip_prefix("Exec="))?;
        Some(PathBuf::from(program_of(exec)))
    }

    pub fn enable(agent: &Path) -> io::Result<()> {
        let text = format!(
            "[Desktop Entry]\nType=Application\nName=Claude Flash\nComment=Attention signals for Claude Code\nExec={}\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
            exec_argument(&agent.display().to_string())
        );
        store::write_atomic(&entry(), text.as_bytes())
    }

    pub fn disable() -> io::Result<()> {
        match fs::remove_file(entry()) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// A path as one argument of a desktop entry's `Exec` key. Inside the quotes,
    /// `"`, `` ` ``, `$` and `\` take a backslash and `%` is doubled; the general
    /// escaping of string values then doubles every backslash.
    fn exec_argument(path: &str) -> String {
        let mut quoted = String::from("\"");
        for c in path.chars() {
            match c {
                '"' | '`' | '$' | '\\' => {
                    quoted.push('\\');
                    quoted.push(c);
                }
                '%' => quoted.push_str("%%"),
                c => quoted.push(c),
            }
        }
        quoted.push('"');
        quoted.replace('\\', "\\\\")
    }

    /// Reverses [`exec_argument`].
    fn program_of(exec: &str) -> String {
        let unescaped = exec.trim().replace("\\\\", "\\");
        let inner = unescaped.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(&unescaped);
        let mut program = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => program.extend(chars.next()),
                '%' => {
                    chars.next();
                    program.push('%');
                }
                c => program.push(c),
            }
        }
        program
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn exec_paths_survive_quoting() {
            for path in ["/usr/local/bin/flash-agent", "/home/a b/it's $HOME/\"x\"/back\\slash/100%"] {
                assert_eq!(program_of(&exec_argument(path)), path);
            }
            assert_eq!(exec_argument("/opt/a$b"), "\"/opt/a\\\\$b\"");
        }
    }
}
