//! What every client does with the pages its browser panels show, whichever engine draws them. Each kept page
//! reports each page it shows and each title it takes to its server, in the order its engine told them. A page on a
//! loopback host reaches that port on its server's machine through a tunnel from the same port of this client,
//! held while a kept page is on that port; a page on another port lets its tunnel go once it loads.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use octopus_client::tunnel::Tunnel;
use octopus_proto::ops::BrowserReport;
use octopus_proto::types::{BrowserId, web_page};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Url};
use tokio::sync::mpsc;
use ts_rs::TS;

use crate::hub::Hub;
use crate::served_files::ServedFiles;
use crate::task_lists::TaskRef;
use crate::ui::Bounds;
use crate::webkit::Change;
use crate::{emit, lock};

pub const PAGED_EVENT: &str = "browser_paged";
pub const FAILED_EVENT: &str = "browser_failed";
pub const OPENED_EVENT: &str = "browser_opened";

/// The page a browser panel shows on this client, whenever it or its loading changes.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Paged {
    /// The task's key.
    pub task: String,
    pub panel: BrowserId,
    pub url: String,
    pub loading: bool,
    pub back: bool,
    pub forward: bool,
    /// The port of the server's machine the page reaches through a tunnel, when it does.
    pub tunnel: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Failed {
    pub task: String,
    pub panel: BrowserId,
    pub url: String,
    pub error: String,
}

/// A page a browser panel's page opens in a new tab, selected or behind the selected one.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Opened {
    pub task: String,
    pub panel: BrowserId,
    pub url: String,
    pub select: bool,
}

/// The browser panel the UI shows, at its rect.
#[derive(Debug, Clone, PartialEq, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct OnScreen {
    pub at: TaskRef,
    pub panel: BrowserId,
    pub bounds: Bounds,
}

/// A browser panel anywhere in the client: `<server>/<task>/<panel>`.
pub fn panel_key(at: &TaskRef, panel: &BrowserId) -> String {
    format!("{}/{panel}", at.key())
}

struct Kept {
    at: TaskRef,
    panel: BrowserId,
    url: Option<Url>,
    title: String,
}

struct Held {
    server: String,
    panels: BTreeSet<String>,
    _tunnel: Tunnel,
}

pub struct BrowserPages {
    app: AppHandle,
    kept: Mutex<HashMap<String, Kept>>,
    /// Keyed by port: this client's `localhost:<port>` reaches one server at a time.
    tunnels: tokio::sync::Mutex<HashMap<u16, Held>>,
    reports: mpsc::UnboundedSender<(TaskRef, BrowserReport)>,
}

impl BrowserPages {
    pub fn new(app: AppHandle) -> Self {
        let (reports, mut told) = mpsc::unbounded_channel::<(TaskRef, BrowserReport)>();
        let reporting = app.clone();
        tauri::async_runtime::spawn(async move {
            while let Some((at, report)) = told.recv().await {
                let hub = Arc::clone(reporting.state::<Arc<Hub>>().inner());
                if let Err(error) = hub.call(&at.server, &report).await {
                    eprintln!("octopus app: report the page of a browser panel of {}: {error}", at.key());
                }
            }
        });
        Self { app, kept: Mutex::default(), tunnels: tokio::sync::Mutex::default(), reports }
    }

    fn hub(&self) -> Arc<Hub> {
        Arc::clone(self.app.state::<Arc<Hub>>().inner())
    }

    fn page_of(&self, loaded: &str) -> Option<Url> {
        Url::parse(loaded).ok().map(|loaded| self.app.state::<Arc<ServedFiles>>().page_of(&loaded))
    }

    pub fn panel_of(&self, key: &str) -> Option<(TaskRef, BrowserId)> {
        lock(&self.kept).get(key).map(|kept| (kept.at.clone(), kept.panel.clone()))
    }

    /// Keeps the panel's page with the tunnel `url` needs held, and gives its key and the URL its engine loads.
    pub async fn load(&self, at: &TaskRef, panel: &BrowserId, url: &str) -> Result<(String, Url), String> {
        let key = panel_key(at, panel);
        let page = web_page(url)?;
        self.hold_tunnel(&at.server, &key, &page).await?;
        let loaded = self.app.state::<Arc<ServedFiles>>().loaded_from(&at.server, &page)?;
        lock(&self.kept).entry(key.clone()).or_insert_with(|| Kept {
            at: at.clone(),
            panel: panel.clone(),
            url: None,
            title: String::new(),
        });
        Ok((key, loaded))
    }

    /// Lets go of the panel's page, and of every tunnel only it held.
    pub async fn forget(&self, key: &str) {
        lock(&self.kept).remove(key);
        self.release_tunnels(key, None).await;
    }

    /// Whether the panel's page may navigate to `page`, with the tunnel a loopback page needs held first. A page
    /// whose tunnel fails shows why.
    pub async fn heading(&self, key: &str, page: &Url) -> bool {
        let Some((at, _)) = self.panel_of(key) else { return false };
        match self.hold_tunnel(&at.server, key, page).await {
            Ok(()) => true,
            Err(error) => {
                self.changed(key, Change::Failed { url: page.to_string(), error }).await;
                false
            }
        }
    }

    async fn hold_tunnel(&self, server: &str, key: &str, page: &Url) -> Result<(), String> {
        let hub = self.hub();
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

    async fn release_tunnels(&self, key: &str, keep: Option<u16>) {
        let mut tunnels = self.tunnels.lock().await;
        for (port, held) in tunnels.iter_mut() {
            if Some(*port) != keep {
                held.panels.remove(key);
            }
        }
        tunnels.retain(|_, held| !held.panels.is_empty());
    }

    /// What the engine tells of the panel's page, in the order it tells it; a change only one client draws is its own.
    pub async fn changed(&self, key: &str, change: Change) {
        let Some((at, panel)) = self.panel_of(key) else { return };
        let (app, task) = (&self.app, at.key());
        match change {
            Change::Seen { url, loading, back, forward } => {
                let page = url.as_deref().and_then(|url| self.page_of(url)).filter(|page| page.scheme() != "about");
                if let Some(kept) = lock(&self.kept).get_mut(key) {
                    kept.url.clone_from(&page);
                }
                let tunnel = page.as_ref().and_then(|page| self.hub().tunneled_port(&at.server, page));
                if !loading && page.is_some() {
                    self.release_tunnels(key, tunnel).await;
                    self.report(key);
                }
                let url = page.map(String::from).unwrap_or_default();
                emit(app, PAGED_EVENT, Paged { task, panel, url, loading, back, forward, tunnel });
            }
            Change::Titled(title) => {
                if let Some(kept) = lock(&self.kept).get_mut(key) {
                    kept.title = title;
                }
                self.report(key);
            }
            Change::Failed { url, error } => {
                let url = self.page_of(&url).map_or(url, String::from);
                emit(app, FAILED_EVENT, Failed { task, panel, url, error });
            }
            Change::Opened { url, select } => {
                if let Some(url) = self.page_of(&url) {
                    emit(app, OPENED_EVENT, Opened { task, panel, url: url.into(), select });
                }
            }
            Change::Hovered { .. } | Change::EdgeDragged { .. } | Change::Ended => {}
        }
    }

    fn report(&self, key: &str) {
        let (at, report) = {
            let kept = lock(&self.kept);
            let Some(Kept { at, panel, url: Some(url), title }) = kept.get(key) else { return };
            let report = BrowserReport {
                task: at.task.clone(),
                panel: panel.clone(),
                url: url.to_string(),
                title: title.clone(),
            };
            (at.clone(), report)
        };
        self.reports.send((at, report)).expect("the reporter runs for as long as the app");
    }
}
