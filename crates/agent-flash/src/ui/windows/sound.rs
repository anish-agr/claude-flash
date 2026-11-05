//! The optional system sound that can accompany a flash.

use flash_core::event::Attention;
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONASTERISK, MB_ICONEXCLAMATION, MB_ICONHAND, MB_ICONQUESTION};

/// Plays the system sound Windows maps to this kind of message, so it follows the
/// sound scheme and the volume already chosen. It returns at once; the sound plays
/// on its own.
pub fn play(kind: Attention) {
    let sound = match kind {
        Attention::Done => MB_ICONASTERISK,
        Attention::Question => MB_ICONQUESTION,
        Attention::Approval => MB_ICONEXCLAMATION,
        Attention::Error => MB_ICONHAND,
    };
    // SAFETY: a documented call taking one constant and no pointers.
    let _ = unsafe { MessageBeep(sound) };
}
