use std::path::{Component, Path};
use std::sync::Arc;

use octopus_proto::ops::{BrowserClose, BrowserMove, BrowserOpen};
use octopus_proto::types::{BrowserId, BrowserTarget};
use octopus_shell::hub::Hub;
use octopus_shell::task_lists::TaskRef;
use serde::Deserialize;
use tauri::{AppHandle, State};
use ts_rs::TS;

use crate::browser::{Browsers, Go, OnScreen};
use crate::menu::MenuGroup;

type Hubbed<'a> = State<'a, Arc<Hub>>;
type Browsing<'a> = State<'a, Browsers>;

/// Shows `url` in a browser panel of the task on its server, which pushes the panel to every client; resolves to
/// the panel's id.
#[tauri::command]
pub async fn browser_open(
    hub: Hubbed<'_>,
    at: TaskRef,
    panel: BrowserTarget,
    url: String,
) -> Result<BrowserId, String> {
    Ok(hub.call(&at.server, &BrowserOpen { task: at.task, url, panel }).await?.panel)
}

/// Removes a browser panel of the task on its server, which every client then stops rendering.
#[tauri::command]
pub async fn browser_close(hub: Hubbed<'_>, at: TaskRef, panel: BrowserId) -> Result<(), String> {
    hub.call(&at.server, &BrowserClose { task: at.task, panel }).await.map(drop)
}

/// Moves a browser panel of the task on its server to index `to` among the task's panels, on every client.
#[tauri::command]
pub async fn browser_move(hub: Hubbed<'_>, at: TaskRef, panel: BrowserId, to: u16) -> Result<(), String> {
    hub.call(&at.server, &BrowserMove { task: at.task, panel, to }).await.map(drop)
}

/// Loads the panel's `url` on this client unless its webview already loaded this `load` of the panel.
#[tauri::command]
pub async fn browser_load(
    app: AppHandle,
    hub: Hubbed<'_>,
    browsers: Browsing<'_>,
    at: TaskRef,
    panel: BrowserId,
    url: String,
    load: u64,
) -> Result<(), String> {
    browsers.load(&app, &hub, &at, &panel, &url, load).await
}

#[tauri::command]
pub async fn browser_show_page(browsers: Browsing<'_>, on_screen: Option<OnScreen>) -> Result<(), String> {
    browsers.show_page(on_screen)
}

#[tauri::command]
pub async fn browser_go(browsers: Browsing<'_>, at: TaskRef, panel: BrowserId, go: Go) -> Result<(), String> {
    browsers.go(&at, &panel, go)
}

#[tauri::command]
pub async fn browser_find(
    browsers: Browsing<'_>,
    at: TaskRef,
    panel: BrowserId,
    text: String,
    backwards: bool,
) -> Result<(), String> {
    browsers.find(&at, &panel, text, backwards)
}

/// Lists the UI's commands in the macOS menu bar.
#[tauri::command]
pub fn app_menu(app: AppHandle, groups: Vec<MenuGroup>) -> Result<(), String> {
    crate::menu::set(&app, &groups).map_err(|error| error.to_string())
}

/// Closes the panel's webview on this client; the panel stays on its server.
#[tauri::command]
pub async fn browser_forget(browsers: Browsing<'_>, at: TaskRef, panel: BrowserId) -> Result<(), String> {
    browsers.forget(&at, &panel).await
}

/// What this machine does with an entry of a task whose server runs here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Outside {
    /// Shows it in the system's file manager.
    Reveal,
    /// Opens it in its default app.
    Open,
}

/// Acts on a checkout entry through this machine's own apps, which reach it only when its server runs here.
#[tauri::command]
pub fn file_outside(hub: Hubbed<'_>, at: TaskRef, path: String, how: Outside) -> Result<(), String> {
    if hub.is_another_machine(&at.server) {
        return Err(format!("{} runs on another machine", hub.label(&at.server)));
    }
    let relative = Path::new(&path);
    if relative.components().any(|component| !matches!(component, Component::Normal(_) | Component::CurDir)) {
        return Err(format!("{path} is not a path inside the checkout"));
    }
    let task = hub.model().tasks.remove(&at.key()).ok_or_else(|| format!("no task {}", at.task))?;
    let entry = Path::new(&task.task.directory).join(relative);
    match how {
        Outside::Reveal => tauri_plugin_opener::reveal_item_in_dir(&entry),
        Outside::Open => tauri_plugin_opener::open_path(&entry, None::<&str>),
    }
    .map_err(|error| format!("{}: {error}", entry.display()))
}

/// What double-clicking the title bar does, as the user set it: on macOS in System Settings ▸ Desktop & Dock,
/// elsewhere always a zoom.
fn title_bar_action() -> &'static str {
    if !cfg!(target_os = "macos") {
        return "Maximize";
    }
    let read = std::process::Command::new("defaults").args(["read", "-g", "AppleActionOnDoubleClick"]).output();
    match read.map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string()).as_deref() {
        Ok("Minimize") => "Minimize",
        Ok("None") => "None",
        _ => "Maximize",
    }
}

#[tauri::command]
pub fn title_bar_double_click(window: tauri::Window) -> Result<(), String> {
    let done = match title_bar_action() {
        "Minimize" => window.minimize(),
        "None" => Ok(()),
        _ if window.is_maximized().map_err(|error| error.to_string())? => window.unmaximize(),
        _ => window.maximize(),
    };
    done.map_err(|error| error.to_string())
}

/// Taps the trackpad the way macOS marks a dragged item lining up; elsewhere does nothing.
#[tauri::command]
pub fn haptic_alignment() {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{
            NSHapticFeedbackManager, NSHapticFeedbackPattern, NSHapticFeedbackPerformanceTime,
            NSHapticFeedbackPerformer,
        };
        NSHapticFeedbackManager::defaultPerformer().performFeedbackPattern_performanceTime(
            NSHapticFeedbackPattern::Alignment,
            NSHapticFeedbackPerformanceTime::Now,
        );
    }
}
