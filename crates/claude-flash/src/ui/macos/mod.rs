//! The macOS front end: a menu bar item and full-screen overlay windows, run by
//! AppKit on the main thread.

mod menu;
mod overlay;
mod probe;

use std::cell::RefCell;
use std::panic::{self, AssertUnwindSafe};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Instant;

use dispatch2::DispatchQueue;
use flash_core::engine::{FlashSpec, Notice};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSMenu, NSMenuDelegate, NSMenuItem};
use objc2_foundation::NSTimer;

use super::{Native, UiEvent, UiHandle};
use crate::api::{Control, Status};
use crate::paths::Paths;
use crate::runtime::Request;
use crate::{log, system};

struct App {
    mtm: MainThreadMarker,
    target: Retained<Target>,
    requests: Sender<Request>,
    paths: Paths,
    menu: menu::StatusMenu,
    overlay: Option<overlay::Overlay>,
    timer: Option<Retained<NSTimer>>,
    /// The latest status and when it arrived.
    status: Option<(Box<Status>, Instant)>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the app, unless the app is already borrowed further up the stack, as
/// it can be while a modal alert runs its own event loop.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| {
        let mut app = cell.try_borrow_mut().ok()?;
        app.as_mut().map(f)
    })
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and Target does not
    // implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ClaudeFlashTarget"]
    struct Target;

    impl Target {
        #[unsafe(method(tick:))]
        fn tick(&self, _timer: &NSTimer) {
            with_app(App::animate);
        }

        #[unsafe(method(choose:))]
        fn choose(&self, item: &NSMenuItem) {
            menu::choose(item.tag());
        }
    }

    unsafe impl NSObjectProtocol for Target {}

    unsafe impl NSMenuDelegate for Target {
        #[unsafe(method(menuNeedsUpdate:))]
        fn menu_needs_update(&self, menu: &NSMenu) {
            with_app(|app| menu::fill(app, menu));
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker) -> Retained<Target> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's designated initialiser, on a fresh allocation.
        unsafe { msg_send![super(this), init] }
    }
}

/// Hands events to the main thread through its dispatch queue.
struct Handle;

impl UiHandle for Handle {
    fn send(&self, event: UiEvent) {
        DispatchQueue::main().exec_async(move || deliver(event));
    }
}

fn deliver(event: UiEvent) {
    match event {
        UiEvent::Flash(spec) => {
            with_app(|app| app.flash(&spec));
        }
        UiEvent::Notify(notice) => notify(&notice),
        UiEvent::Status(status) => {
            with_app(|app| {
                app.menu.show_status(app.mtm, &status);
                app.status = Some((status, Instant::now()));
            });
        }
        UiEvent::Quit => {
            if let Some(mtm) = MainThreadMarker::new() {
                NSApplication::sharedApplication(mtm).terminate(None);
            }
        }
    }
}

pub fn run(native: Native) -> ExitCode {
    let Some(mtm) = MainThreadMarker::new() else {
        log!("the menu bar front end must run on the main thread; continuing without a display");
        return super::headless::run(native);
    };
    let application = NSApplication::sharedApplication(mtm);
    // A menu bar item only: no Dock icon and no menu bar of its own.
    application.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let Native { requests, incoming, launch, paths } = native;
    let target = Target::new(mtm);
    let menu = menu::StatusMenu::new(mtm, &target);
    APP.with(|cell| {
        *cell.borrow_mut() = Some(App { mtm, target, requests, paths, menu, overlay: None, timer: None, status: None });
    });

    let spawned = thread::Builder::new().name("runtime".into()).spawn(move || {
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
            launch("macos", Box::new(Handle), Box::new(probe::SystemProbe)).run(incoming);
        }));
        if outcome.is_err() {
            // The menu bar item is no use without the runtime. Exit with a failure so
            // launchd starts the agent again.
            DispatchQueue::main().exec_async(|| std::process::exit(1));
        }
    });
    if let Err(e) = spawned {
        log!("could not start the runtime thread: {e}");
        return ExitCode::FAILURE;
    }
    application.run();
    ExitCode::SUCCESS
}

impl App {
    fn flash(&mut self, spec: &FlashSpec) {
        self.overlay = overlay::Overlay::show(self.mtm, spec);
        if self.overlay.is_none() {
            log!("could not show a {} flash: no overlay window could be created", spec.kind);
            self.stop_timer();
        } else if self.timer.is_none() {
            // SAFETY: Target implements `tick:`, and the timer keeps it alive.
            let timer = unsafe {
                NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                    1.0 / 60.0,
                    &self.target,
                    sel!(tick:),
                    None,
                    true,
                )
            };
            self.timer = Some(timer);
        }
    }

    fn animate(&mut self) {
        if !self.overlay.as_mut().is_some_and(overlay::Overlay::frame) {
            self.overlay = None;
            self.stop_timer();
        }
    }

    fn stop_timer(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.invalidate();
        }
    }

    fn send(&self, control: Control) {
        let _ = self.requests.send(Request::Control(control, None));
    }
}

/// A notification through Notification Center, without sound. The text travels as
/// script arguments, never as script source, so nothing in it can be run.
fn notify(notice: &Notice) {
    let child = Command::new("osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 2 of argv) with title (item 1 of argv)",
            "-e",
            "end run",
            &notice.title,
            &notice.body,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match child {
        Ok(child) => system::reap(child),
        Err(e) => log!("could not show a notification: {e}"),
    }
}
