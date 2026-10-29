//! The iOS and Android app: the client app's core behind one full-screen webview.

use std::path::Path;

use anyhow::Context;
use octopus_client::Identity;
use octopus_client::servers::{Config, Servers};
use octopus_shell::Client;
use tauri::{Manager, WebviewWindowBuilder};

use crate::browser::{self, Shown};
use crate::client_menu::{self, Phone};
use crate::device;
use crate::feel::{self, Feel};

/// Under the OS's config directory, which on Android is the app's whole data directory, sealed key included.
const CONFIG_DIR: &str = "octopus";

#[tauri::mobile_entry_point]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(octopus_shell::plugin())
        .plugin(tauri_plugin_barcode_scanner::init())
        .plugin(tauri_plugin_clipboard_manager::init());
    #[cfg(target_os = "android")]
    let builder = builder.plugin(device::plugin());
    builder
        .invoke_handler(tauri::generate_handler![
            browser::phone_browse,
            browser::phone_unbrowse,
            client_menu::phone_named,
            client_menu::phone_invite_key,
            client_menu::phone_pair,
            phone_insets,
            phone_settings,
            phone_feel,
            phone_back_pops
        ])
        .setup(|app| {
            let handle = app.handle();
            let dir = app.path().app_config_dir()?.join(CONFIG_DIR);
            let key = device::key(handle, || start_unpaired(&dir))?;
            let config = Config::at(dir);
            let (phone, name) = Phone::load(config.clone(), || device::name(handle))?;
            let asking = handle.clone();
            let client = Client {
                me: Identity::new(name, key),
                servers: Servers::load(&config)?,
                config,
                microphone_access: Box::new(move || device::microphone(&asking)),
            };
            app.manage(phone);
            app.manage(Shown::default());
            octopus_shell::start(handle, client, None);
            let window = app.config().app.windows.first().context("tauri.conf.json declares the window")?;
            WebviewWindowBuilder::from_config(handle, window)?.build()?;
            #[cfg(target_os = "ios")]
            crate::webview::set_up(app)?;
            #[cfg(target_os = "android")]
            device::follow_appearance(app);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("run the app");
}

/// The page's insets where its webview leaves them out of the safe area, read again whenever the page resizes. Async,
/// so the OS answers on its own thread while this one waits.
#[tauri::command]
async fn phone_insets(app: tauri::AppHandle) -> Result<Option<device::Insets>, String> {
    device::insets(&app).map_err(|error| format!("{error:#}"))
}

/// Opens this app's page of the OS's settings, where a denied microphone is granted.
#[tauri::command]
fn phone_settings(app: tauri::AppHandle) -> Result<(), String> {
    device::settings(&app).map_err(|error| format!("{error:#}"))
}

/// Async, so a feel never waits on the thread that draws the page.
#[tauri::command]
async fn phone_feel(app: tauri::AppHandle, feel: Feel) -> Result<(), String> {
    feel::play(&app, feel).map_err(|error| format!("{error:#}"))
}

#[tauri::command]
async fn phone_back_pops(app: tauri::AppHandle, pops: bool) -> Result<(), String> {
    device::back_pops(&app, pops).map_err(|error| format!("{error:#}"))
}

/// Every server pinned the old key, so a phone with a new one starts as one that never paired.
fn start_unpaired(dir: &Path) -> anyhow::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("clear {}", dir.display()))
        }
        _ => Ok(()),
    }
}
