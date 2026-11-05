//! Terminal colour, used only when printing to a terminal that wants it.

use std::io::IsTerminal;
use std::sync::OnceLock;

use flash_core::config;
use flash_core::event::Attention;

static COLOR: OnceLock<bool> = OnceLock::new();

pub fn init() {
    let _ = COLOR.set(wanted());
}

/// Colour unless `NO_COLOR` is set, output is redirected, or the terminal is dumb.
fn wanted() -> bool {
    if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) || !std::io::stdout().is_terminal() {
        return false;
    }
    if matches!(std::env::var("TERM").as_deref(), Ok("dumb")) {
        return false;
    }
    enable_escape_sequences()
}

#[cfg(windows)]
fn enable_escape_sequences() -> bool {
    use windows_sys::Win32::System::Console::{
        ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle, STD_OUTPUT_HANDLE, SetConsoleMode,
    };
    // SAFETY: the handle comes straight from GetStdHandle, and `mode` is a valid
    // out-pointer for GetConsoleMode.
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        GetConsoleMode(handle, &mut mode) != 0
            && (mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0
                || SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0)
    }
}

#[cfg(not(windows))]
fn enable_escape_sequences() -> bool {
    true
}

fn paint(text: &str, sgr: &str) -> String {
    if COLOR.get().copied().unwrap_or(false) { format!("\x1b[{sgr}m{text}\x1b[0m") } else { text.to_owned() }
}

pub fn bold(text: &str) -> String {
    paint(text, "1")
}

pub fn dim(text: &str) -> String {
    paint(text, "2")
}

pub fn red(text: &str) -> String {
    paint(text, "31")
}

pub fn green(text: &str) -> String {
    paint(text, "32")
}

pub fn yellow(text: &str) -> String {
    paint(text, "33")
}

/// In the signal's default flash colour.
pub fn kind(kind: Attention, text: &str) -> String {
    let c = config::builtin(kind).color;
    paint(text, &format!("38;2;{};{};{}", c.r, c.g, c.b))
}
