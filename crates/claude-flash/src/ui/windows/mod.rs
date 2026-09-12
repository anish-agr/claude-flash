//! The Windows front end: a hidden window that owns the tray icon and the overlays,
//! run by the main thread's message loop.

mod icons;
mod menu;
mod overlay;
mod probe;
mod sound;
mod tray;

use std::cell::RefCell;
use std::panic::{self, AssertUnwindSafe};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use flash_core::engine::FlashSpec;
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, KillTimer, MSG, PostMessageW,
    PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SetTimer, TranslateMessage, WM_APP, WM_CLOSE,
    WM_CONTEXTMENU, WM_DESTROY, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};

use super::{Native, UiEvent, UiHandle};
use crate::api::{Control, Status};
use crate::paths::Paths;
use crate::runtime::Request;
use crate::{log, system};

const WM_WAKE: u32 = WM_APP + 1;
const WM_TRAY: u32 = WM_APP + 2;
/// Tray callbacks with `NOTIFYICON_VERSION_4`: a click, and the same by keyboard.
const NIN_SELECT: u32 = 0x400;
const NIN_KEYSELECT: u32 = 0x401;
const ANIMATION_TIMER: usize = 1;
/// About one frame at 60 Hz. Windows rounds timers to its own tick in any case.
const FRAME_MS: u32 = 15;

/// Broadcast when Explorer restarts, after which the tray icon must be added again.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static RUNTIME_FAILED: AtomicBool = AtomicBool::new(false);

struct App {
    hwnd: HWND,
    instance: HINSTANCE,
    events: Receiver<UiEvent>,
    requests: Sender<Request>,
    paths: Paths,
    tray: tray::Tray,
    overlay: Option<overlay::Overlay>,
    /// The latest status and when it arrived.
    status: Option<(Box<Status>, Instant)>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the app, unless the app is already borrowed further up the stack, as
/// it can be while a modal loop dispatches messages.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| {
        let mut app = cell.try_borrow_mut().ok()?;
        app.as_mut().map(f)
    })
}

/// NUL-terminated UTF-16 that lives as long as the process, for window class names.
fn leak_wide(s: &str) -> *const u16 {
    Box::leak(system::wide(s).into_boxed_slice()).as_ptr()
}

struct Handle {
    /// The window handle as a plain number, so the handle can live on another thread.
    hwnd: isize,
    events: Sender<UiEvent>,
}

impl UiHandle for Handle {
    fn send(&self, event: UiEvent) {
        if self.events.send(event).is_ok() {
            // SAFETY: PostMessageW may be called from any thread, and fails harmlessly
            // once the window is gone.
            unsafe { PostMessageW(self.hwnd as HWND, WM_WAKE, 0, 0) };
        }
    }
}

pub fn run(native: Native) -> ExitCode {
    // SAFETY: process-wide setup with no preconditions, done before any window exists.
    let instance = unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        TASKBAR_CREATED.store(RegisterWindowMessageW(system::wide("TaskbarCreated").as_ptr()), Ordering::Relaxed);
        GetModuleHandleW(std::ptr::null())
    };
    let Some(hwnd) = create_window(instance) else {
        log!("could not create the agent window; continuing without a display");
        return super::headless::run(native);
    };

    let Native { requests, incoming, launch, paths } = native;
    let (events, receiver) = mpsc::channel();
    let handle = Box::new(Handle { hwnd: hwnd as isize, events });
    let window = hwnd as isize;
    let spawned = thread::Builder::new().name("runtime".into()).spawn(move || {
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
            launch("windows", handle, Box::new(probe::SystemProbe)).run(incoming);
        }));
        if outcome.is_err() {
            RUNTIME_FAILED.store(true, Ordering::Relaxed);
        }
        // SAFETY: as in `Handle::send`.
        unsafe { PostMessageW(window as HWND, WM_CLOSE, 0, 0) };
    });
    if let Err(e) = spawned {
        log!("could not start the runtime thread: {e}");
        return ExitCode::FAILURE;
    }

    let tray = tray::Tray::add(hwnd, WM_TRAY);
    APP.with(|cell| {
        *cell.borrow_mut() =
            Some(App { hwnd, instance, events: receiver, requests, paths, tray, overlay: None, status: None });
    });

    // SAFETY: the standard message loop, on the thread that created the windows.
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    // Dropping the app removes the tray icon and any overlay still on screen.
    APP.with(|cell| drop(cell.borrow_mut().take()));
    if RUNTIME_FAILED.load(Ordering::Relaxed) { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn create_window(instance: HINSTANCE) -> Option<HWND> {
    let class = leak_wide("ClaudeFlashAgent");
    // SAFETY: the class name lives for the whole process and `window_proc` has the
    // signature Windows expects.
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class,
            ..std::mem::zeroed()
        };
        if RegisterClassExW(&wc) == 0 {
            return None;
        }
        // A top-level window that is never shown: unlike a message-only window, it
        // receives the broadcast that tells it Explorer has restarted.
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class,
            class,
            WS_POPUP,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        (!hwnd.is_null()).then_some(hwnd)
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_WAKE => {
            with_app(App::drain);
            0
        }
        WM_TIMER if wparam == ANIMATION_TIMER => {
            with_app(App::animate);
            0
        }
        WM_TRAY => {
            // The low word of lParam is the event; the high word is the icon's id.
            if matches!((lparam as u32) & 0xFFFF, NIN_SELECT | NIN_KEYSELECT | WM_CONTEXTMENU) {
                menu::show(hwnd);
            }
            0
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            0
        }
        m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            with_app(|app| app.tray.restore());
            0
        }
        // SAFETY: default handling, with the arguments Windows passed in.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

impl App {
    fn drain(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                UiEvent::Flash(spec) => self.flash(&spec),
                UiEvent::Notify(notice) => self.tray.notify(&notice),
                UiEvent::Status(status) => {
                    self.tray.show_status(&status);
                    self.status = Some((status, Instant::now()));
                }
                UiEvent::Quit => {
                    // SAFETY: destroying our own window; WM_DESTROY ends the loop.
                    unsafe { DestroyWindow(self.hwnd) };
                    return;
                }
            }
        }
    }

    fn flash(&mut self, spec: &FlashSpec) {
        self.overlay = None;
        self.overlay = overlay::Overlay::show(spec, self.instance);
        if self.overlay.is_some() {
            // SAFETY: a timer on our own window, replaced if one is already running.
            unsafe { SetTimer(self.hwnd, ANIMATION_TIMER, FRAME_MS, None) };
        } else {
            log!("could not show a {} flash: no overlay window could be created", spec.kind);
        }
    }

    fn animate(&mut self) {
        if !self.overlay.as_mut().is_some_and(overlay::Overlay::frame) {
            self.overlay = None;
            // SAFETY: stops the timer started in `flash`.
            unsafe { KillTimer(self.hwnd, ANIMATION_TIMER) };
        }
    }

    fn send(&self, control: Control) {
        let _ = self.requests.send(Request::Control(control, None));
    }
}
