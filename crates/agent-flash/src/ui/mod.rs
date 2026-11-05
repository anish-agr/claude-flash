//! What the user sees: screen overlays, the tray icon or menu bar item, and
//! notifications.
//!
//! The policy engine runs on a thread of its own and hands this layer [`UiEvent`]s
//! through a [`UiHandle`]. The platform's event loop keeps the main thread, which is
//! where both Windows and macOS require windows to be created and drawn.

use std::process::ExitCode;
use std::sync::mpsc::{Receiver, Sender};

use flash_core::engine::{FlashSpec, Notice};

use crate::api::Status;
use crate::paths::Paths;
use crate::runtime::{Request, Runtime};

pub mod headless;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(windows, target_os = "macos"))]
mod present;
#[cfg(windows)]
mod windows;

pub enum UiEvent {
    Flash(FlashSpec),
    Notify(Notice),
    /// The latest status, for the tray or menu bar.
    Status(Box<Status>),
    Quit,
}

/// Delivers events to the front end's thread.
pub trait UiHandle: Send {
    fn send(&self, event: UiEvent);
}

/// System state the policy engine consults, safe to query from any thread.
pub trait Probe: Send {
    /// Milliseconds since the last keyboard or mouse input, when the system says.
    fn idle_ms(&self) -> Option<u64>;
    /// Name of the application that owns the focused window.
    fn focused_app(&self) -> Option<String>;
}

/// Builds the runtime once the front end exists to receive what it produces.
pub type Launch = Box<dyn FnOnce(&'static str, Box<dyn UiHandle>, Box<dyn Probe>) -> Runtime + Send>;

pub struct Native {
    /// The channel the HTTP server feeds, for actions chosen from a menu.
    pub requests: Sender<Request>,
    /// Handed on to the runtime thread.
    pub incoming: Receiver<Request>,
    pub launch: Launch,
    pub paths: Paths,
}

/// Runs the platform's front end on this thread until the agent quits.
pub fn run_native(native: Native) -> ExitCode {
    #[cfg(windows)]
    let run = windows::run;
    #[cfg(target_os = "macos")]
    let run = macos::run;
    #[cfg(not(any(windows, target_os = "macos")))]
    let run = headless::run;
    run(native)
}
