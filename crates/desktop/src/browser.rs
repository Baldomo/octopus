//! Each browser panel of a task renders as a native webview in the window's page layer. The webview loads the panel's
//! `url` each time its `load` moves, and reports every page it navigates to back to the server. A page on a loopback
//! host of a server on another machine reaches that port on the server's machine through a tunnel, and a file page
//! loads through `octopus_shell::served_files`. A link the user opens in a new tab, and a page's new window, go to the UI,
//! which opens them as browser panels in the task's order of tabs.

use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use octopus_client::tunnel::Tunnel;
use octopus_proto::ops::BrowserReport;
use octopus_proto::types::{BrowserId, web_page};
use octopus_shell::hub::Hub;
use octopus_shell::served_files::ServedFiles;
use octopus_shell::task_lists::TaskRef;
use octopus_shell::ui::{Bounds, UiStore};
use octopus_shell::{emit, lock};
use serde::{Deserialize, Serialize};
use tauri::webview::{DownloadEvent, NewWindowResponse, WebviewBuilder};
use tauri::{AppHandle, Manager, Url, Webview, WebviewUrl};
use ts_rs::TS;

use crate::layers::Layers;

pub(crate) const PAGED_EVENT: &str = "browser_paged";
pub(crate) const FAILED_EVENT: &str = "browser_failed";
pub(crate) const DOWNLOADED_EVENT: &str = "browser_downloaded";
pub(crate) const OPENED_EVENT: &str = "browser_opened";
pub(crate) const HOVERED_EVENT: &str = "browser_hovered";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Go {
    Back,
    Forward,
    Reload,
    ReloadFromOrigin,
}

/// The page a browser panel's webview shows on this client, whenever it or its loading changes.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub(crate) struct Paged {
    /// The task's key.
    pub task: String,
    pub panel: BrowserId,
    pub url: String,
    pub loading: bool,
    /// Whether the panel's history goes back, and forward.
    pub back: bool,
    pub forward: bool,
    /// The port of the server's machine the page reaches through a tunnel, when it does.
    pub tunnel: Option<u16>,
}

/// A page a browser panel's webview could not load.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Failed {
    /// The task's key.
    pub task: String,
    pub panel: BrowserId,
    pub url: String,
    pub error: String,
}

/// A page a browser panel's page opens in a new tab: a link the user opened there, or a new window the page asked for.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Opened {
    /// The task's key.
    pub task: String,
    /// The panel whose page opened it.
    pub panel: BrowserId,
    pub url: String,
    /// Whether the new tab takes the selection, else it opens behind the selected one.
    pub select: bool,
}

/// The link under the pointer in the page on screen, and the page's rect in the UI's CSS pixels.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub(crate) struct Hovered {
    pub link: String,
    pub page: Bounds,
}

/// What a webview tells of the page it shows.
enum Change {
    Seen {
        url: Option<String>,
        loading: bool,
        back: bool,
        forward: bool,
    },
    Failed {
        url: String,
        error: String,
    },
    #[cfg(target_os = "macos")]
    Opened {
        url: String,
        select: bool,
    },
    Hovered {
        link: Option<String>,
    },
}

/// A download a page started, finished in this machine's Downloads folder.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub(crate) struct Downloaded {
    pub path: String,
    pub success: bool,
}

/// A browser panel anywhere in the client: `<server>/<task>/<panel>`.
fn panel_key(at: &TaskRef, panel: &BrowserId) -> String {
    format!("{}/{panel}", at.key())
}

/// The browser panel the page layer shows, at its rect.
#[derive(Debug, Clone, PartialEq, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct OnScreen {
    pub at: TaskRef,
    pub panel: BrowserId,
    pub bounds: Bounds,
}

/// The page a webview shows, and the last one it reported. A load that fails leaves the webview on the page it
/// reported before, which the server already moved past.
#[derive(Default)]
struct Page {
    url: Option<Url>,
    title: String,
    reported: Option<(String, String)>,
}

struct Rendered {
    webview: Webview,
    /// The panel's `load` the webview last loaded.
    load: u64,
}

/// One listening tunnel, held while any panel has shown a page on its port.
struct Held {
    server: String,
    panels: BTreeSet<String>,
    _tunnel: Tunnel,
}

pub(crate) struct Browsers {
    app: AppHandle,
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
    pub fn new(app: AppHandle, layers: Arc<Layers>) -> Self {
        Self {
            app,
            layers,
            next: AtomicU64::new(0),
            rendered: Mutex::default(),
            tunnels: tokio::sync::Mutex::default(),
        }
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

    /// Loads the panel's `url` into its webview, created hidden and blank on its first page so that it is watched
    /// before it loads, unless the webview already loaded this `load`; `show_page` puts it on screen.
    pub async fn load(&self, at: &TaskRef, panel: &BrowserId, url: &str, load: u64) -> Result<(), String> {
        let key = panel_key(at, panel);
        if lock(&self.rendered).get(&key).is_some_and(|rendered| rendered.load == load) {
            return Ok(());
        }
        let page = web_page(url)?;
        self.hold_tunnel(&at.server, &key, &page).await?;
        let loaded = self.app.state::<Arc<ServedFiles>>().loaded_from(&at.server, &page)?;
        let mut rendered = lock(&self.rendered);
        let rendered = match rendered.entry(key) {
            Entry::Occupied(existing) => existing.into_mut(),
            Entry::Vacant(first) => first.insert(Rendered { webview: self.create(at, panel)?, load }),
        };
        rendered.load = load;
        rendered.webview.navigate(loaded).map_err(failed)
    }

    fn create(&self, at: &TaskRef, panel: &BrowserId) -> Result<Webview, String> {
        let app = &self.app;
        let label = format!("browser-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let blank = WebviewUrl::External(Url::parse("about:blank").expect("about:blank is a URL"));
        let page = Arc::new(Mutex::new(Page::default()));
        let titled = Arc::clone(&page);
        let (app_for_title, at_for_title, panel_for_title) = (app.clone(), at.clone(), panel.clone());
        let (app_for_window, at_for_window, panel_for_window) = (app.clone(), at.clone(), panel.clone());
        let (app_for_navigation, at_for_navigation, key) = (app.clone(), at.clone(), panel_key(at, panel));
        let downloading = Mutex::new(HashMap::<Url, std::path::PathBuf>::new());
        let builder = WebviewBuilder::new(label, blank)
            .on_navigation(move |url| {
                hold_while_navigating(&app_for_navigation, &at_for_navigation, &key, url);
                true
            })
            .on_document_title_changed(move |_, title| {
                lock(&titled).title = title;
                report(&app_for_title, &at_for_title, &panel_for_title, &titled);
            })
            .on_new_window(move |url, _| {
                open_tab(&app_for_window, &at_for_window, &panel_for_window, &url, true);
                NewWindowResponse::Deny
            })
            .on_download(move |webview, event| {
                match event {
                    DownloadEvent::Requested { url, destination } => {
                        lock(&downloading).insert(url, destination.clone());
                    }
                    DownloadEvent::Finished { url, path, success } => {
                        let path = path.or_else(|| lock(&downloading).remove(&url)).unwrap_or_default();
                        let path = path.display().to_string();
                        emit(webview.app_handle(), DOWNLOADED_EVENT, Downloaded { path, success });
                    }
                    _ => {}
                }
                true
            });
        let webview = self.layers.add_page(builder).map_err(failed)?;
        let (app, at, panel, layers) = (app.clone(), at.clone(), panel.clone(), Arc::clone(&self.layers));
        native::watch(&webview, move |change| match change {
            Change::Seen { url, loading, back, forward } => {
                let url = url.as_deref().and_then(|url| Url::parse(url).ok()).filter(|url| url.scheme() != "about");
                let finished = !loading && url.is_some();
                lock(&page).url.clone_from(&url);
                let shown = url.as_ref().map(|url| page_of(&app, url));
                let hub = app.state::<Arc<Hub>>();
                let tunnel = shown.as_ref().and_then(|shown| hub.tunneled_port(&at.server, shown));
                let url = shown.map(String::from).unwrap_or_default();
                let paged = Paged { task: at.key(), panel: panel.clone(), url, loading, back, forward, tunnel };
                emit(&app, PAGED_EVENT, paged);
                if finished {
                    report(&app, &at, &panel, &page);
                }
            }
            Change::Failed { url, error } => {
                let url = Url::parse(&url).map_or(url, |url| page_of(&app, &url).to_string());
                emit(&app, FAILED_EVENT, Failed { task: at.key(), panel: panel.clone(), url, error });
            }
            #[cfg(target_os = "macos")]
            Change::Opened { url, select } => {
                if let Ok(url) = Url::parse(&url) {
                    open_tab(&app, &at, &panel, &url, select);
                }
            }
            Change::Hovered { link } => {
                let link = link.as_deref().and_then(|link| Url::parse(link).ok()).map(|link| page_of(&app, &link));
                let hovered = link.zip(layers.page_bounds()).map(|(link, page)| Hovered { link: link.into(), page });
                emit(&app, HOVERED_EVENT, hovered);
            }
        })
        .map_err(failed)?;
        Ok(webview)
    }

    /// Holds the tunnel to the page's port for the panel `key`, when the page is on a loopback host of a server on
    /// another machine.
    async fn hold_tunnel(&self, server: &str, key: &str, page: &Url) -> Result<(), String> {
        let hub = self.app.state::<Arc<Hub>>();
        let Some(port) = hub.tunneled_port(server, page) else { return Ok(()) };
        let mut tunnels = self.tunnels.lock().await;
        match tunnels.get_mut(&port) {
            Some(held) if held.server != server => {
                Err(format!("this client's port {port} already tunnels to {}", hub.label(&held.server)))
            }
            Some(held) => {
                held.panels.insert(key.to_string());
                Ok(())
            }
            None => {
                let tunnel = hub.tunnel(server, port).await?;
                let panels = BTreeSet::from([key.to_string()]);
                tunnels.insert(port, Held { server: server.to_string(), panels, _tunnel: tunnel });
                Ok(())
            }
        }
    }

    pub(crate) fn show_page(&self, on_screen: Option<OnScreen>) -> Result<(), String> {
        let next = on_screen
            .map(|OnScreen { at, panel, bounds }| self.with(&at, &panel, |webview| Ok((webview.clone(), bounds))))
            .transpose()?;
        emit(&self.app, HOVERED_EVENT, None::<Hovered>);
        self.layers.show_page(next).map_err(failed)
    }

    pub fn go(&self, at: &TaskRef, panel: &BrowserId, go: Go) -> Result<(), String> {
        self.with(at, panel, |webview| native::go(webview, go))
    }

    /// Selects the next match of `text` in the page, or the one before when `backwards`, wrapping around its ends.
    pub fn find(&self, at: &TaskRef, panel: &BrowserId, text: String, backwards: bool) -> Result<(), String> {
        self.with(at, panel, |webview| native::find(webview, text, backwards))
    }

    /// Closes the panel's webview on this client, and every tunnel only it held.
    pub async fn forget(&self, at: &TaskRef, panel: &BrowserId) -> Result<(), String> {
        let key = panel_key(at, panel);
        let mut tunnels = self.tunnels.lock().await;
        for held in tunnels.values_mut() {
            held.panels.remove(&key);
        }
        tunnels.retain(|_, held| !held.panels.is_empty());
        let rendered = lock(&self.rendered).remove(&key).ok_or_else(|| format!("no browser panel {key}"))?;
        native::unwatch(&rendered.webview).map_err(failed)?;
        self.layers.close_page(&rendered.webview).map_err(failed)
    }
}

/// A frame of the page navigating to a loopback port of a server on another machine reaches it through a tunnel
/// held before the navigation goes on.
fn hold_while_navigating(app: &AppHandle, at: &TaskRef, key: &str, url: &Url) {
    let held = tauri::async_runtime::block_on(app.state::<Browsers>().hold_tunnel(&at.server, key, url));
    if let Err(problem) = held {
        app.state::<Arc<UiStore>>().report(problem);
    }
}

/// Records on the server each new page or title the webview shows.
fn report(app: &AppHandle, at: &TaskRef, panel: &BrowserId, page: &Mutex<Page>) {
    let (url, title) = {
        let mut page = lock(page);
        let Some(url) = &page.url else { return };
        let shown = (page_of(app, url).to_string(), page.title.clone());
        if page.reported.as_ref() == Some(&shown) {
            return;
        }
        page.reported = Some(shown.clone());
        shown
    };
    let hub = Arc::clone(app.state::<Arc<Hub>>().inner());
    let (at, panel) = (at.clone(), panel.clone());
    tauri::async_runtime::spawn(async move {
        let reported = hub.call(&at.server, &BrowserReport { task: at.task.clone(), panel, url, title }).await;
        if let Err(error) = reported {
            eprintln!("octopus app: report the page of a browser panel of {}: {error}", at.key());
        }
    });
}

/// The page a webview shows at `loaded`, as the server records it.
fn page_of(app: &AppHandle, loaded: &Url) -> Url {
    app.state::<Arc<ServedFiles>>().page_of(loaded)
}

fn open_tab(app: &AppHandle, at: &TaskRef, panel: &BrowserId, url: &Url, select: bool) {
    let url = page_of(app, url).to_string();
    emit(app, OPENED_EVENT, Opened { task: at.key(), panel: panel.clone(), url, select });
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
#[path = "browser/macos.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "browser/linux.rs"]
mod native;
pub(crate) use native::has_focus as page_has_focus;
