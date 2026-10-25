//! A browser panel's tunnel: this client listens on its own `localhost:<port>` and carries each connection to the
//! server over data stream `tunnel:<port>`, and the server dials the same port on its machine. The page keeps its
//! origin, so cookies, websockets and devtools behave as they would against a dev server on this machine.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use octopus_proto::types::Stream;
use tokio::net::{TcpListener, TcpStream};

use crate::{Connection, Task};

const ACCEPT_RETRY: Duration = Duration::from_millis(100);

/// The server's connection at the moment a browser connects, which a reconnect may have replaced.
pub type Dial = Arc<dyn Fn() -> Result<Arc<Connection>, String> + Send + Sync>;

/// Listens while it lives; connections already carried finish on their own.
pub struct Tunnel {
    _accepting: Vec<Task>,
}

/// Binds `port` on every address `localhost` resolves to, as a browser may try any of them.
pub async fn listen_on_localhost(port: u16) -> io::Result<Vec<TcpListener>> {
    let mut listeners = Vec::new();
    for address in tokio::net::lookup_host(("localhost", port)).await? {
        let listener = TcpListener::bind(address).await.map_err(|error| {
            io::Error::new(error.kind(), format!("listen on {address} for the tunnel to port {port}: {error}"))
        })?;
        listeners.push(listener);
    }
    Ok(listeners)
}

async fn carry(port: u16, mut browser: TcpStream, dial: Dial) -> Result<(), String> {
    let connection = dial()?;
    let mut stream = connection.open_data_stream(&Stream::Tunnel(port)).await.map_err(|error| error.to_string())?;
    tokio::io::copy_bidirectional(&mut browser, &mut stream).await.map(drop).map_err(|error| error.to_string())
}

impl Tunnel {
    /// Carries every connection `listeners` accept to `port` on the server.
    pub fn new(port: u16, listeners: Vec<TcpListener>, dial: Dial) -> Self {
        let accepting = listeners.into_iter().map(|listener| {
            let dial = Arc::clone(&dial);
            Task(tokio::spawn(async move {
                loop {
                    match listener.accept().await {
                        Ok((browser, _)) => {
                            let dial = Arc::clone(&dial);
                            tokio::spawn(async move {
                                if let Err(error) = carry(port, browser, dial).await {
                                    eprintln!("octopus: tunnel to port {port}: {error}");
                                }
                            });
                        }
                        Err(error) => {
                            eprintln!("octopus: tunnel to port {port}: accept: {error}");
                            tokio::time::sleep(ACCEPT_RETRY).await;
                        }
                    }
                }
            }))
        });
        Self { _accepting: accepting.collect() }
    }
}
