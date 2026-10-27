mod browser;
mod commands;
mod hub;
mod layers;
mod task_lists;
mod terminal;
pub mod types;
mod ui;
mod window;

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use anyhow::Context;
use octopus_client::Identity;
use octopus_client::servers::Servers;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::browser::Browsers;
use crate::commands::Launched;
use crate::hub::Hub;
use crate::layers::Layers;
use crate::terminal::Terminals;
use crate::ui::UiStore;

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// A count or duration for the UI, which a `u32` always holds in practice.
pub(crate) fn saturate(n: impl TryInto<u32>) -> u32 {
    n.try_into().unwrap_or(u32::MAX)
}

/// Reaches every layer of the UI, the webviews that listen; an event only fails to once the window is gone, and
/// then nobody is left to tell.
pub(crate) fn emit(app: &AppHandle, event: &str, payload: impl Serialize + Clone) {
    if let Err(error) = app.emit(event, payload) {
        eprintln!("octopus app: emit {event}: {error}");
    }
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
            commands::ui_act,
            commands::title_bar_double_click,
            commands::server_pair,
            commands::server_offer,
            commands::server_clients,
            commands::server_revoke,
            commands::server_rename,
            commands::server_forget,
            commands::workspace_add,
            commands::workspace_remove,
            commands::section_rename,
            commands::section_color,
            commands::auto_color,
            commands::task_focus,
            commands::task_move,
            commands::task_rename,
            commands::task_archive,
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
            commands::browser_load,
            commands::browser_show_page,
            commands::browser_go,
            commands::browser_close,
        ])
        .setup(move |app| {
            app.manage(Launched(launched));
            app.manage(Terminals::default());
            let ui = UiStore::load(app.handle().clone());
            app.manage(Arc::clone(&ui));
            app.manage(Hub::start(app.handle().clone(), me, servers));
            app.manage(Browsers::new(Layers::open(app, ui)?));
            Ok(())
        })
        .run(tauri::generate_context!())
        .context("run the app")
}
