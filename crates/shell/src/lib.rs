//! The client app's core every client app adds as its `octopus` plugin.

pub mod client_menu;
mod commands;
pub mod hub;
pub mod microphone;
pub mod served_files;
pub mod task_lists;
mod task_view;
mod terminal;
pub mod types;
pub mod ui;

use std::sync::Arc;
use std::time::Instant;

use octopus_client::Identity;
use octopus_client::servers::{Config, Servers};
pub use octopus_proto::lock;
use serde::Serialize;

use crate::microphone::AskAccess;
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Emitter, Manager, Runtime, Wry};

use crate::commands::Launched;
pub use crate::commands::Offered;
use crate::hub::Hub;
use crate::terminal::Terminals;
use crate::ui::UiStore;

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

macro_rules! plugin_commands {
    ($($command:ident),* $(,)?) => {
        /// The plugin's command names, which the UI invokes.
        pub(crate) const COMMANDS: &[&str] = &[$(stringify!($command)),*];

        /// The plugin's commands, before it manages what they act on.
        pub fn commands<R: Runtime>() -> tauri::plugin::Builder<R> {
            tauri::plugin::Builder::new("octopus").invoke_handler(tauri::generate_handler![$(commands::$command),*])
        }
    };
}

include!("../plugin_commands.rs");

/// The core's commands, the terminals they attach and the files it serves, timing launch from the call.
pub fn plugin() -> TauriPlugin<Wry> {
    let launched = Instant::now();
    served_files::protocol(commands())
        .setup(move |app, _| {
            app.manage(Launched(launched));
            app.manage(Terminals::default());
            app.manage(Offered::default());
            app.manage(Arc::new(served_files::ServedFiles::default()));
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
    app.manage(UiStore::load(app.clone(), config.clone(), &me, launch_problem));
    app.manage(Hub::start(app.clone(), config, me, servers, microphone_access));
}
