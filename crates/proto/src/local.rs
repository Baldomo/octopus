//! The local socket between processes of one OS user: a Unix socket on macOS and Linux, a named pipe on Windows.

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::frames::{Outcome, REPLY_BUFFER, decode_reply, line};

pub use platform::{Accepted, BlockingStream, Listener, Stream, connect, connect_blocking};

const ACCEPT_RETRY: Duration = Duration::from_millis(100);

fn already_served(address: &Path) -> io::Error {
    io::Error::new(io::ErrorKind::AddrInUse, format!("{} is already served", address.display()))
}

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
pub async fn accept_forever(mut listener: Listener, mut serve: impl FnMut(Accepted)) {
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

#[cfg(unix)]
mod platform {
    use std::io;
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use std::path::Path;

    pub type Stream = tokio::net::UnixStream;
    pub type Accepted = tokio::net::UnixStream;
    pub type BlockingStream = std::os::unix::net::UnixStream;

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
            Ok(_) => Err(super::already_served(address)),
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

        pub async fn accept(&mut self) -> io::Result<Accepted> {
            Ok(self.0.accept().await?.0)
        }
    }

    pub async fn connect(address: &Path) -> io::Result<Stream> {
        Stream::connect(address).await
    }

    pub fn connect_blocking(address: &Path) -> io::Result<BlockingStream> {
        BlockingStream::connect(address)
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::OsString;
    use std::io;
    use std::path::Path;
    use std::time::{Duration, Instant};

    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions};

    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_PIPE_BUSY: i32 = 231;
    const BUSY_PATIENCE: Duration = Duration::from_secs(2);
    const BUSY_RETRY: Duration = Duration::from_millis(5);

    pub type Stream = NamedPipeClient;
    pub type Accepted = NamedPipeServer;
    pub type BlockingStream = std::fs::File;

    /// Keeps one instance waiting, so a client finds the pipe between accepts.
    pub struct Listener {
        address: OsString,
        waiting: NamedPipeServer,
    }

    fn instance(address: &OsString, first: bool) -> io::Result<NamedPipeServer> {
        ServerOptions::new().first_pipe_instance(first).reject_remote_clients(true).create(address)
    }

    impl Listener {
        pub fn bind(address: &Path) -> io::Result<Self> {
            let name = address.as_os_str().to_owned();
            match instance(&name, true) {
                Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED) => Err(super::already_served(address)),
                created => Ok(Self { waiting: created?, address: name }),
            }
        }

        /// A failed connect still leaves a fresh instance waiting.
        pub async fn accept(&mut self) -> io::Result<Accepted> {
            let connected = self.waiting.connect().await;
            let previous = std::mem::replace(&mut self.waiting, instance(&self.address, false)?);
            connected.map(|()| previous)
        }
    }

    /// A pipe is busy between an accept and the listener creating its next instance, so a retry soon succeeds.
    fn still_busy<T>(opened: &io::Result<T>, deadline: Instant) -> bool {
        matches!(opened, Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY)) && Instant::now() < deadline
    }

    pub async fn connect(address: &Path) -> io::Result<Stream> {
        let deadline = Instant::now() + BUSY_PATIENCE;
        loop {
            let opened = ClientOptions::new().open(address);
            if !still_busy(&opened, deadline) {
                return opened;
            }
            tokio::time::sleep(BUSY_RETRY).await;
        }
    }

    pub fn connect_blocking(address: &Path) -> io::Result<BlockingStream> {
        let deadline = Instant::now() + BUSY_PATIENCE;
        loop {
            let opened = std::fs::OpenOptions::new().read(true).write(true).open(address);
            if !still_busy(&opened, deadline) {
                return opened;
            }
            std::thread::sleep(BUSY_RETRY);
        }
    }
}
