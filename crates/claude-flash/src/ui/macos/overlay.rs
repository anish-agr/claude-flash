//! The flash on macOS: one borderless, click-through window per display, above
//! everything including full-screen apps and the menu bar, faded through the
//! window's alpha.

use std::time::Instant;

use flash_core::color::Rgb;
use flash_core::engine::FlashSpec;
use flash_core::render::{self, ByteOrder, Envelope};
use objc2::rc::Retained;
use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSImage, NSImageScaling, NSImageView, NSScreen, NSScreenSaverWindowLevel, NSView,
    NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::{CFData, CFRetained};
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGEventSource, CGEventSourceStateID,
    CGEventType, CGImage, CGImageAlphaInfo,
};
use objc2_foundation::NSSize;

use super::probe;

/// Width of the painted wash. It is a smooth gradient, so the window server scaling
/// a small image up looks the same as painting every pixel of a Retina display.
const WASH_WIDTH: usize = 480;

pub struct Overlay {
    windows: Vec<Retained<NSWindow>>,
    envelope: Envelope,
    started: Instant,
}

impl Overlay {
    pub fn show(mtm: MainThreadMarker, spec: &FlashSpec) -> Option<Overlay> {
        let windows: Vec<Retained<NSWindow>> =
            NSScreen::screens(mtm).iter().filter_map(|screen| window_for(mtm, &screen, spec)).collect();
        if windows.is_empty() {
            return None;
        }
        let reduce_motion = spec.respect_reduce_motion && probe::reduce_motion();
        for window in &windows {
            window.setAlphaValue(0.0);
            window.orderFrontRegardless();
        }
        Some(Overlay {
            windows,
            envelope: Envelope::new(spec.timing, spec.opacity, reduce_motion),
            started: Instant::now(),
        })
    }

    /// Draws the next frame. Returns false once the flash has finished.
    pub fn frame(&mut self) -> bool {
        let elapsed = self.started.elapsed();
        let t = elapsed.as_millis() as u64;
        if user_acted_within(elapsed.as_secs_f64()) {
            self.envelope.dismiss(t);
        }
        match self.envelope.alpha(t) {
            Some(alpha) => {
                for window in &self.windows {
                    window.setAlphaValue(f64::from(alpha));
                }
                true
            }
            None => false,
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        for window in &self.windows {
            window.orderOut(None);
            window.close();
        }
    }
}

fn window_for(mtm: MainThreadMarker, screen: &NSScreen, spec: &FlashSpec) -> Option<Retained<NSWindow>> {
    let frame = screen.frame();
    if frame.size.width < 1.0 || frame.size.height < 1.0 {
        return None;
    }
    let width = WASH_WIDTH;
    let height = (WASH_WIDTH as f64 * frame.size.height / frame.size.width).round().max(1.0) as usize;
    let mut pixels = vec![0u8; width * height * 4];
    render::paint(&mut pixels, width, height, spec.color, spec.style, spec.vignette, ByteOrder::Rgba);
    let wash = cg_image(&pixels, width, height, true)?;
    let image = NSImage::initWithCGImage_size(NSImage::alloc(), &wash, frame.size);

    // SAFETY: a plain borderless window. It is not released on close, because
    // `Retained` owns it.
    let window = unsafe {
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setReleasedWhenClosed(false);
        window
    };
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setHasShadow(false);
    window.setIgnoresMouseEvents(true);
    window.setLevel(NSScreenSaverWindowLevel);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    // While a window sits at the normal level, AppKit keeps it clear of the menu bar
    // and the Dock. Now that it is above both, the screen's own frame fits.
    window.setFrame_display(frame, false);
    let view = NSImageView::imageViewWithImage(&image, mtm);
    view.setImageScaling(NSImageScaling::ScaleAxesIndependently);
    let view: &NSView = &view;
    window.setContentView(Some(view));
    Some(window)
}

/// Whether a key was pressed or a mouse button clicked in the last `seconds`. This
/// reads when each kind of input last happened, so it needs no event tap and no
/// Input Monitoring permission, and a mouse glide does not count.
fn user_acted_within(seconds: f64) -> bool {
    [CGEventType::KeyDown, CGEventType::LeftMouseDown, CGEventType::RightMouseDown, CGEventType::OtherMouseDown]
        .into_iter()
        .any(|kind| {
            CGEventSource::seconds_since_last_event_type(CGEventSourceStateID::CombinedSessionState, kind) < seconds
        })
}

/// A CGImage over a copy of RGBA pixels.
fn cg_image(rgba: &[u8], width: usize, height: usize, premultiplied: bool) -> Option<CFRetained<CGImage>> {
    let data = CFData::from_bytes(rgba);
    let provider = CGDataProvider::with_cf_data(Some(&data))?;
    let space = CGColorSpace::new_device_rgb()?;
    let alpha = if premultiplied { CGImageAlphaInfo::PremultipliedLast } else { CGImageAlphaInfo::Last };
    // SAFETY: the provider holds exactly width * height * 4 bytes, laid out as the
    // arguments describe, and no decode array is passed.
    unsafe {
        CGImage::new(
            width,
            height,
            8,
            32,
            width * 4,
            Some(&space),
            CGBitmapInfo(alpha.0),
            Some(&provider),
            std::ptr::null(),
            true,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
}

/// The Claude Flash sphere in `color`, sized for the menu bar.
pub fn sphere_image(color: Rgb) -> Option<Retained<NSImage>> {
    const POINTS: f64 = 16.0;
    const PIXELS: usize = 32;
    let mut rgba = vec![0u8; PIXELS * PIXELS * 4];
    render::sphere(&mut rgba, PIXELS, color, ByteOrder::Rgba);
    let image = cg_image(&rgba, PIXELS, PIXELS, false)?;
    Some(NSImage::initWithCGImage_size(NSImage::alloc(), &image, NSSize::new(POINTS, POINTS)))
}
