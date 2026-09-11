//! The menu bar item and its menu.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use flash_core::color::Rgb;
use flash_core::config;
use flash_core::event::Attention;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSApplication, NSControlStateValueOff, NSControlStateValueOn, NSImage, NSMenu,
    NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{NSString, ns_string};

use super::{App, Target, overlay, with_app};
use crate::api::{Control, Status};
use crate::ui::present::{self, KINDS, PAUSES};
use crate::{autostart, log, system};

const TOGGLE: isize = 1;
const PAUSE: isize = 10;
const RESUME: isize = 19;
const TEST: isize = 20;
const AUTOSTART: isize = 30;
const SETTINGS: isize = 31;
const JOURNAL: isize = 32;
const QUIT: isize = 40;

pub struct StatusMenu {
    item: Retained<NSStatusItem>,
    icons: HashMap<(u8, u8, u8), Retained<NSImage>>,
    shown: Option<(Rgb, String)>,
}

impl StatusMenu {
    pub fn new(mtm: MainThreadMarker, target: &Target) -> StatusMenu {
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        let menu = NSMenu::new(mtm);
        menu.setAutoenablesItems(false);
        // Filled in just before it opens, so it never shows a stale status.
        menu.setDelegate(Some(ProtocolObject::from_ref(target)));
        item.setMenu(Some(&menu));
        let mut status_menu = StatusMenu { item, icons: HashMap::new(), shown: None };
        status_menu.indicate(mtm, config::builtin(Attention::Done).color, "Claude Flash".to_owned());
        status_menu
    }

    pub fn show_status(&mut self, mtm: MainThreadMarker, status: &Status) {
        let (color, tip) = present::indicator(status);
        self.indicate(mtm, color, tip);
    }

    fn indicate(&mut self, mtm: MainThreadMarker, color: Rgb, tip: String) {
        if self.shown.as_ref().is_some_and(|(c, t)| *c == color && *t == tip) {
            return;
        }
        let Some(button) = self.item.button(mtm) else { return };
        let key = (color.r, color.g, color.b);
        if !self.icons.contains_key(&key)
            && let Some(image) = overlay::sphere_image(color)
        {
            self.icons.insert(key, image);
        }
        button.setImage(self.icons.get(&key).map(|image| &**image));
        button.setToolTip(Some(&NSString::from_str(&tip)));
        self.shown = Some((color, tip));
    }
}

/// Rebuilds `menu` from the latest status.
pub fn fill(app: &App, menu: &NSMenu) {
    let mtm = app.mtm;
    let target: &AnyObject = &app.target;
    let status = app.status.as_ref().map(|(status, at)| (status.as_ref(), at.elapsed().as_millis() as u64));
    let elapsed = status.map_or(0, |(_, elapsed)| elapsed);

    menu.removeAllItems();
    menu.addItem(&label(mtm, &present::headline(status.map(|(s, _)| s), elapsed)));
    if let Some((status, _)) = status {
        for wait in status.waiting.iter().take(6) {
            menu.addItem(&label(mtm, &present::waiting(wait, elapsed)));
        }
    }
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    let toggle = action(mtm, target, "Flashes on", TOGGLE);
    toggle.setState(state(status.is_none_or(|(s, _)| s.enabled)));
    menu.addItem(&toggle);

    let pause = NSMenu::new(mtm);
    pause.setAutoenablesItems(false);
    for (i, (_, text)) in PAUSES.iter().enumerate() {
        pause.addItem(&action(mtm, target, text, PAUSE + i as isize));
    }
    if status.is_some_and(|(s, _)| s.paused_for_ms.is_some()) {
        pause.addItem(&NSMenuItem::separatorItem(mtm));
        pause.addItem(&action(mtm, target, "Resume now", RESUME));
    }
    menu.addItem(&submenu(mtm, "Pause", &pause));

    let test = NSMenu::new(mtm);
    test.setAutoenablesItems(false);
    for (i, kind) in KINDS.iter().enumerate() {
        test.addItem(&action(mtm, target, present::title(*kind), TEST + i as isize));
    }
    menu.addItem(&submenu(mtm, "Test flash", &test));

    menu.addItem(&NSMenuItem::separatorItem(mtm));
    let login = action(mtm, target, "Start at login", AUTOSTART);
    login.setState(state(autostart::registered().is_some()));
    menu.addItem(&login);
    menu.addItem(&action(mtm, target, "Open settings", SETTINGS));
    menu.addItem(&action(mtm, target, "Open journal folder", JOURNAL));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&action(mtm, target, "Quit Claude Flash", QUIT));
}

fn state(on: bool) -> isize {
    if on { NSControlStateValueOn } else { NSControlStateValueOff }
}

fn item(mtm: MainThreadMarker, text: &str, action: Option<Sel>) -> Retained<NSMenuItem> {
    // SAFETY: when an action is given, it names a method Target implements.
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(text),
            action,
            ns_string!(""),
        )
    }
}

fn label(mtm: MainThreadMarker, text: &str) -> Retained<NSMenuItem> {
    let label = item(mtm, text, None);
    label.setEnabled(false);
    label
}

fn action(mtm: MainThreadMarker, target: &AnyObject, text: &str, tag: isize) -> Retained<NSMenuItem> {
    let action = item(mtm, text, Some(sel!(choose:)));
    // SAFETY: the target is the app's Target, which lives as long as the menu.
    unsafe { action.setTarget(Some(target)) };
    action.setTag(tag);
    action
}

fn submenu(mtm: MainThreadMarker, text: &str, menu: &NSMenu) -> Retained<NSMenuItem> {
    let parent = item(mtm, text, None);
    parent.setSubmenu(Some(menu));
    parent
}

pub fn choose(tag: isize) {
    let control = match tag {
        TOGGLE => Some(Control::Toggle { confirm: false }),
        RESUME => Some(Control::Resume),
        QUIT => confirm_quit().then_some(Control::Quit),
        t if (PAUSE..PAUSE + PAUSES.len() as isize).contains(&t) => {
            Some(Control::Pause { duration: PAUSES[(t - PAUSE) as usize].0.to_owned() })
        }
        t if (TEST..TEST + KINDS.len() as isize).contains(&t) => {
            Some(Control::Test { kind: KINDS[(t - TEST) as usize] })
        }
        _ => None,
    };
    if let Some(control) = control {
        with_app(|app| app.send(control));
        return;
    }
    match tag {
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

fn confirm_quit() -> bool {
    let Some(mtm) = MainThreadMarker::new() else { return false };
    let alert = NSAlert::new(mtm);
    alert.setMessageText(ns_string!("Quit Claude Flash?"));
    alert.setInformativeText(&NSString::from_str(present::QUIT_DETAIL));
    alert.addButtonWithTitle(ns_string!("Quit"));
    alert.addButtonWithTitle(ns_string!("Cancel"));
    // A menu bar app has to come forward, or its alert opens behind other windows.
    #[allow(deprecated)]
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    alert.runModal() == NSAlertFirstButtonReturn
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
