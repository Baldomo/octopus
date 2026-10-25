mod browser;
mod commands;
mod hub;
mod task_lists;
mod terminal;
pub mod types;
mod window;

use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use anyhow::Context;
use octopus_client::Identity;
use octopus_client::servers::Servers;
use serde::Serialize;
use tauri::{AppHandle, Emitter, EventTarget, Manager, WebviewWindowBuilder};

use crate::browser::Browsers;
use crate::commands::Launched;
use crate::hub::Hub;
use crate::terminal::Terminals;

/// The label of the window and of the webview that renders the UI in it, as `tauri.conf.json` declares it.
const UI: &str = "main";

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// A count or duration for the UI, which a `u32` always holds in practice.
pub(crate) fn saturate(n: impl TryInto<u32>) -> u32 {
    n.try_into().unwrap_or(u32::MAX)
}

/// An event only fails to reach the UI once its webview is gone, and then nobody is left to tell.
pub(crate) fn emit(app: &AppHandle, event: &str, payload: impl Serialize + Clone) {
    if let Err(error) = app.emit_to(EventTarget::labeled(UI), event, payload) {
        eprintln!("octopus app: emit {event}: {error}");
    }
}

fn open_window(app: &tauri::App) -> anyhow::Result<()> {
    let background: tauri::window::Color =
        env!("OCTOPUS_BG").parse().map_err(|error: String| anyhow::anyhow!("DESIGN.html --bg: {error}"))?;
    let config = app.config().app.windows.iter().find(|window| window.label == UI).context("tauri.conf.json window")?;
    WebviewWindowBuilder::from_config(app, config)?.background_color(background).build()?;
    Ok(())
}

/// Opens the window and blocks until it closes.
pub fn run() -> anyhow::Result<()> {
    let launched = Instant::now();
    let servers = Servers::load()?.servers;
    let me = Identity::load()?;
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::boot,
            commands::ready,
            commands::server_pair,
            commands::task_focus,
            commands::task_move,
            commands::task_create,
            commands::term_attach,
            commands::term_input,
            commands::term_detach,
            commands::term_take_lock,
            commands::term_resize,
            commands::file_list,
            commands::file_read,
            commands::file_write,
            commands::file_search,
            commands::browser_open,
            commands::browser_show,
            commands::browser_place,
            commands::browser_go,
            commands::browser_close,
        ])
        .setup(move |app| {
            app.manage(Launched(launched));
            app.manage(Terminals::default());
            app.manage(Browsers::default());
            app.manage(Hub::start(app.handle().clone(), me, servers));
            open_window(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .context("run the app")
}
