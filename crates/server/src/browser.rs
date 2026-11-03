//! A task's browser panels, and what the server reaches on its machine for them: a port through a tunnel, and the
//! files of a file page.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::{Component, Path};

use octopus_proto::Error;
use octopus_proto::ops::{BrowserClose, BrowserMove, BrowserOpen, BrowserReport, Revised, Shown};
use octopus_proto::types::{BrowserId, BrowserPanel, BrowserTarget};
use octopus_proto::types::{TaskId, web_page};
use tokio::net::TcpStream;

use crate::{Server, State, lock};

/// The panel `target` names among the task's `panels` once it shows `url` once more, or a fresh panel showing it, or
/// holding it unshown in the background.
pub fn opened(panels: &[BrowserPanel], target: BrowserTarget, url: String) -> Result<BrowserPanel, Error> {
    let title = String::new();
    match target {
        BrowserTarget::New => Ok(BrowserPanel { panel: BrowserId::random(), url, title, load: 1 }),
        BrowserTarget::Background => Ok(BrowserPanel { panel: BrowserId::random(), url, title, load: 0 }),
        BrowserTarget::Panel(id) => {
            let known = find(panels, &id)?;
            Ok(BrowserPanel { panel: id, url, title, load: known.load + 1 })
        }
    }
}

/// The task's `panels` with the panel `id` taken out and put back at index `to`.
pub fn moved(panels: &[BrowserPanel], id: &BrowserId, to: u16) -> Result<Vec<BrowserPanel>, Error> {
    let at = panels.iter().position(|known| known.panel == *id).ok_or_else(|| missing_panel(id))?;
    if usize::from(to) >= panels.len() {
        return Err(Error::invalid(format!("index {to} is past the last of {} browser panels", panels.len())));
    }
    let mut order = panels.to_vec();
    let panel = order.remove(at);
    order.insert(usize::from(to), panel);
    Ok(order)
}

fn missing_panel(id: &BrowserId) -> Error {
    Error::not_found(format!("browser panel {id}"))
}

pub fn find<'a>(panels: &'a [BrowserPanel], id: &BrowserId) -> Result<&'a BrowserPanel, Error> {
    panels.iter().find(|known| known.panel == *id).ok_or_else(|| missing_panel(id))
}

/// Dials the port on this machine's loopback, where a dev server listens for the browser panel: 127.0.0.1, then
/// ::1 for a dev server that bound `localhost` to IPv6 alone.
pub async fn dial(port: u16) -> Result<TcpStream, Error> {
    match TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await {
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
            TcpStream::connect((Ipv6Addr::LOCALHOST, port))
                .await
                .map_err(|_| Error::not_found(format!("a listener on 127.0.0.1:{port} or [::1]:{port}")))
        }
        dialed => dialed.map_err(|error| Error::internal(format!("dial 127.0.0.1:{port}: {error}"))),
    }
}

fn missing(path: &Path, error: &std::io::Error) -> Error {
    match error.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::IsADirectory => Error::not_found(path.display()),
        _ => Error::internal(format!("read {}: {error}", path.display())),
    }
}

/// The bytes of the file `path` names under a file page's `root`, which neither `..` nor a symlink leads out of.
pub(crate) async fn read_page(root: &str, path: &str) -> Result<Vec<u8>, Error> {
    let (root, relative) = (Path::new(root), Path::new(path));
    if !root.is_absolute() || !relative.components().all(|part| matches!(part, Component::Normal(_))) {
        return Err(Error::invalid(format!("{path} is no path under the absolute directory {}", root.display())));
    }
    let root = tokio::fs::canonicalize(root).await.map_err(|error| missing(root, &error))?;
    let wanted = root.join(relative);
    let file = tokio::fs::canonicalize(&wanted).await.map_err(|error| missing(&wanted, &error))?;
    if !file.starts_with(&root) {
        return Err(Error::invalid(format!("{} leads out of {}", wanted.display(), root.display())));
    }
    tokio::fs::read(&file).await.map_err(|error| missing(&wanted, &error))
}

impl State {
    pub(crate) fn browsers_of(&self, task: &TaskId) -> &[BrowserPanel] {
        self.browsers.get(task).map(Vec::as_slice).unwrap_or_default()
    }
}

impl Server {
    pub(crate) fn browser_open(&self, op: BrowserOpen) -> Result<Shown, Error> {
        web_page(&op.url).map_err(Error::invalid)?;
        let mut state = lock(&self.state);
        state.store.task(&op.task)?;
        let opened = opened(state.browsers_of(&op.task), op.panel, op.url)?;
        let panel = opened.panel.clone();
        let rev = self.save_browser(&mut state, &op.task, opened)?;
        Ok(Shown { panel, rev })
    }

    pub(crate) fn browser_report(&self, op: BrowserReport) -> Result<Revised, Error> {
        web_page(&op.url).map_err(Error::invalid)?;
        let mut state = lock(&self.state);
        let known = find(state.browsers_of(&op.task), &op.panel)?;
        if known.url == op.url && known.title == op.title {
            return Ok(Revised { rev: state.store.task(&op.task)?.rev });
        }
        let reported = BrowserPanel { panel: op.panel, url: op.url, title: op.title, load: known.load };
        Ok(Revised { rev: self.save_browser(&mut state, &op.task, reported)? })
    }

    pub(crate) fn browser_close(&self, op: BrowserClose) -> Result<Revised, Error> {
        let mut state = lock(&self.state);
        find(state.browsers_of(&op.task), &op.panel)?;
        let row = state.store.close_browser(&op.task, &op.panel)?;
        state.browsers.entry(op.task).or_default().retain(|known| known.panel != op.panel);
        self.publish_task(&state, &row);
        Ok(Revised { rev: row.rev })
    }

    pub(crate) fn browser_move(&self, op: BrowserMove) -> Result<Revised, Error> {
        let mut state = lock(&self.state);
        let panels = state.browsers_of(&op.task);
        let order = moved(panels, &op.panel, op.to)?;
        if order == panels {
            return Ok(Revised { rev: state.store.task(&op.task)?.rev });
        }
        let row = state.store.order_browsers(&op.task, &order)?;
        state.browsers.insert(op.task, order);
        self.publish_task(&state, &row);
        Ok(Revised { rev: row.rev })
    }

    fn save_browser(&self, state: &mut State, task: &TaskId, saved: BrowserPanel) -> Result<u64, Error> {
        let row = state.store.save_browser(task, &saved)?;
        let panels = state.browsers.entry(task.clone()).or_default();
        match panels.iter_mut().find(|known| known.panel == saved.panel) {
            Some(known) => *known = saved,
            None => panels.push(saved),
        }
        self.publish_task(state, &row);
        Ok(row.rev)
    }
}
