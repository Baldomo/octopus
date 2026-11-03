//! The phone's Browser screen shows one browser panel's page in a frame of its webview. A page on a loopback host
//! reaches that port on the server's machine through a tunnel from the same port on the phone, held while the screen
//! shows the page, and a file page loads through `octopus_shell::pages`.

use std::sync::Arc;

use octopus_client::tunnel::Tunnel;
use octopus_proto::types::web_page;
use octopus_shell::hub::Hub;
use octopus_shell::pages::Pages;
use serde::Serialize;
use tauri::State;

/// The tunnel the shown page reaches its server through, with the port and server it carries to.
#[derive(Default)]
pub struct Shown(tokio::sync::Mutex<Option<(u16, String, Tunnel)>>);

#[derive(Serialize)]
pub struct Page {
    url: String,
    /// Whether the page reaches the server's port through the tunnel.
    tunneled: bool,
}

/// Shows `url`, a page of `server`'s task, holding the tunnel to its port and letting go of any other.
#[tauri::command]
pub(crate) async fn phone_browse(
    hub: State<'_, Arc<Hub>>,
    shown: State<'_, Shown>,
    pages: State<'_, Arc<Pages>>,
    server: String,
    url: String,
) -> Result<Page, String> {
    let page = web_page(&url)?;
    let loaded = pages.loaded_from(&server, &page)?.to_string();
    let port = hub.tunneled_port(&server, &page);
    let mut held = shown.0.lock().await;
    let kept = matches!((&*held, port), (Some((at, to, _)), Some(port)) if *at == port && *to == server);
    if !kept {
        *held = None;
        if let Some(port) = port {
            let tunnel = hub.tunnel(&server, port).await?;
            held.replace((port, server, tunnel));
        }
    }
    Ok(Page { url: loaded, tunneled: port.is_some() })
}

/// Lets go of the shown page's tunnel once the Browser screen closes.
#[tauri::command]
pub(crate) async fn phone_unbrowse(shown: State<'_, Shown>) -> Result<(), String> {
    *shown.0.lock().await = None;
    Ok(())
}
