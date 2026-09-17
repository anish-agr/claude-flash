//! Starting the agent at login: a scheduled task on Windows, a LaunchAgent on
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
    //! A logon-triggered scheduled task, not a `Run` registry value. A packaged host
    //! such as the Claude desktop app copies the `HKCU` writes its child processes
    //! make into a private hive, so a `Run` value written from inside it never
    //! reaches the real profile and the agent would not start at login. The Task
    //! Scheduler keeps one store for the machine that the redirection leaves alone,
    //! so a task works wherever `flash install` runs from, and it starts on battery
    //! as well as on mains power.

    use std::io;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, RegCloseKey, RegDeleteValueW, RegOpenKeyExW,
    };

    use crate::system::wide;

    /// The task's name; it appears as `\Claude Flash` in the Task Scheduler.
    const TASK: &str = "Claude Flash";
    /// Where versions up to 2.2.0 registered the agent. `enable` clears it.
    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const RUN_VALUE: &str = "Claude Flash";
    /// No console window for the `schtasks` child.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub fn registered() -> Option<PathBuf> {
        let xml = schtasks(&["/Query", "/TN", TASK, "/XML", "ONE"]).ok()?;
        let command = between(&xml, "<Command>", "</Command>")?;
        Some(PathBuf::from(unescape(command.trim())))
    }

    pub fn enable(agent: &Path) -> io::Result<()> {
        let file = std::env::temp_dir().join(format!("claude-flash-task-{}.xml", std::process::id()));
        std::fs::write(&file, utf16_le_bom(&task_definition(agent)))?;
        let created = schtasks(&["/Create", "/TN", TASK, "/XML", file.to_string_lossy().as_ref(), "/F"]);
        let _ = std::fs::remove_file(&file);
        created?;
        // A login item an earlier version left in the registry would start a second
        // agent, so it goes now that the task has taken over.
        remove_run_value();
        Ok(())
    }

    pub fn disable() -> io::Result<()> {
        remove_run_value();
        match schtasks(&["/Delete", "/TN", TASK, "/F"]) {
            Ok(_) => Ok(()),
            // Deleting a task that was never there is not a failure.
            Err(_) if registered().is_none() => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Runs `schtasks.exe`, returning its standard output when it succeeds.
    fn schtasks(args: &[&str]) -> io::Result<String> {
        let output = Command::new("schtasks.exe").args(args).creation_flags(CREATE_NO_WINDOW).output()?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(io::Error::other(String::from_utf8_lossy(&output.stderr).trim().to_owned()))
        }
    }

    /// A task that starts the agent at logon for the current user, at normal
    /// privilege, on battery as well as on mains power.
    fn task_definition(agent: &Path) -> String {
        let user = escape(&current_user());
        let command = escape(&agent.display().to_string());
        format!(
            r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Starts the Claude Flash agent when you sign in.</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
    </Exec>
  </Actions>
</Task>
"#
        )
    }

    /// `DOMAIN\user`, or just the user name when there is no domain.
    fn current_user() -> String {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        match (var("USERDOMAIN"), var("USERNAME")) {
            (Some(domain), Some(name)) => format!("{domain}\\{name}"),
            (_, Some(name)) => name,
            (_, None) => String::new(),
        }
    }

    /// Removes the `Run` value an earlier version wrote, if it is there.
    fn remove_run_value() {
        let subkey = wide(RUN);
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: `subkey` is NUL-terminated UTF-16 and `key` is a valid out-pointer.
        if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_SET_VALUE, &mut key) } == ERROR_SUCCESS {
            let name = wide(RUN_VALUE);
            // SAFETY: `key` is open for writing and `name` is NUL-terminated UTF-16.
            unsafe {
                RegDeleteValueW(key, name.as_ptr());
                RegCloseKey(key);
            }
        }
    }

    /// The text between the first `open` and the next `close`.
    fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
        let start = text.find(open)? + open.len();
        let end = start + text[start..].find(close)?;
        Some(&text[start..end])
    }

    fn escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    fn unescape(s: &str) -> String {
        s.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
    }

    /// UTF-16 little-endian with a byte-order mark, which `schtasks /XML` expects.
    fn utf16_le_bom(s: &str) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in s.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_task_starts_the_agent_at_logon_on_battery() {
            let path = r"C:\Users\a b\AppData\Local\Microsoft\WindowsApps\flash-agent.exe";
            let xml = task_definition(Path::new(path));
            assert!(xml.contains("<LogonTrigger>"));
            assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
            assert_eq!(unescape(between(&xml, "<Command>", "</Command>").unwrap()), path);
        }

        #[test]
        fn a_path_with_xml_characters_is_escaped_then_recovered() {
            let path = r"C:\a & b\<x>\flash-agent.exe";
            let xml = task_definition(Path::new(path));
            assert!(!xml.contains(r"<x>\"), "the raw characters must not reach the XML");
            assert_eq!(unescape(between(&xml, "<Command>", "</Command>").unwrap()), path);
        }

        #[test]
        fn utf16_starts_with_a_byte_order_mark() {
            let bytes = utf16_le_bom("Ab");
            assert_eq!(bytes, vec![0xFF, 0xFE, b'A', 0, b'b', 0]);
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
