//! The optional system sound that can accompany a flash.

use flash_core::event::Attention;
use objc2_app_kit::NSSound;
use objc2_foundation::NSString;

/// Plays one of the sounds macOS keeps in /System/Library/Sounds, chosen to suit
/// the signal. Playback is asynchronous, and a sound the system does not have is
/// simply not played.
pub fn play(kind: Attention) {
    let name = match kind {
        Attention::Done => "Glass",
        Attention::Question => "Ping",
        Attention::Approval => "Purr",
        Attention::Error => "Basso",
    };
    if let Some(sound) = NSSound::soundNamed(&NSString::from_str(name)) {
        sound.play();
    }
}
