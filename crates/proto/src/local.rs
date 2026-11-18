//! The local socket between processes of one OS user.

use std::io::{self, BufRead, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::frames::{Op, Outcome, REPLY_BUFFER, Request, decode_reply, line};
use crate::ops::SessionPing;

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

/// A socket file whose server is gone refuses connections; anything else at the path is left alone.
fn clear_stale(address: &Path) -> io::Result<()> {
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

/// Whether a server serves `address`: it replies within `patience` to a ping, with the refusal a ping before
/// `session.hello` draws. A socket whose process is still starting takes the connection and replies only once it
/// serves.
pub fn answers(address: &Path, patience: Duration) -> bool {
    let args = serde_json::to_value(SessionPing {}).expect("ops serialize");
    let ping = Request { id: 0, op: SessionPing::NAME.to_string(), args };
    let replied = |mut stream: BlockingStream| -> io::Result<bool> {
        stream.set_read_timeout(Some(patience))?;
        stream.write_all(line(&ping).as_bytes())?;
        let mut reply = Vec::new();
        Ok(io::BufReader::new(stream).read_until(b'\n', &mut reply)? > 0)
    };
    connect_blocking(address).and_then(replied).unwrap_or(false)
}
