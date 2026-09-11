//! The notification-area icon. Its colour shows the state at a glance, and it
//! delivers the notifications sent while the user is away.

use flash_core::color::Rgb;
use flash_core::config;
use flash_core::engine::Notice;
use flash_core::event::Attention;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_LARGE_ICON, NIIF_NOSOUND, NIIF_USER, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFY_ICON_DATA_FLAGS, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXICON, SM_CXSMICON};

use super::icons::IconCache;
use crate::api::Status;
use crate::ui::present;

const ID: u32 = 1;

pub struct Tray {
    hwnd: HWND,
    callback: u32,
    icons: IconCache,
    color: Rgb,
    tip: String,
}

impl Tray {
    pub fn add(hwnd: HWND, callback: u32) -> Tray {
        let mut tray = Tray {
            hwnd,
            callback,
            icons: IconCache::default(),
            color: config::builtin(Attention::Done).color,
            tip: "Claude Flash".to_owned(),
        };
        tray.register();
        tray
    }

    fn register(&mut self) {
        let mut data = self.data(NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP);
        // SAFETY: `data` is fully initialised and carries its own size.
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &data);
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            Shell_NotifyIconW(NIM_SETVERSION, &data);
        }
    }

    /// Adds the icon again after Explorer restarts and forgets it.
    pub fn restore(&mut self) {
        self.register();
    }

    pub fn show_status(&mut self, status: &Status) {
        let (color, tip) = present::indicator(status);
        if color == self.color && tip == self.tip {
            return;
        }
        self.color = color;
        self.tip = tip;
        let data = self.data(NIF_ICON | NIF_TIP | NIF_SHOWTIP);
        // SAFETY: as in `register`.
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
    }

    /// A silent notification with the signal's sphere as its picture.
    pub fn notify(&mut self, notice: &Notice) {
        let mut data = self.data(NIF_INFO);
        copy(&mut data.szInfoTitle, &notice.title);
        copy(&mut data.szInfo, &notice.body);
        data.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON | NIIF_NOSOUND;
        // SAFETY: GetSystemMetrics has no preconditions.
        let size = unsafe { GetSystemMetrics(SM_CXICON) };
        data.hBalloonIcon = self.icons.get(config::builtin(notice.kind).color, size);
        // SAFETY: as in `register`.
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
    }

    fn data(&mut self, flags: NOTIFY_ICON_DATA_FLAGS) -> NOTIFYICONDATAW {
        // SAFETY: an all-zero NOTIFYICONDATAW is a valid starting point; the fields
        // that matter are set below.
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.hwnd;
        data.uID = ID;
        data.uFlags = flags;
        data.uCallbackMessage = self.callback;
        // SAFETY: GetSystemMetrics has no preconditions.
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) };
        data.hIcon = self.icons.get(self.color, size);
        copy(&mut data.szTip, &self.tip);
        data
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let data = self.data(0);
        // SAFETY: as in `register`.
        unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
    }
}

/// Copies `text` into a fixed UTF-16 buffer, truncating and terminating it.
fn copy(dest: &mut [u16], text: &str) {
    let (mut len, room) = (0, dest.len().saturating_sub(1));
    for (slot, unit) in dest.iter_mut().zip(text.encode_utf16().take(room)) {
        *slot = unit;
        len += 1;
    }
    if let Some(end) = dest.get_mut(len) {
        *end = 0;
    }
}
