//! The macOS menu bar lists the UI's commands under their keys. AppKit gives a key to the menu once the focused
//! webview leaves it unhandled, so a command's key reaches the UI even while a browser page holds the focus.

use octopus_shell::emit;
use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, Wry};
use ts_rs::TS;

pub(crate) const COMMAND_EVENT: &str = "menu_command";

/// A command of the UI under one of its keys, which the menu names back to it when chosen.
#[derive(Debug, Clone, PartialEq, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct MenuCommand {
    /// The key as the UI writes it, e.g. `⌘⇧V`.
    pub key: String,
    pub label: String,
    /// The key in Tauri's accelerator syntax, e.g. `Cmd+Shift+KeyV`.
    pub accelerator: String,
}

/// A menu of the menu bar.
#[derive(Debug, Clone, PartialEq, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct MenuGroup {
    pub title: String,
    pub commands: Vec<MenuCommand>,
}

fn group(app: &AppHandle, group: &MenuGroup) -> tauri::Result<Submenu<Wry>> {
    let submenu = Submenu::new(app, &group.title, true)?;
    for command in &group.commands {
        submenu.append(&MenuItem::with_id(app, &command.key, &command.label, true, Some(&command.accelerator))?)?;
    }
    Ok(submenu)
}

/// Puts the app's, Edit's and Window's standard items around `groups` in the menu bar.
pub fn set(app: &AppHandle, groups: &[MenuGroup]) -> tauri::Result<()> {
    let separator = || PredefinedMenuItem::separator(app);
    let octopus = Submenu::with_items(
        app,
        "octopus",
        true,
        &[
            &PredefinedMenuItem::about(app, None, None)?,
            &separator()?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &separator()?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &separator()?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[&PredefinedMenuItem::minimize(app, None)?, &PredefinedMenuItem::fullscreen(app, None)?],
    )?;
    let menu = Menu::with_items(app, &[&octopus, &edit])?;
    for group in groups {
        menu.append(&self::group(app, group)?)?;
    }
    menu.append(&window)?;
    app.set_menu(menu)?;
    #[cfg(target_os = "macos")]
    app.run_on_main_thread(shifted::type_shifted_keys)?;
    Ok(())
}

/// AppKit matches a key with ⇧ by the character it types, such as `T` for ⌘⇧T and `}` for ⌘⇧], where Tauri writes
/// the unshifted key and ⇧; the menu's keys take the typed character instead.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod shifted {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSString;

    const SHIFT: usize = 1 << 17;
    const TYPED: [(&str, &str); 8] =
        [("[", "{"), ("]", "}"), ("\\", "|"), (",", "<"), ("/", "?"), ("=", "+"), ("-", "_"), ("\t", "\u{19}")];

    fn typed(key: &str) -> Option<String> {
        let punctuation = TYPED.iter().find(|(unshifted, _)| key == *unshifted).map(|(_, typed)| (*typed).to_string());
        let letter = Some(key).filter(|key| key.len() == 1 && key.chars().all(|letter| letter.is_ascii_lowercase()));
        punctuation.or_else(|| letter.map(str::to_uppercase))
    }

    /// # Safety
    /// `menu` is an `NSMenu`, on the main thread.
    unsafe fn retype(menu: &AnyObject) {
        let count: isize = unsafe { msg_send![menu, numberOfItems] };
        for index in 0..count {
            let item: *mut AnyObject = unsafe { msg_send![menu, itemAtIndex: index] };
            let item = unsafe { &*item };
            let mask: usize = unsafe { msg_send![item, keyEquivalentModifierMask] };
            let key: *mut NSString = unsafe { msg_send![item, keyEquivalent] };
            let key = unsafe { &*key };
            if let Some(typed) = typed(&key.to_string()).filter(|_| mask & SHIFT != 0) {
                let typed = NSString::from_str(&typed);
                let () = unsafe { msg_send![item, setKeyEquivalent: &*typed] };
                let () = unsafe { msg_send![item, setKeyEquivalentModifierMask: mask & !SHIFT] };
            }
            let submenu: *mut AnyObject = unsafe { msg_send![item, submenu] };
            if let Some(submenu) = unsafe { submenu.as_ref() } {
                unsafe { retype(submenu) };
            }
        }
    }

    pub(crate) fn type_shifted_keys() {
        let application = AnyClass::get(c"NSApplication").expect("AppKit has NSApplication");
        // SAFETY: Tauri runs this on the main thread, where the shared application holds the menu bar it just set.
        unsafe {
            let shared: *mut AnyObject = msg_send![application, sharedApplication];
            let menu: *mut AnyObject = msg_send![&*shared, mainMenu];
            if let Some(menu) = menu.as_ref() {
                retype(menu);
            }
        }
    }
}

/// A key the menu took, and whether a browser page held the keyboard then.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MenuKey {
    pub key: String,
    pub in_page: bool,
}

/// Tells the UI each key chosen from the menu.
pub fn relay(app: &AppHandle) {
    app.on_menu_event(|app, event| {
        let in_page = app.get_window(crate::layers::BASE).is_some_and(|window| crate::browser::page_has_focus(&window));
        emit(app, COMMAND_EVENT, MenuKey { key: event.id().0.clone(), in_page });
    });
}
