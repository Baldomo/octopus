//! The client app's core every client app adds as its `octopus` plugin.

pub mod client_menu;
mod commands;
pub mod hub;
pub mod microphone;
pub mod task_lists;
mod task_view;
mod terminal;
pub mod types;
pub mod ui;

use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use octopus_client::Identity;
use octopus_client::servers::{Config, Servers};
use serde::Serialize;

use crate::microphone::AskAccess;
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Emitter, Manager, Runtime, Wry};

use crate::commands::Launched;
use crate::hub::Hub;
use crate::terminal::Terminals;
use crate::ui::UiStore;

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// A count or duration for the UI, which a `u32` always holds in practice.
pub(crate) fn saturate(n: impl TryInto<u32>) -> u32 {
    n.try_into().unwrap_or(u32::MAX)
}

/// Reaches every layer of the UI, the webviews that listen; an event only fails to once the window is gone, and
/// then nobody is left to tell.
pub fn emit(app: &AppHandle, event: &str, payload: impl Serialize + Clone) {
    if let Err(error) = app.emit(event, payload) {
        eprintln!("octopus app: emit {event}: {error}");
    }
}

/// The plugin's commands, before it manages what they act on.
pub fn commands<R: Runtime>() -> tauri::plugin::Builder<R> {
    tauri::plugin::Builder::new("octopus").invoke_handler(tauri::generate_handler![
        commands::boot,
        commands::ready,
        commands::ui_act,
        commands::quit,
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
        commands::term_open,
        commands::term_close,
        commands::term_claim,
        commands::term_take_lock,
        commands::term_resize,
        commands::file_list,
        commands::file_read,
        commands::file_write,
        commands::file_search,
    ])
}

/// The core's commands and the terminals they attach, timing launch from the call.
pub fn plugin() -> TauriPlugin<Wry> {
    let launched = Instant::now();
    commands()
        .setup(move |app, _| {
            app.manage(Launched(launched));
            app.manage(Terminals::default());
            Ok(())
        })
        .build()
}

/// What a client app loads before its window opens, so a launch that cannot load them fails with no window.
pub struct Client {
    pub config: Config,
    pub me: Identity,
    pub servers: Servers,
    /// Asks the OS for the microphone on the first capture that needs it.
    pub microphone_access: AskAccess,
}

/// Manages the `Arc<Hub>` and `Arc<UiStore>` the commands act on, connecting as `me` to its servers.
pub fn start(app: &AppHandle, client: Client, launch_problem: Option<String>) {
    let Client { config, me, servers, microphone_access } = client;
    app.manage(UiStore::load(app.clone(), config.clone(), launch_problem));
    app.manage(Hub::start(app.clone(), config, me, servers.servers, microphone_access));
}
