//! A task's browser panels, and the tunnel that reaches a port on this machine for them.

use std::net::{Ipv4Addr, Ipv6Addr};

use octopus_proto::Error;
use octopus_proto::types::{BrowserId, BrowserPanel, BrowserTarget};
use tokio::net::TcpStream;

/// The panel `target` names among the task's `panels`, or a fresh one.
pub fn panel(panels: &[BrowserPanel], target: BrowserTarget) -> Result<BrowserId, Error> {
    match target {
        BrowserTarget::New => Ok(BrowserId::random()),
        BrowserTarget::Panel(id) if panels.iter().any(|known| known.panel == id) => Ok(id),
        BrowserTarget::Panel(id) => Err(Error::not_found(format!("browser panel {id}"))),
    }
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
