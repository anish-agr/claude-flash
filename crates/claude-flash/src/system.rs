//! Operating-system services that have nothing to do with drawing.

use std::io;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Cryptographically secure random bytes from the operating system.
pub fn random_bytes(len: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; len];
    fill_random(&mut buf)?;
    Ok(buf)
}

#[cfg(windows)]
fn fill_random(buf: &mut [u8]) -> io::Result<()> {
    use windows_sys::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};
    let len = u32::try_from(buf.len()).map_err(|_| io::Error::other("random request too large"))?;
    // SAFETY: `buf` is valid for `len` bytes, and the system-preferred RNG flag is
    // documented to take a null algorithm handle.
    let status = unsafe { BCryptGenRandom(std::ptr::null_mut(), buf.as_mut_ptr(), len, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    if status == 0 { Ok(()) } else { Err(io::Error::other(format!("BCryptGenRandom failed with {status:#x}"))) }
}

#[cfg(unix)]
fn fill_random(buf: &mut [u8]) -> io::Result<()> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")?.read_exact(buf)
}

/// The local offset from UTC at this moment, in minutes.
#[cfg(windows)]
pub fn utc_offset_minutes() -> i32 {
    use windows_sys::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    // SAFETY: the structure is plain data that the call fills in.
    let (id, tz) = unsafe {
        let mut tz: TIME_ZONE_INFORMATION = std::mem::zeroed();
        (GetTimeZoneInformation(&mut tz), tz)
    };
    // UTC = local time + bias, with the daylight or standard bias on top.
    match id {
        1 => -(tz.Bias + tz.StandardBias),
        2 => -(tz.Bias + tz.DaylightBias),
        0 => -tz.Bias,
        _ => 0,
    }
}

#[cfg(unix)]
pub fn utc_offset_minutes() -> i32 {
    // SAFETY: `localtime_r` writes only into the `tm` it is given.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() { 0 } else { (tm.tm_gmtoff / 60) as i32 }
    }
}

/// Opens a folder, or a file in its default application.
pub fn open(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        if shell_open(path) { Ok(()) } else { Err(io::Error::other(format!("Windows could not open {}", path.display()))) }
    }
    #[cfg(target_os = "macos")]
    {
        reap(Command::new("open").arg(path).stdin(Stdio::null()).stdout(Stdio::null()).spawn()?);
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        reap(Command::new("xdg-open").arg(path).stdin(Stdio::null()).stdout(Stdio::null()).spawn()?);
        Ok(())
    }
}

/// Opens a text file for editing: `$VISUAL` or `$EDITOR` from a terminal, otherwise
/// the system's text editor.
pub fn edit(path: &Path, wait: bool) -> io::Result<()> {
    let editor = std::env::var("VISUAL").ok().or_else(|| std::env::var("EDITOR").ok()).filter(|e| !e.trim().is_empty());
    if let Some(editor) = editor {
        let mut parts = editor.split_whitespace();
        let program = parts.next().expect("filtered to non-empty");
        let mut child = Command::new(program).args(parts).arg(path).spawn()?;
        if wait {
            child.wait()?;
        } else {
            reap(child);
        }
        return Ok(());
    }
    #[cfg(windows)]
    {
        if shell_open(path) {
            return Ok(());
        }
        reap(Command::new("notepad.exe").arg(path).spawn()?);
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        reap(Command::new("open").arg("-t").arg(path).spawn()?);
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        open(path)
    }
}

#[cfg(windows)]
fn shell_open(path: &Path) -> bool {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let file = wide(path.as_os_str());
    let verb = wide("open");
    // SAFETY: both strings are NUL-terminated UTF-16 and outlive the call.
    let result = unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)
    };
    // Values above 32 mean success; anything else is an error code, including "no
    // application is associated with this file type".
    result as usize > 32
}

/// NUL-terminated UTF-16, for Windows APIs.
#[cfg(windows)]
pub fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

/// Waits for a child on a background thread, so it does not linger as a zombie.
pub fn reap(mut child: Child) {
    let _ = thread::Builder::new().name("reap".into()).spawn(move || child.wait());
}

/// Starts a program that outlives this process: no console, no inherited handles,
/// its own process group.
///
/// Claude Code waits for a hook's output pipes to close, not just for the hook
/// process to exit. A child that inherited those pipes would hold the hook open for
/// as long as it ran, which for the agent is forever.
pub fn spawn_detached(program: &Path, args: &[&str]) -> io::Result<()> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        disinherit_standard_handles();
        // Leave the hook runner's job object when it allows that, so the agent is not
        // killed with it; otherwise start inside it.
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
        if let Ok(child) = command.spawn() {
            drop(child);
            return Ok(());
        }
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command.spawn()?;
    reap(child);
    Ok(())
}

/// Windows creates child processes with every inheritable handle, and the standard
/// handles a hook runner gives us are inheritable. Marking them otherwise keeps
/// them out of anything we start.
#[cfg(windows)]
fn disinherit_standard_handles() {
    use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: GetStdHandle has no preconditions; SetHandleInformation fails
        // harmlessly on a null or invalid handle.
        unsafe {
            let handle = GetStdHandle(which);
            if !handle.is_null() && handle as isize != -1 {
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_bytes_are_random_enough_to_differ() {
        let (a, b) = (random_bytes(32).unwrap(), random_bytes(32).unwrap());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }

    #[test]
    fn utc_offset_is_a_real_offset() {
        assert!((-14 * 60..=14 * 60).contains(&utc_offset_minutes()));
    }
}
