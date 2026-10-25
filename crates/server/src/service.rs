//! The terminal host: a process of its own that keeps every terminal, so terminals outlive the server process.
//!
//! Each connection to the host opens with one `HostRequest` line and gets one `Outcome` line back. An `attach`
//! connection then carries the terminal's scrollback and live output; an `input` connection carries bytes to it.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use octopus_proto::local::{self, BlockingStream, Listener};
use octopus_proto::types::PtyId;
use octopus_proto::{Error, Outcome, line};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::broadcast;

use crate::pty::{Attachment, Pty};
use crate::{lock, report};

/// Bumped whenever a request or reply changes shape; a server refuses a host that speaks another.
const PROTOCOL: u32 = 1;
const HOST_START: Duration = Duration::from_secs(5);
const HOST_START_POLL: Duration = Duration::from_millis(5);
const WAIT_RETRY: Duration = Duration::from_secs(1);
const INPUT_BYTES: usize = 4096;

#[derive(Debug, Clone)]
pub struct TerminalHost {
    /// An `octopus` binary: the host runs as `<program> server terminal-host --address <address>`.
    pub program: PathBuf,
    pub address: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum HostRequest {
    Spawn { id: PtyId, argv: Vec<String>, cwd: PathBuf, env: Vec<(String, String)> },
    List,
    Attach { id: PtyId },
    Input { id: PtyId },
    Resize { id: PtyId, cols: u16, rows: u16 },
    Terminate { id: PtyId },
    Forget { id: PtyId },
    WaitExit { id: PtyId },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Listing {
    protocol: u32,
    terminals: Vec<Hosted>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hosted {
    pub id: PtyId,
    pub running: bool,
}

type Terminals = Arc<Mutex<HashMap<PtyId, Arc<Pty>>>>;

/// What a connection carries once its request is answered.
enum Then {
    Close,
    Output(Attachment),
    Input(Arc<Pty>),
}

/// Serves terminals at `address` until the address is gone, ending every terminal on the way out.
pub async fn run_terminal_host(address: &Path) -> std::io::Result<()> {
    let listener = Listener::bind(address)?;
    let unreachable = platform::unreachable(address)?;
    let terminals = Terminals::default();
    let serving = Arc::clone(&terminals);
    tokio::spawn(local::accept_forever(listener, move |stream| {
        drop(tokio::spawn(serve(Arc::clone(&serving), stream)))
    }));
    unreachable.await;
    let terminals = lock(&terminals);
    report!("{} is gone; ending its {} terminals", address.display(), terminals.len());
    terminals.values().for_each(|pty| pty.kill());
    Ok(())
}

#[cfg(unix)]
mod platform {
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;
    use std::time::Duration;

    use crate::report;

    const CHECK: Duration = Duration::from_secs(1);

    fn identity(address: &Path) -> std::io::Result<(u64, u64)> {
        std::fs::metadata(address).map(|metadata| (metadata.dev(), metadata.ino()))
    }

    /// Resolves once the socket file the host bound is gone or replaced: nothing can reach the host again.
    pub fn unreachable(address: &Path) -> std::io::Result<impl Future<Output = ()> + use<'_>> {
        let bound = identity(address)?;
        Ok(async move {
            loop {
                tokio::time::sleep(CHECK).await;
                match identity(address) {
                    Ok(current) if current == bound => {}
                    Ok(_) => return,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                    Err(error) => report!("check {}: {error}", address.display()),
                }
            }
        })
    }
}

#[cfg(windows)]
mod platform {
    use std::path::Path;

    /// A named pipe lives exactly as long as the host that serves it.
    pub fn unreachable(_address: &Path) -> std::io::Result<impl Future<Output = ()>> {
        Ok(std::future::pending())
    }
}

fn find(terminals: &Terminals, id: &PtyId) -> Result<Arc<Pty>, Error> {
    lock(terminals).get(id).cloned().ok_or_else(|| Error::not_found(id))
}

async fn answer<R: AsyncRead + Unpin>(
    terminals: &Terminals,
    request: HostRequest,
    read: &mut R,
) -> Result<(Value, Then), Error> {
    match request {
        HostRequest::Spawn { id, argv, cwd, env } => {
            let already = || Error::conflict(format!("{id} is already hosted"));
            if lock(terminals).contains_key(&id) {
                return Err(already());
            }
            let pty =
                tokio::task::spawn_blocking(move || Pty::spawn(&argv, &cwd, &env)).await.map_err(Error::internal)??;
            let mut terminals = lock(terminals);
            if terminals.contains_key(&id) {
                tokio::spawn(pty.terminate());
                return Err(already());
            }
            terminals.insert(id, pty);
            Ok((Value::Null, Then::Close))
        }
        HostRequest::List => {
            let terminals =
                lock(terminals).iter().map(|(id, pty)| Hosted { id: id.clone(), running: pty.running() }).collect();
            let listing = serde_json::to_value(Listing { protocol: PROTOCOL, terminals }).map_err(Error::internal)?;
            Ok((listing, Then::Close))
        }
        HostRequest::Attach { id } => Ok((Value::Null, Then::Output(find(terminals, &id)?.attach()))),
        HostRequest::Input { id } => Ok((Value::Null, Then::Input(find(terminals, &id)?))),
        HostRequest::Resize { id, cols, rows } => {
            find(terminals, &id)?.resize(cols, rows)?;
            Ok((Value::Null, Then::Close))
        }
        HostRequest::Terminate { id } => {
            tokio::spawn(find(terminals, &id)?.terminate());
            Ok((Value::Null, Then::Close))
        }
        HostRequest::Forget { id } => {
            let pty = lock(terminals).remove(&id).ok_or_else(|| Error::not_found(&id))?;
            tokio::spawn(pty.terminate());
            Ok((Value::Null, Then::Close))
        }
        HostRequest::WaitExit { id } => {
            let pty = find(terminals, &id)?;
            let mut probe = [0u8; 1];
            tokio::select! {
                () = pty.exit() => {}
                // The read returns when the waiter hangs up, which frees this task.
                _ = read.read(&mut probe) => {}
            }
            Ok((Value::Null, Then::Close))
        }
    }
}

async fn serve<S: AsyncRead + AsyncWrite + Send + 'static>(terminals: Terminals, stream: S) {
    let (read, mut write) = tokio::io::split(stream);
    let mut read = BufReader::new(read);
    let mut first = String::new();
    if read.read_line(&mut first).await.unwrap_or(0) == 0 {
        return;
    }
    let answered = match serde_json::from_str::<HostRequest>(&first) {
        Ok(request) => answer(&terminals, request, &mut read).await,
        Err(error) => Err(Error::invalid(error)),
    };
    let (reply, then) = match answered {
        Ok((value, then)) => (Ok(value), then),
        Err(error) => (Err(error), Then::Close),
    };
    if write.write_all(line(&Outcome::from(reply)).as_bytes()).await.is_err() {
        return;
    }
    match then {
        Then::Close => {}
        Then::Output(attachment) => {
            let _peer_gone = stream_output(attachment, &mut write).await;
        }
        Then::Input(pty) => forward_input(pty, read).await,
    }
}

/// Ends an attachment that falls behind, so its client never sees output with a gap.
async fn stream_output<W: AsyncWrite + Unpin>(attachment: Attachment, write: &mut W) -> std::io::Result<()> {
    write.write_all(&attachment.scrollback).await?;
    if let Some(mut live) = attachment.live {
        loop {
            match live.recv().await {
                Ok(chunk) => write.write_all(&chunk).await?,
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    report!("closing an attachment {missed} chunks behind");
                    break;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
    write.shutdown().await
}

/// Writes happen off the runtime: a full PTY input buffer blocks them.
async fn forward_input<R: AsyncRead + Unpin>(pty: Arc<Pty>, mut read: R) {
    let mut buffer = [0u8; INPUT_BYTES];
    while let Ok(read) = read.read(&mut buffer).await
        && read > 0
    {
        let (pty, bytes) = (Arc::clone(&pty), buffer[..read].to_vec());
        match tokio::task::spawn_blocking(move || pty.write(&bytes)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return report!("terminal input: {error}"),
            Err(error) => return report!("terminal input panicked: {error}"),
        }
    }
}

pub struct HostClient {
    address: PathBuf,
}

fn host_error(address: &Path, error: impl std::fmt::Display) -> Error {
    Error::internal(format!("terminal host {}: {error}", address.display()))
}

/// Takes the host out of the server's process group, so a Ctrl-C or group signal aimed at the server spares it.
#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

impl HostClient {
    pub fn reach_or_start(host: &TerminalHost) -> Result<Self, Error> {
        let address = host.address.clone();
        if local::connect_blocking(&address).is_ok() {
            return Ok(Self { address });
        }
        let mut command = Command::new(&host.program);
        command.args(["server", "terminal-host", "--address"]).arg(&address);
        // stderr stays inherited, so the host's diagnostics land in the server's log.
        command.stdin(Stdio::null()).stdout(Stdio::null());
        detach(&mut command);
        let mut child = command.spawn().map_err(|error| host_error(&address, format!("start: {error}")))?;
        let deadline = Instant::now() + HOST_START;
        while local::connect_blocking(&address).is_err() {
            if let Some(status) = child.try_wait().map_err(|error| host_error(&address, error))? {
                return Err(host_error(&address, format!("exited on start with {status}")));
            }
            if Instant::now() > deadline {
                return Err(host_error(&address, format!("not answering {HOST_START:?} after start")));
            }
            std::thread::sleep(HOST_START_POLL);
        }
        std::thread::spawn(move || child.wait());
        Ok(Self { address })
    }

    fn failed(&self, error: impl std::fmt::Display) -> Error {
        host_error(&self.address, error)
    }

    fn open(&self, request: &HostRequest) -> Result<(BlockingStream, Value), Error> {
        let (stream, outcome) = local::open_blocking(&self.address, request).map_err(|error| self.failed(error))?;
        Ok((stream, Result::from(outcome)?))
    }

    fn call<T: DeserializeOwned>(&self, request: &HostRequest) -> Result<T, Error> {
        serde_json::from_value(self.open(request)?.1).map_err(|error| self.failed(error))
    }

    /// Reads the protocol before the rest, so a host of another version fails with its version, not its shape.
    pub fn list(&self) -> Result<Vec<Hosted>, Error> {
        let listing = self.open(&HostRequest::List)?.1;
        let protocol = listing.get("protocol").and_then(Value::as_u64);
        if protocol != Some(PROTOCOL.into()) {
            let found = protocol.map_or("no protocol".to_string(), |protocol| format!("protocol {protocol}"));
            return Err(self.failed(format!("speaks {found}, this server {PROTOCOL}; end the host to start over")));
        }
        let listing: Listing = serde_json::from_value(listing).map_err(|error| self.failed(error))?;
        Ok(listing.terminals)
    }

    /// The terminal's scrollback, then live bytes until it exits.
    pub async fn attach(&self, id: &PtyId) -> Result<local::Stream, Error> {
        let request = HostRequest::Attach { id: id.clone() };
        let (stream, outcome) = local::open(&self.address, &request).await.map_err(|error| self.failed(error))?;
        Result::from(outcome)?;
        Ok(stream)
    }

    /// Whether anything still answers at the host's address.
    fn reachable(&self) -> bool {
        local::connect_blocking(&self.address).is_ok()
    }

    pub fn spawn(&self, id: &PtyId, argv: Vec<String>, cwd: &Path, env: Vec<(String, String)>) -> Result<(), Error> {
        self.call(&HostRequest::Spawn { id: id.clone(), argv, cwd: cwd.into(), env })
    }

    pub fn forget(&self, id: &PtyId) -> Result<(), Error> {
        self.call(&HostRequest::Forget { id: id.clone() })
    }
}

/// The server's handle on a hosted terminal: the server holds the client write lock, the host holds the process
/// and its output.
pub struct Terminal {
    pub id: PtyId,
    host: Arc<HostClient>,
    running: AtomicBool,
    holder: Mutex<Option<String>>,
    input: Mutex<Option<BlockingStream>>,
}

impl Terminal {
    pub fn new(id: PtyId, host: Arc<HostClient>, running: bool) -> Self {
        Self { id, host, running: AtomicBool::new(running), holder: Mutex::new(None), input: Mutex::new(None) }
    }

    pub fn running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Blocks until the host reports the exit or stops answering; a failed wait on a host that still answers is
    /// asked again, so a passing failure never marks a live agent exited.
    pub fn wait_exit(&self) -> Result<(), Error> {
        let waited = loop {
            match self.host.call(&HostRequest::WaitExit { id: self.id.clone() }) {
                Err(error) if !matches!(error, Error::NotFound { .. }) && self.host.reachable() => {
                    report!("waiting on {}: {error}; asking again", self.id);
                    std::thread::sleep(WAIT_RETRY);
                }
                waited => break waited,
            }
        };
        self.running.store(false, Ordering::Relaxed);
        waited
    }

    pub fn holder(&self) -> Option<String> {
        lock(&self.holder).clone()
    }

    /// Gives `client` the lock when it is free, already theirs, or `steal` is set; reports whether the holder changed.
    pub fn claim(&self, client: &str, steal: bool) -> Result<bool, Error> {
        let mut holder = lock(&self.holder);
        match holder.as_deref() {
            Some(current) if current == client => Ok(false),
            Some(other) if !steal => Err(Error::PtyLocked { holder: other.to_string() }),
            _ => {
                *holder = Some(client.to_string());
                Ok(true)
            }
        }
    }

    pub fn release(&self, client: &str) -> bool {
        let mut holder = lock(&self.holder);
        let held = holder.as_deref() == Some(client);
        if held {
            *holder = None;
        }
        held
    }

    /// Writes only while `client` still holds the lock it claimed earlier.
    pub fn write_as(&self, client: &str, bytes: &[u8]) -> Result<(), Error> {
        match self.holder() {
            Some(holder) if holder == client => {}
            Some(holder) => return Err(Error::PtyLocked { holder }),
            None => return Err(Error::conflict(format!("{client} lost the lock on {} before writing", self.id))),
        }
        if !self.running() {
            return Err(Error::conflict(format!("{} has exited", self.id)));
        }
        let mut input = lock(&self.input);
        let mut stream = match input.take() {
            Some(stream) => stream,
            None => self.host.open(&HostRequest::Input { id: self.id.clone() })?.0,
        };
        stream.write_all(bytes).map_err(|error| Error::internal(format!("input to {}: {error}", self.id)))?;
        *input = Some(stream);
        Ok(())
    }

    pub fn resize(&self, client: &str, cols: u16, rows: u16) -> Result<(), Error> {
        if let Some(holder) = self.holder().filter(|holder| holder != client) {
            return Err(Error::PtyLocked { holder });
        }
        self.host.call(&HostRequest::Resize { id: self.id.clone(), cols, rows })
    }

    pub fn terminate(&self) -> Result<(), Error> {
        self.host.call(&HostRequest::Terminate { id: self.id.clone() })
    }

    /// Ends the terminal and drops its scrollback.
    pub fn forget(&self) -> Result<(), Error> {
        self.host.forget(&self.id)
    }

    pub async fn attach(&self) -> Result<local::Stream, Error> {
        self.host.attach(&self.id).await
    }
}
