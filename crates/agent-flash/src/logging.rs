//! The agent's own log: startup, configuration problems and failures worth
//! diagnosing later. Kept small by rotating once it passes a megabyte.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use crate::system;

const MAX_BYTES: u64 = 1024 * 1024;

struct Sink {
    file: Option<File>,
    /// Also write to stderr, for an agent running in a terminal.
    echo: bool,
}

static SINK: Mutex<Option<Sink>> = Mutex::new(None);

pub fn init(path: &Path, echo: bool) {
    if fs::metadata(path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = fs::rename(path, path.with_extension("log.old"));
    }
    let file = OpenOptions::new().create(true).append(true).open(path).ok();
    *SINK.lock().unwrap_or_else(|e| e.into_inner()) = Some(Sink { file, echo });
}

pub fn write(args: fmt::Arguments<'_>) {
    let line = format!("{} {args}\n", flash_core::time::rfc3339(system::unix_ms()));
    let mut sink = SINK.lock().unwrap_or_else(|e| e.into_inner());
    match sink.as_mut() {
        Some(sink) => {
            if let Some(file) = sink.file.as_mut() {
                let _ = file.write_all(line.as_bytes());
            }
            if sink.echo {
                let _ = std::io::stderr().write_all(line.as_bytes());
            }
        }
        None => {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
    }
}

/// Writes one timestamped line to the agent log.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::logging::write(format_args!($($arg)*))
    };
}
