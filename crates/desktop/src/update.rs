//! Updating the Mac app and the AppImage in the background from the latest GitHub release; the next launch runs the
//! update.

use std::path::PathBuf;
use std::sync::Arc;

use octopus_shell::ui::UiStore;
use tauri::{AppHandle, Runtime};
use tauri_plugin_updater::UpdaterExt;

/// Checks for a newer release and installs it over `installed`, reporting a failure in the window.
pub fn start<R: Runtime>(app: &AppHandle<R>, ui: Arc<UiStore>, installed: PathBuf) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let updated = async {
            match app.updater()?.check().await? {
                Some(update) => update.download_and_install(|_, _| {}, || {}).await,
                None => Ok(()),
            }
        };
        if let Err(error) = updated.await {
            ui.report(format!("update {}: {error}", installed.display()));
        }
    });
}
