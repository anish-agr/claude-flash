//! The flash itself: a layered, click-through, topmost window on every monitor.
//!
//! Each window's pixels are painted once, with the shape in per-pixel alpha. The fade
//! then only changes the window's overall opacity through `UpdateLayeredWindow`,
//! which the compositor applies without repainting anything.

use std::cell::Cell;
use std::sync::OnceLock;
use std::time::Instant;

use flash_core::engine::FlashSpec;
use flash_core::render::{self, ByteOrder, Envelope};
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, POINT, RECT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors, GetDC, GetMonitorInfoW, HBITMAP,
    HDC, HGDIOBJ, HMONITOR, MONITORINFO, ReleaseDC, SelectObject,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetLastInputInfo, LASTINPUTINFO, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, RegisterClassExW, SW_SHOWNOACTIVATE, ShowWindow,
    ULW_ALPHA, UpdateLayeredWindow, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows_sys::core::BOOL;

use super::{leak_wide, probe};

pub struct Overlay {
    windows: Vec<Layered>,
    envelope: Envelope,
    started: Instant,
    input: InputWatch,
}

impl Overlay {
    pub fn show(spec: &FlashSpec, instance: HINSTANCE) -> Option<Overlay> {
        let class = class(instance);
        let windows: Vec<Layered> =
            monitors().iter().filter_map(|rect| Layered::create(rect, spec, class, instance)).collect();
        if windows.is_empty() {
            return None;
        }
        if spec.sound {
            super::sound::play(spec.kind);
        }
        let reduce_motion = spec.respect_reduce_motion && probe::reduce_motion();
        let overlay = Overlay {
            windows,
            envelope: Envelope::new(spec.timing, spec.opacity, reduce_motion),
            started: Instant::now(),
            input: InputWatch::start(),
        };
        overlay.apply(0.0);
        for window in &overlay.windows {
            // SAFETY: our own window; showing it without activation leaves focus alone.
            unsafe { ShowWindow(window.hwnd, SW_SHOWNOACTIVATE) };
        }
        Some(overlay)
    }

    /// Draws the next frame. Returns false once the flash has finished.
    pub fn frame(&mut self) -> bool {
        let t = self.started.elapsed().as_millis() as u64;
        if self.input.user_acted() {
            self.envelope.dismiss(t);
        }
        match self.envelope.alpha(t) {
            Some(alpha) => {
                self.apply(alpha);
                true
            }
            None => false,
        }
    }

    fn apply(&self, alpha: f32) {
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        for window in &self.windows {
            window.update(&blend);
        }
    }
}

/// One monitor's window with the bitmap that holds its pixels.
struct Layered {
    hwnd: HWND,
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    origin: POINT,
    size: SIZE,
    painted: Cell<bool>,
}

impl Layered {
    fn create(rect: &RECT, spec: &FlashSpec, class: *const u16, instance: HINSTANCE) -> Option<Layered> {
        let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
        if width <= 0 || height <= 0 {
            return None;
        }
        // SAFETY: every handle created here is checked, and released in `Drop` or on
        // the failure path; the pixel slice covers exactly the DIB's memory.
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class,
                std::ptr::null(),
                WS_POPUP,
                rect.left,
                rect.top,
                width,
                height,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return None;
            }
            let screen = GetDC(std::ptr::null_mut());
            let dc = CreateCompatibleDC(screen);
            ReleaseDC(std::ptr::null_mut(), screen);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    // Negative height: top-down rows, as `render::paint` writes them.
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..std::mem::zeroed()
                },
                ..std::mem::zeroed()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if dc.is_null() || bitmap.is_null() || bits.is_null() {
                if !bitmap.is_null() {
                    DeleteObject(bitmap);
                }
                if !dc.is_null() {
                    DeleteDC(dc);
                }
                DestroyWindow(hwnd);
                return None;
            }
            let (w, h) = (width as usize, height as usize);
            let pixels = std::slice::from_raw_parts_mut(bits.cast::<u8>(), w * h * 4);
            render::paint(pixels, w, h, spec.color, spec.style, spec.vignette, ByteOrder::Bgra);
            let previous = SelectObject(dc, bitmap);
            Some(Layered {
                hwnd,
                dc,
                bitmap,
                previous,
                origin: POINT { x: rect.left, y: rect.top },
                size: SIZE { cx: width, cy: height },
                painted: Cell::new(false),
            })
        }
    }

    fn update(&self, blend: &BLENDFUNCTION) {
        let source = POINT { x: 0, y: 0 };
        // SAFETY: all handles belong to this window. The first call hands over the
        // pixels; later calls pass no source and change only the opacity.
        unsafe {
            if self.painted.replace(true) {
                UpdateLayeredWindow(
                    self.hwnd,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    0,
                    blend,
                    ULW_ALPHA,
                );
            } else {
                UpdateLayeredWindow(
                    self.hwnd,
                    std::ptr::null_mut(),
                    &self.origin,
                    &self.size,
                    self.dc,
                    &source,
                    0,
                    blend,
                    ULW_ALPHA,
                );
            }
        }
    }
}

impl Drop for Layered {
    fn drop(&mut self) {
        // SAFETY: releases exactly what `create` acquired, once.
        unsafe {
            DestroyWindow(self.hwnd);
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

/// Registers the overlay window class on first use.
fn class(instance: HINSTANCE) -> *const u16 {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| {
        let name = leak_wide("ClaudeFlashOverlay");
        // SAFETY: the name lives for the whole process; DefWindowProcW suits windows
        // that never take input.
        unsafe {
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(DefWindowProcW),
                hInstance: instance,
                lpszClassName: name,
                ..std::mem::zeroed()
            };
            RegisterClassExW(&wc);
        }
        name as usize
    }) as *const u16
}

/// Every monitor's rectangle, in physical pixels.
fn monitors() -> Vec<RECT> {
    unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the vector passed to EnumDisplayMonitors below, alive for
        // the duration of the enumeration.
        unsafe {
            let rects = &mut *(data as *mut Vec<RECT>);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..std::mem::zeroed() };
            if GetMonitorInfoW(monitor, &mut info) != 0 {
                rects.push(info.rcMonitor);
            }
        }
        1
    }
    let mut rects: Vec<RECT> = Vec::new();
    // SAFETY: the callback only touches `rects`, which outlives the call.
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect),
            &mut rects as *mut Vec<RECT> as LPARAM,
        )
    };
    rects
}

/// Notices a key press or click after a flash starts, without hooking or polling the
/// keyboard: any input advances the system's last-input time, and input that did not
/// move the pointer, or that came with a mouse button down, was not a mouse glide.
struct InputWatch {
    last_input: u32,
    cursor: POINT,
}

impl InputWatch {
    fn start() -> InputWatch {
        InputWatch { last_input: last_input_tick(), cursor: cursor() }
    }

    fn user_acted(&mut self) -> bool {
        let tick = last_input_tick();
        if tick == self.last_input {
            return false;
        }
        self.last_input = tick;
        let now = cursor();
        let moved = now.x != self.cursor.x || now.y != self.cursor.y;
        self.cursor = now;
        // SAFETY: GetAsyncKeyState has no preconditions.
        let clicking =
            [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON].iter().any(|&vk| unsafe { GetAsyncKeyState(vk.into()) } < 0);
        clicking || !moved
    }
}

fn last_input_tick() -> u32 {
    let mut info = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    // SAFETY: `info` is initialised with its size, as the call requires.
    unsafe { GetLastInputInfo(&mut info) };
    info.dwTime
}

fn cursor() -> POINT {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: `point` is a valid out-pointer.
    unsafe { GetCursorPos(&mut point) };
    point
}
