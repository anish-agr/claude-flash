//! The tray icon's menu.

use std::fs;
use std::path::Path;

use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, HMENU, IDOK, MB_ICONINFORMATION, MB_OKCANCEL,
    MB_SETFOREGROUND, MENU_ITEM_FLAGS, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MessageBoxW,
    PostMessageW, SetForegroundWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL,
};

use super::{App, with_app};
use crate::api::Control;
use crate::ui::present::{self, KINDS, PAUSES};
use crate::{autostart, log, system};

const TOGGLE: usize = 1;
const PAUSE: usize = 10;
const RESUME: usize = 19;
const TEST: usize = 20;
const AUTOSTART: usize = 30;
const SETTINGS: usize = 31;
const JOURNAL: usize = 32;
const QUIT: usize = 40;

pub fn show(hwnd: HWND) {
    let Some(menu) = with_app(build) else { return };
    // SAFETY: the usual tray menu sequence on our own window. With TPM_RETURNCMD the
    // chosen command comes back as the return value; the menu is destroyed after.
    let command = unsafe {
        let mut point = POINT { x: 0, y: 0 };
        GetCursorPos(&mut point);
        // Without this the menu would stay open when the user clicks elsewhere.
        SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            point.x,
            point.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        DestroyMenu(menu);
        PostMessageW(hwnd, WM_NULL, 0, 0);
        command
    };
    if command > 0 {
        choose(hwnd, command as usize);
    }
}

fn build(app: &mut App) -> HMENU {
    let status = app.status.as_ref().map(|(status, at)| (status.as_ref(), at.elapsed().as_millis() as u64));
    let elapsed = status.map_or(0, |(_, elapsed)| elapsed);
    // SAFETY: AppendMenuW copies each label, and a submenu is owned by its parent
    // once appended, so destroying the top menu frees everything.
    unsafe {
        let menu = CreatePopupMenu();
        append(menu, MF_STRING | MF_GRAYED, 0, &present::headline(status.map(|(s, _)| s), elapsed));
        if let Some((status, _)) = status {
            for wait in status.waiting.iter().take(6) {
                append(menu, MF_STRING | MF_GRAYED, 0, &present::waiting(wait, elapsed));
            }
        }
        append(menu, MF_SEPARATOR, 0, "");
        append(menu, checked(status.is_none_or(|(s, _)| s.enabled)), TOGGLE, "Flashes on");

        let pause = CreatePopupMenu();
        for (i, (_, label)) in PAUSES.iter().enumerate() {
            append(pause, MF_STRING, PAUSE + i, label);
        }
        if status.is_some_and(|(s, _)| s.paused_for_ms.is_some()) {
            append(pause, MF_SEPARATOR, 0, "");
            append(pause, MF_STRING, RESUME, "Resume now");
        }
        append(menu, MF_POPUP, pause as usize, "Pause");

        let test = CreatePopupMenu();
        for (i, kind) in KINDS.iter().enumerate() {
            append(test, MF_STRING, TEST + i, present::title(*kind));
        }
        append(menu, MF_POPUP, test as usize, "Test flash");
        append(menu, MF_SEPARATOR, 0, "");
        append(menu, checked(autostart::registered().is_some()), AUTOSTART, "Start at login");
        append(menu, MF_STRING, SETTINGS, "Open settings");
        append(menu, MF_STRING, JOURNAL, "Open journal folder");
        append(menu, MF_SEPARATOR, 0, "");
        append(menu, MF_STRING, QUIT, "Quit");
        menu
    }
}

/// # Safety
/// `menu` must be a live menu handle.
unsafe fn append(menu: HMENU, flags: MENU_ITEM_FLAGS, id: usize, label: &str) {
    // A single `&` would underline the next letter as a keyboard shortcut.
    let label = system::wide(label.replace('&', "&&"));
    // SAFETY: the caller guarantees `menu`; AppendMenuW copies the label.
    unsafe { AppendMenuW(menu, flags, id, label.as_ptr()) };
}

fn checked(on: bool) -> MENU_ITEM_FLAGS {
    if on { MF_STRING | MF_CHECKED } else { MF_STRING }
}

fn choose(hwnd: HWND, command: usize) {
    let control = match command {
        TOGGLE => Some(Control::Toggle { confirm: false }),
        RESUME => Some(Control::Resume),
        QUIT => confirm_quit(hwnd).then_some(Control::Quit),
        id if (PAUSE..PAUSE + PAUSES.len()).contains(&id) => {
            Some(Control::Pause { duration: PAUSES[id - PAUSE].0.to_owned() })
        }
        id if (TEST..TEST + KINDS.len()).contains(&id) => Some(Control::Test { kind: KINDS[id - TEST] }),
        _ => None,
    };
    if let Some(control) = control {
        with_app(|app| app.send(control));
        return;
    }
    match command {
        AUTOSTART => toggle_autostart(),
        SETTINGS => {
            if let Some(path) = with_app(|app| app.paths.config_file()) {
                open(&path, true);
            }
        }
        JOURNAL => {
            if let Some(path) = with_app(|app| app.paths.journal_dir()) {
                open(&path, false);
            }
        }
        _ => {}
    }
}

fn confirm_quit(hwnd: HWND) -> bool {
    let text = system::wide(format!("Quit Claude Flash?\n\n{}", present::QUIT_DETAIL));
    let caption = system::wide("Claude Flash");
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe {
        MessageBoxW(hwnd, text.as_ptr(), caption.as_ptr(), MB_OKCANCEL | MB_ICONINFORMATION | MB_SETFOREGROUND) == IDOK
    }
}

fn toggle_autostart() {
    let result = match autostart::registered() {
        Some(_) => autostart::disable(),
        None => std::env::current_exe().and_then(|exe| autostart::enable(&exe)),
    };
    if let Err(e) = result {
        log!("could not change whether the agent starts at login: {e}");
    }
}

fn open(path: &Path, text: bool) {
    let result =
        if text { system::open_text(path) } else { fs::create_dir_all(path).and_then(|()| system::open(path)) };
    if let Err(e) = result {
        log!("could not open {}: {e}", path.display());
    }
}
