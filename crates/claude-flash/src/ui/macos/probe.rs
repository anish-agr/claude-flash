//! Idle time, the frontmost application and the reduced-motion preference.

use objc2::rc::autoreleasepool;
use objc2_app_kit::NSWorkspace;
use objc2_core_graphics::{CGEventSource, CGEventSourceStateID, CGEventType};

use crate::ui::Probe;

/// `kCGAnyInputEventType`: every kind of keyboard, mouse and trackpad input.
const ANY_INPUT: CGEventType = CGEventType(u32::MAX);

/// Safe to call from the runtime thread: nothing here touches AppKit's windows.
pub struct SystemProbe;

impl Probe for SystemProbe {
    fn idle_ms(&self) -> Option<u64> {
        let seconds =
            CGEventSource::seconds_since_last_event_type(CGEventSourceStateID::CombinedSessionState, ANY_INPUT);
        (seconds.is_finite() && seconds >= 0.0).then_some((seconds * 1000.0) as u64)
    }

    fn focused_app(&self) -> Option<String> {
        autoreleasepool(|_| {
            let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
            Some(app.localizedName()?.to_string())
        })
    }
}

/// Whether "Reduce motion" is on in Accessibility settings.
pub fn reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}
