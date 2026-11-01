//! A task's browser panels, and what the server reaches on its machine for them: a port through a tunnel, and the
//! files of a file page.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::{Component, Path};

use octopus_proto::Error;
use octopus_proto::types::{BrowserId, BrowserPanel, BrowserTarget};
use tokio::net::TcpStream;

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
pub async fn read_page(root: &str, path: &str) -> Result<Vec<u8>, Error> {
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
