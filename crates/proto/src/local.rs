//! The local socket between processes of one OS user.

use std::io::{self, BufRead, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::Path;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::frames::{Op, Outcome, REPLY_BUFFER, Request, Response, ServerFrame, decode_reply, line};
use crate::ops::{ServerVersion, SessionHello, Version, Welcome};

const ACCEPT_RETRY: Duration = Duration::from_millis(100);

pub type Stream = tokio::net::UnixStream;
pub type BlockingStream = std::os::unix::net::UnixStream;

/// Connects, sends `header` as one line, and reads the one `Outcome` line that answers it.
pub async fn open(address: &Path, header: &impl Serialize) -> io::Result<(Stream, Outcome)> {
    let mut stream = connect(address).await?;
    let outcome = crate::frames::open(&mut stream, header).await?;
    Ok((stream, outcome))
}

pub fn open_blocking(address: &Path, header: &impl Serialize) -> io::Result<(BlockingStream, Outcome)> {
    let mut stream = connect_blocking(address)?;
    stream.write_all(line(header).as_bytes())?;
    let mut reply = Vec::new();
    io::BufReader::with_capacity(REPLY_BUFFER, &mut stream).read_until(b'\n', &mut reply)?;
    Ok((stream, decode_reply(&reply)?))
}

/// Hands every connection to `serve`; a failed accept is one peer's failure, so it is reported and the
/// listener keeps going.
pub async fn accept_forever(mut listener: Listener, mut serve: impl FnMut(Stream)) {
    loop {
        match listener.accept().await {
            Ok(stream) => serve(stream),
            Err(error) => {
                let _stderr_closed = writeln!(io::stderr(), "octopus: accept: {error}");
                tokio::time::sleep(ACCEPT_RETRY).await;
            }
        }
    }
}

pub struct Listener(tokio::net::UnixListener);

/// Fails while a server serves `address`. A socket file whose server is gone refuses connections and goes;
/// anything else at the path is left alone.
pub fn clear_stale(address: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(address) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
        Ok(metadata) if !metadata.file_type().is_socket() => {
            let message = format!("{} exists and is not a socket", address.display());
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, message));
        }
        Ok(_) => {}
    }
    match connect_blocking(address) {
        Ok(_) => Err(io::Error::new(io::ErrorKind::AddrInUse, format!("{} is already served", address.display()))),
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => std::fs::remove_file(address),
        Err(error) => Err(error),
    }
}

impl Listener {
    /// Admits only the owning user.
    pub fn bind(address: &Path) -> io::Result<Self> {
        clear_stale(address)?;
        let listener = tokio::net::UnixListener::bind(address)?;
        std::fs::set_permissions(address, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self(listener))
    }

    pub async fn accept(&mut self) -> io::Result<Stream> {
        Ok(self.0.accept().await?.0)
    }
}

pub async fn connect(address: &Path) -> io::Result<Stream> {
    Stream::connect(address).await
}

pub fn connect_blocking(address: &Path) -> io::Result<BlockingStream> {
    BlockingStream::connect(address)
}

/// What the server serving a socket says of itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub server: String,
    /// None from a server that predates `server.version`.
    pub version: Option<String>,
}

fn request<O: Op + Serialize>(id: u64, op: &O) -> String {
    line(&Request { id, op: O::NAME.to_string(), args: serde_json::to_value(op).expect("ops serialize") })
}

/// Asks the server serving `address` its name and version, as `client`; None when nothing there answers both
/// within `patience`, or answers with something other than replies and pushes. A socket whose process is still
/// starting takes the connection and replies only once it serves.
pub fn ask(address: &Path, client: &str, patience: Duration) -> Option<Answer> {
    let deadline = Instant::now() + patience;
    let asked = |mut stream: BlockingStream| -> io::Result<Option<Answer>> {
        let hello = request(0, &SessionHello { client: client.to_string() });
        stream.write_all((hello + &request(1, &ServerVersion {})).as_bytes())?;
        let mut reader = io::BufReader::new(stream);
        let mut server = None;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            reader.get_ref().set_read_timeout(Some(left))?;
            let mut read = String::new();
            if reader.read_line(&mut read)? == 0 {
                return Ok(None);
            }
            let outcome = match serde_json::from_str::<ServerFrame>(&read) {
                Ok(ServerFrame::Response(Response { id: 0, outcome: Outcome::Ok(welcome) })) => {
                    server = serde_json::from_value::<Welcome>(welcome).ok().map(|welcome| welcome.server);
                    continue;
                }
                Ok(ServerFrame::Response(Response { id: 1, outcome })) => outcome,
                Ok(ServerFrame::Push(_)) => continue,
                _ => return Ok(None),
            };
            let version = match outcome {
                Outcome::Ok(version) => serde_json::from_value::<Version>(version).ok().map(|version| version.version),
                Outcome::Err(_) => None,
            };
            return Ok(server.map(|server| Answer { server, version }));
        }
    };
    connect_blocking(address).and_then(asked).ok().flatten()
}
