//! Tray and notification icons: the Claude Flash sphere, drawn in a state's colour.

use std::collections::HashMap;

use flash_core::color::Rgb;
use flash_core::render::{self, ByteOrder};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, GetDC,
    ReleaseDC,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyIcon, HICON, ICONINFO};

/// Icons by colour and size, kept for the life of the tray.
#[derive(Default)]
pub struct IconCache {
    icons: HashMap<(u8, u8, u8, i32), HICON>,
}

impl IconCache {
    pub fn get(&mut self, color: Rgb, size: i32) -> HICON {
        *self.icons.entry((color.r, color.g, color.b, size)).or_insert_with(|| sphere(color, size))
    }
}

impl Drop for IconCache {
    fn drop(&mut self) {
        for icon in self.icons.values().filter(|icon| !icon.is_null()) {
            // SAFETY: each icon came from CreateIconIndirect and is destroyed once.
            unsafe { DestroyIcon(*icon) };
        }
    }
}

fn sphere(color: Rgb, size: i32) -> HICON {
    let size = size.clamp(16, 256);
    let side = size as usize;
    // SAFETY: the DIB's memory is exactly side * side * 4 bytes; the bitmaps are
    // deleted once CreateIconIndirect has copied them.
    unsafe {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            },
            ..std::mem::zeroed()
        };
        let mut bits = std::ptr::null_mut();
        let screen = GetDC(std::ptr::null_mut());
        let color_bitmap = CreateDIBSection(screen, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        ReleaseDC(std::ptr::null_mut(), screen);
        if color_bitmap.is_null() || bits.is_null() {
            return std::ptr::null_mut();
        }
        let pixels = std::slice::from_raw_parts_mut(bits.cast::<u8>(), side * side * 4);
        render::sphere(pixels, side, color, ByteOrder::Bgra);
        // An all-zero mask: transparency comes from the colour bitmap's alpha.
        let mask_bits = vec![0u8; side.div_ceil(16) * 2 * side];
        let mask = CreateBitmap(size, size, 1, 1, mask_bits.as_ptr().cast());
        let icon =
            CreateIconIndirect(&ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color_bitmap });
        DeleteObject(mask);
        DeleteObject(color_bitmap);
        icon
    }
}
