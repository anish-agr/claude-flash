//! A front end without a display, for Linux and for tests. Flashes are written to
//! the agent log; on a Linux desktop, notifications also go through `notify-send`.

use std::process::ExitCode;

use flash_core::engine::Notice;

use super::{Native, Probe, UiEvent, UiHandle};
use crate::log;

pub struct LogUi;

impl UiHandle for LogUi {
    fn send(&self, event: UiEvent) {
        match event {
            UiEvent::Flash(spec) => {
                log!("flash {} {} at {:.0}%", spec.kind, spec.color.to_hex(), spec.opacity * 100.0);
            }
            UiEvent::Notify(notice) => {
                log!("notify {}: {}", notice.title, notice.body);
                desktop_notification(&notice);
            }
            UiEvent::Status(_) | UiEvent::Quit => {}
        }
    }
}

/// Reports nothing, so the engine never considers the user away.
pub struct NoProbe;

impl Probe for NoProbe {
    fn idle_ms(&self) -> Option<u64> {
        None
    }

    fn focused_app(&self) -> Option<String> {
        None
    }
}

pub fn run(native: Native) -> ExitCode {
    let runtime = (native.launch)("headless", Box::new(LogUi), Box::new(NoProbe));
    runtime.run(native.incoming);
    ExitCode::SUCCESS
}

#[cfg(all(unix, not(target_os = "macos")))]
fn desktop_notification(notice: &Notice) {
    use std::process::{Command, Stdio};
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return;
    }
    let child = Command::new("notify-send")
        .args(["--app-name=Claude Flash", &notice.title, &notice.body])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(child) = child {
        crate::system::reap(child);
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn desktop_notification(_notice: &Notice) {}
