//! The iOS and Android app: the client app's core behind one full-screen webview.

use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use octopus_client::Identity;
use octopus_client::servers::{Config, Servers};
use octopus_shell::Client;
use octopus_shell::browsing::BrowserPages;
use octopus_shell::hub::Hub;
use tauri::{Manager, WebviewWindowBuilder};

use crate::browser;
use crate::client_menu::{self, Phone};
use crate::{device, feel, paste};

/// Under the OS's config directory, which on Android is the app's whole data directory, sealed key included.
const CONFIG_DIR: &str = "octopus";

#[tauri::mobile_entry_point]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(octopus_shell::plugin())
        .plugin(tauri_plugin_barcode_scanner::init())
        .plugin(tauri_plugin_opener::init());
    #[cfg(target_os = "android")]
    let builder = builder.plugin(device::plugin()).plugin(browser::plugin());
    builder
        .invoke_handler(tauri::generate_handler![
            browser::phone_browser_engine,
            browser::phone_browser_load,
            browser::phone_browser_place,
            browser::phone_browser_go,
            browser::phone_browser_find,
            browser::phone_browser_forget,
            client_menu::phone_named,
            client_menu::phone_invite_key,
            client_menu::phone_link_account,
            client_menu::phone_pair,
            client_menu::phone_join,
            device::phone_insets,
            device::phone_settings,
            device::phone_back_pops,
            feel::phone_feel,
            paste::phone_paste,
            #[cfg(target_os = "ios")]
            paste::phone_edit_menu,
            #[cfg(target_os = "ios")]
            paste::phone_edit_menu_close,
            phone_redial,
            phone_resume
        ])
        .setup(|app| {
            let handle = app.handle();
            paste::clear(handle)?;
            let dir = app.path().app_config_dir()?.join(CONFIG_DIR);
            let key = device::key(handle, || start_unpaired(&dir))?;
            let config = Config::at(dir);
            let (phone, name) = Phone::load(config.clone(), || device::name(handle))?;
            let asking = handle.clone();
            let client = Client {
                me: Identity::open(name, key, &config)?,
                servers: Servers::load(&config)?,
                config,
                microphone_access: Box::new(move || device::microphone(&asking)),
            };
            app.manage(phone);
            app.manage(BrowserPages::new(handle.clone()));
            octopus_shell::start(handle, client, None);
            let window = app.config().app.windows.first().context("tauri.conf.json declares the window")?;
            WebviewWindowBuilder::from_config(handle, window)?.build()?;
            #[cfg(target_os = "android")]
            browser::set_up(app)?;
            #[cfg(target_os = "ios")]
            crate::webview::set_up(app)?;
            #[cfg(target_os = "ios")]
            crate::linger::follow();
            #[cfg(target_os = "android")]
            device::follow_appearance(app);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("run the app");
}

#[tauri::command]
async fn phone_redial(hub: tauri::State<'_, Arc<Hub>>) -> Result<(), String> {
    hub.redial().await;
    Ok(())
}

#[tauri::command]
async fn phone_resume(hub: tauri::State<'_, Arc<Hub>>, servers: Vec<String>) -> Result<(), String> {
    hub.resume(&servers).await;
    Ok(())
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
