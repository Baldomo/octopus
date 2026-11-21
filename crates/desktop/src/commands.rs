use std::path::{Component, Path};
use std::sync::Arc;

use octopus_proto::types::BrowserId;
use octopus_shell::browsing::OnScreen;
use octopus_shell::hub::Hub;
use octopus_shell::task_lists::TaskRef;
use octopus_shell::webkit::Go;
use serde::Deserialize;
use tauri::{AppHandle, State, Webview};
use ts_rs::TS;

use crate::Skill;
use crate::browser::Browsers;
use crate::menu::MenuGroup;
use crate::setup::{Setup, SetupView};
use crate::show::{ShowDrive, ShowScene, ShowSpot, Shows};

type Hubbed<'a> = State<'a, Arc<Hub>>;
type Browsing<'a> = State<'a, Browsers>;

#[tauri::command]
pub(crate) async fn browser_load(
    browsers: Browsing<'_>,
    at: TaskRef,
    panel: BrowserId,
    url: String,
) -> Result<(), String> {
    browsers.load(&at, &panel, &url).await
}

#[tauri::command]
pub(crate) async fn browser_show_page(browsers: Browsing<'_>, on_screen: Option<OnScreen>) -> Result<(), String> {
    browsers.show_page(on_screen)
}

#[tauri::command]
pub(crate) async fn browser_go(browsers: Browsing<'_>, at: TaskRef, panel: BrowserId, go: Go) -> Result<(), String> {
    browsers.go(&at, &panel, go)
}

#[tauri::command]
pub(crate) async fn browser_find(
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
pub(crate) fn app_menu(app: AppHandle, groups: Vec<MenuGroup>) -> Result<(), String> {
    crate::menu::set(&app, &groups).map_err(|error| error.to_string())
}

/// Closes the panel's webview on this client; the panel stays on its server.
#[tauri::command]
pub(crate) async fn browser_forget(browsers: Browsing<'_>, at: TaskRef, panel: BrowserId) -> Result<(), String> {
    browsers.forget(&at, &panel).await
}

/// What this machine does with an entry of a task whose server runs here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outside {
    /// Shows it in the system's file manager.
    Reveal,
    /// Opens it in its default app.
    Open,
}

/// Acts on an entry of a task's directory through this machine's own apps, which reach it only when its server runs here.
#[tauri::command]
pub(crate) fn file_outside(hub: Hubbed<'_>, at: TaskRef, path: String, how: Outside) -> Result<(), String> {
    if hub.is_another_machine(&at.server) {
        return Err(format!("{} runs on another machine", hub.label(&at.server)));
    }
    let relative = Path::new(&path);
    if relative.components().any(|component| !matches!(component, Component::Normal(_) | Component::CurDir)) {
        return Err(format!("{path} is not a path inside the task's directory"));
    }
    let entry = Path::new(&hub.task(&at)?.directory).join(relative);
    match how {
        Outside::Reveal => tauri_plugin_opener::reveal_item_in_dir(&entry),
        Outside::Open => tauri_plugin_opener::open_path(&entry, None::<&str>),
    }
    .map_err(|error| format!("{}: {error}", entry.display()))
}

/// What the user set double-clicking a title bar to do in System Settings ▸ Desktop & Dock; nothing outside macOS.
fn title_bar_setting() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let read = std::process::Command::new("defaults").args(["read", "-g", "AppleActionOnDoubleClick"]).output().ok()?;
    Some(String::from_utf8_lossy(&read.stdout).trim().to_string())
}

/// Does what double-clicking the title bar does as the user set it, else a zoom.
#[tauri::command]
pub(crate) fn title_bar_double_click(window: tauri::Window) -> Result<(), String> {
    let done = match title_bar_setting().as_deref() {
        Some("Minimize") => window.minimize(),
        Some("None") => Ok(()),
        _ if window.is_maximized().map_err(|error| error.to_string())? => window.unmaximize(),
        _ => window.maximize(),
    };
    done.map_err(|error| error.to_string())
}

/// The install of this machine's server this launch runs, if any.
#[tauri::command]
pub(crate) fn setup(setup: State<'_, Setup>) -> Option<SetupView> {
    setup.view()
}

/// Installs this machine's server again after a failed install.
#[tauri::command]
pub(crate) fn setup_retry(setup: State<'_, Setup>) {
    setup.retry();
}

/// Whether a show is playing.
#[tauri::command]
pub(crate) fn show_playing(shows: State<'_, Shows>) -> bool {
    shows.current().is_some_and(|show| show.playing())
}

/// The scene of the display the asking show page covers.
#[tauri::command]
pub(crate) fn show_scene(webview: Webview, shows: State<'_, Shows>) -> Result<ShowScene, String> {
    Ok(shows.of(webview.label())?.scene(webview.label()))
}

/// The asking show page is prepared; the show starts once every page is.
#[tauri::command]
pub(crate) fn show_ready(webview: Webview, shows: State<'_, Shows>) -> Result<(), String> {
    shows.of(webview.label())?.ready(webview.label());
    Ok(())
}

/// Hands every show page octopus's window as it stands, for the show to dive into.
#[tauri::command]
pub(crate) async fn show_app(webview: Webview, shows: State<'_, Shows>) -> Result<(), String> {
    shows.of(webview.label())?.app().await;
    Ok(())
}

/// Hands octopus's window the tour's next step.
#[tauri::command]
pub(crate) fn show_drive(webview: Webview, shows: State<'_, Shows>, step: ShowDrive) -> Result<(), String> {
    shows.of(webview.label())?.drive(step);
    Ok(())
}

/// A layer of octopus's window tells the playing show where a target sits.
#[tauri::command]
pub(crate) fn show_spot(shows: State<'_, Shows>, spot: ShowSpot) {
    if let Some(show) = shows.current() {
        show.spot(spot);
    }
}

/// Shows or hides the cursor while the show plays.
#[tauri::command]
pub(crate) fn show_cursor(webview: Webview, shows: State<'_, Shows>, shown: bool) -> Result<(), String> {
    shows.of(webview.label())?.cursor(shown);
    Ok(())
}

/// Plays the system's alert sound over the show's song.
#[tauri::command]
pub(crate) fn show_alert(webview: Webview, shows: State<'_, Shows>) -> Result<(), String> {
    shows.of(webview.label())?.alert();
    Ok(())
}

/// Ends the show at once.
#[tauri::command]
pub(crate) fn show_skip(webview: Webview, shows: State<'_, Shows>) -> Result<(), String> {
    shows.of(webview.label())?.end();
    Ok(())
}

/// Writes the octopus skill for the user's Claude Code, as `octopus skill install` does.
#[tauri::command]
pub(crate) fn skill_install(skill: State<'_, Skill>) -> Result<(), String> {
    (skill.0)().map(drop).map_err(|error| format!("{error:#}"))
}

/// Taps the trackpad the way macOS marks a dragged item lining up; elsewhere does nothing.
#[tauri::command]
pub(crate) fn haptic_alignment() {
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
