use std::sync::Arc;

use octopus_proto::ops::BrowserOpen;
use octopus_proto::types::{BrowserId, BrowserTarget};
use octopus_shell::hub::Hub;
use octopus_shell::task_lists::TaskRef;
use tauri::{AppHandle, State};

use crate::browser::{Browsers, Go, OnScreen};

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

/// Renders the panel's server `url` on this client; resolves to the port it reaches through a tunnel, if any.
#[tauri::command]
pub async fn browser_load(
    app: AppHandle,
    hub: Hubbed<'_>,
    browsers: Browsing<'_>,
    at: TaskRef,
    panel: BrowserId,
    url: String,
) -> Result<Option<u16>, String> {
    browsers.load(&app, &hub, &at, &panel, &url).await
}

#[tauri::command]
pub async fn browser_show_page(browsers: Browsing<'_>, on_screen: Option<OnScreen>) -> Result<(), String> {
    browsers.show_page(on_screen)
}

#[tauri::command]
pub async fn browser_go(browsers: Browsing<'_>, at: TaskRef, panel: BrowserId, go: Go) -> Result<(), String> {
    browsers.go(&at, &panel, go)
}

/// Closes the panel's webview on this client; the panel stays on its server.
#[tauri::command]
pub async fn browser_close(browsers: Browsing<'_>, at: TaskRef, panel: BrowserId) -> Result<(), String> {
    browsers.close(&at, &panel).await
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
