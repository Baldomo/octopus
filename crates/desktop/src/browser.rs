//! Each browser panel of a task renders as a native webview in the window's page layer. A page on a loopback host
//! of a server on another machine loads through a tunnel to that port on the server's machine.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use octopus_client::tunnel::{Tunnel, listen_on_localhost};
use octopus_proto::types::{BrowserId, web_page};
use octopus_shell::hub::Hub;
use octopus_shell::task_lists::TaskRef;
use octopus_shell::ui::Bounds;
use octopus_shell::{emit, lock};
use serde::{Deserialize, Serialize};
use tauri::webview::{PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, Url, Webview, WebviewUrl};
use ts_rs::TS;
use url::Host;

use crate::layers::Layers;

pub const NAVIGATED_EVENT: &str = "browser_navigated";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Go {
    Back,
    Forward,
    Reload,
}

/// A page finished loading in a browser panel, which may be one the user navigated to inside the page.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Navigated {
    /// The task's key.
    pub task: String,
    pub panel: BrowserId,
    pub url: String,
}

/// A browser panel anywhere in the client: `<server>/<task>/<panel>`.
fn panel_key(at: &TaskRef, panel: &BrowserId) -> String {
    format!("{}/{panel}", at.key())
}

/// The port of the server's machine a page reaches, when its host is a loopback name or address.
fn loopback_port(page: &Url) -> Option<u16> {
    let loopback = match page.host()? {
        Host::Domain(name) => name == "localhost",
        Host::Ipv4(address) => address.is_loopback(),
        Host::Ipv6(address) => address.is_loopback(),
    };
    loopback.then(|| page.port_or_known_default()).flatten()
}

/// The browser panel the page layer shows, at its rect.
#[derive(Debug, Clone, PartialEq, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct OnScreen {
    pub at: TaskRef,
    pub panel: BrowserId,
    pub bounds: Bounds,
}

struct Rendered {
    webview: Webview,
    /// The `url` of the panel's server state that the webview last loaded.
    url: String,
}

/// One listening tunnel, held while any panel shows a page on its port.
struct Held {
    server: String,
    panels: BTreeSet<String>,
    _tunnel: Tunnel,
}

pub struct Browsers {
    layers: Arc<Layers>,
    next: AtomicU64,
    rendered: Mutex<HashMap<String, Rendered>>,
    /// Keyed by port: this client's `localhost:<port>` reaches one server at a time.
    tunnels: tokio::sync::Mutex<HashMap<u16, Held>>,
}

fn failed(error: tauri::Error) -> String {
    error.to_string()
}

impl Browsers {
    pub fn new(layers: Arc<Layers>) -> Self {
        Self { layers, next: AtomicU64::new(0), rendered: Mutex::default(), tunnels: tokio::sync::Mutex::default() }
    }

    fn with<T>(
        &self,
        at: &TaskRef,
        panel: &BrowserId,
        act: impl FnOnce(&Webview) -> tauri::Result<T>,
    ) -> Result<T, String> {
        let key = panel_key(at, panel);
        let rendered = lock(&self.rendered);
        let rendered = rendered.get(&key).ok_or_else(|| format!("no browser panel {key}"))?;
        act(&rendered.webview).map_err(failed)
    }

    /// Loads the panel's `url` into its webview, created hidden on its first page; `show_page` puts it on screen.
    /// Resolves to the port the page reaches through a tunnel, if it does.
    pub async fn load(
        &self,
        app: &AppHandle,
        hub: &Arc<Hub>,
        at: &TaskRef,
        panel: &BrowserId,
        url: &str,
    ) -> Result<Option<u16>, String> {
        let key = panel_key(at, panel);
        let page = web_page(url)?;
        let tunneled = loopback_port(&page).filter(|_| hub.is_another_machine(&at.server));
        self.hold_tunnel(hub, &at.server, &key, tunneled).await?;
        if let Err(error) = self.point(app, at, panel, page) {
            release(&mut *self.tunnels.lock().await, &key, None);
            return Err(error);
        }
        Ok(tunneled)
    }

    /// Points the panel's webview at `page`, creating it on the panel's first page.
    fn point(&self, app: &AppHandle, at: &TaskRef, panel: &BrowserId, page: Url) -> Result<(), String> {
        let (key, url) = (panel_key(at, panel), page.to_string());
        let mut rendered = lock(&self.rendered);
        match rendered.get_mut(&key) {
            Some(existing) if existing.url == url => {}
            Some(existing) => {
                existing.webview.navigate(page).map_err(failed)?;
                existing.url = url;
            }
            None => {
                let webview = self.create(app, at, panel, page)?;
                rendered.insert(key, Rendered { webview, url });
            }
        }
        Ok(())
    }

    fn create(&self, app: &AppHandle, at: &TaskRef, panel: &BrowserId, url: Url) -> Result<Webview, String> {
        let label = format!("browser-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let (events, task, panel) = (app.clone(), at.key(), panel.clone());
        let builder = WebviewBuilder::new(label, WebviewUrl::External(url)).on_page_load(move |_, payload| {
            if payload.event() == PageLoadEvent::Finished {
                let navigated = Navigated { task: task.clone(), panel: panel.clone(), url: payload.url().to_string() };
                emit(&events, NAVIGATED_EVENT, navigated);
            }
        });
        self.layers.add_page(builder).map_err(failed)
    }

    /// Holds the tunnel to `port` for the panel `key`, and lets go of any other it held.
    async fn hold_tunnel(&self, hub: &Arc<Hub>, server: &str, key: &str, port: Option<u16>) -> Result<(), String> {
        let mut tunnels = self.tunnels.lock().await;
        if let Some(port) = port
            && let Some(held) = tunnels.get(&port)
            && held.server != server
        {
            return Err(format!("this client's port {port} already tunnels to {}", held.server));
        }
        release(&mut tunnels, key, port);
        let Some(port) = port else { return Ok(()) };
        match tunnels.get_mut(&port) {
            Some(held) => {
                held.panels.insert(key.to_string());
            }
            None => {
                let listeners = listen_on_localhost(port).await.map_err(|error| error.to_string())?;
                let (hub, dialed) = (Arc::clone(hub), server.to_string());
                let tunnel = Tunnel::new(port, listeners, Arc::new(move || hub.connection(&dialed)));
                let panels = BTreeSet::from([key.to_string()]);
                tunnels.insert(port, Held { server: server.to_string(), panels, _tunnel: tunnel });
            }
        }
        Ok(())
    }

    pub fn show_page(&self, on_screen: Option<OnScreen>) -> Result<(), String> {
        let next = on_screen
            .map(|OnScreen { at, panel, bounds }| self.with(&at, &panel, |webview| Ok((webview.clone(), bounds))))
            .transpose()?;
        self.layers.show_page(next).map_err(failed)
    }

    pub fn go(&self, at: &TaskRef, panel: &BrowserId, go: Go) -> Result<(), String> {
        self.with(at, panel, |webview| match go {
            Go::Back => webview.eval("history.back()"),
            Go::Forward => webview.eval("history.forward()"),
            Go::Reload => webview.reload(),
        })
    }

    /// Closes the panel's webview on this client; the panel stays on its server.
    pub async fn close(&self, at: &TaskRef, panel: &BrowserId) -> Result<(), String> {
        let key = panel_key(at, panel);
        release(&mut *self.tunnels.lock().await, &key, None);
        let rendered = lock(&self.rendered).remove(&key).ok_or_else(|| format!("no browser panel {key}"))?;
        self.layers.close_page(&rendered.webview).map_err(failed)
    }
}

/// Drops the panel `key` from every tunnel but the one to `keep`, and ends each tunnel no panel holds.
fn release(tunnels: &mut HashMap<u16, Held>, key: &str, keep: Option<u16>) {
    for (port, held) in tunnels.iter_mut() {
        if Some(*port) != keep {
            held.panels.remove(key);
        }
    }
    tunnels.retain(|_, held| !held.panels.is_empty());
}
