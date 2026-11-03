//! The terminal host: a process of its own that keeps every terminal, so terminals outlive the server process.
//!
//! Each connection to the host opens with one `HostRequest` line and gets one `Outcome` line back. An `attach`
//! connection then carries the terminal's pieces, its screen and then its live output; an `input` connection carries
//! bytes to it.

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use octopus_proto::local::{self, BlockingStream, Listener};
use octopus_proto::term::Piece;
use octopus_proto::types::PtyId;
use octopus_proto::{Error, Outcome, line};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::broadcast;

use crate::client::Client;
use crate::pty::{Activity, Attachment, INITIAL_SIZE, Pty};
use crate::{lock, now, report};

/// Bumped whenever an existing request or reply changes shape; a server refuses a host that speaks another. A host
/// older than a request added since refuses that request.
const PROTOCOL: u32 = 3;
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
    Activity { id: PtyId },
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
    let unreachable = unreachable(address)?;
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

const UNREACHABLE_CHECK: Duration = Duration::from_secs(1);

fn identity(address: &Path) -> std::io::Result<(u64, u64)> {
    std::fs::metadata(address).map(|metadata| (metadata.dev(), metadata.ino()))
}

/// Resolves once the socket file the host bound is gone or replaced: nothing can reach the host again.
fn unreachable(address: &Path) -> std::io::Result<impl Future<Output = ()> + use<'_>> {
    let bound = identity(address)?;
    Ok(async move {
        loop {
            tokio::time::sleep(UNREACHABLE_CHECK).await;
            match identity(address) {
                Ok(current) if current == bound => {}
                Ok(_) => return,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                Err(error) => report!("check {}: {error}", address.display()),
            }
        }
    })
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
        HostRequest::Activity { id } => {
            let pty = find(terminals, &id)?;
            let activity = tokio::task::spawn_blocking(move || pty.activity()).await.map_err(Error::internal)??;
            Ok((serde_json::to_value(activity).map_err(Error::internal)?, Then::Close))
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
    Piece::Screen(attachment.screen).write_to(write).await?;
    if let Some(mut live) = attachment.live {
        loop {
            match live.recv().await {
                Ok(piece) => piece.write_to(write).await?,
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    report!("closing an attachment {missed} pieces behind");
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
        // Out of the server's process group, so a Ctrl-C or group signal aimed at the server spares the host.
        command.process_group(0);
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

    /// The terminal's screen, then its live pieces until it exits.
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
    screen: Mutex<Screen>,
    input: Mutex<Option<BlockingStream>>,
}

/// The lock holder and the size it set change together, so every client sees a size its holder chose. `viewers`
/// counts each client's open data streams of the terminal, and `shows` how many it ever opened, so a grace that
/// started before a client showed the terminal again frees nothing. `unshown_at` is when the last viewer left, or
/// when the server took the terminal.
#[derive(Clone)]
pub struct Screen {
    pub holder: Option<Client>,
    pub cols: u16,
    pub rows: u16,
    viewers: HashMap<Client, usize>,
    shows: HashMap<Client, u64>,
    unshown_at: i64,
}

/// Why a client may not act through a terminal's lock: another client holds it, no client does, or the act failed.
#[derive(Debug)]
pub enum Refusal {
    HeldBy(Client),
    Free,
    Failed(Error),
}

impl From<Error> for Refusal {
    fn from(error: Error) -> Self {
        Self::Failed(error)
    }
}

impl Screen {
    fn check_holder(&self, client: &Client) -> Result<(), Refusal> {
        match &self.holder {
            Some(holder) if holder == client => Ok(()),
            Some(holder) => Err(Refusal::HeldBy(holder.clone())),
            None => Err(Refusal::Free),
        }
    }
}

impl Terminal {
    pub fn new(id: PtyId, host: Arc<HostClient>, running: bool) -> Self {
        let (cols, rows) = INITIAL_SIZE;
        let screen = Mutex::new(Screen {
            holder: None,
            cols,
            rows,
            viewers: HashMap::new(),
            shows: HashMap::new(),
            unshown_at: now(),
        });
        Self { id, host, running: AtomicBool::new(running), screen, input: Mutex::new(None) }
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

    pub fn screen(&self) -> Screen {
        lock(&self.screen).clone()
    }

    /// Gives `client` a free lock sized to `cols`×`rows`, or resizes the one it holds; reports whether the screen
    /// changed.
    pub fn claim(&self, client: &Client, cols: u16, rows: u16) -> Result<bool, Refusal> {
        let mut screen = lock(&self.screen);
        if let Some(other) = screen.holder.as_ref().filter(|holder| *holder != client) {
            return Err(Refusal::HeldBy(other.clone()));
        }
        let resized = self.set_size(&mut screen, cols, rows)?;
        let moved = screen.holder.is_none();
        screen.holder = Some(client.clone());
        Ok(resized || moved)
    }

    pub fn show(&self, client: &Client) {
        let mut screen = lock(&self.screen);
        *screen.viewers.entry(client.clone()).or_default() += 1;
        *screen.shows.entry(client.clone()).or_default() += 1;
    }

    /// How many times `client` has shown the terminal: a grace checks it is still the count it started at.
    pub fn shows(&self, client: &Client) -> u64 {
        lock(&self.screen).shows.get(client).copied().unwrap_or(0)
    }

    pub fn unshow(&self, client: &Client) {
        let mut screen = lock(&self.screen);
        if let Some(count) = screen.viewers.get_mut(client) {
            *count -= 1;
            if *count == 0 {
                screen.viewers.remove(client);
            }
        }
        if screen.viewers.is_empty() {
            screen.unshown_at = now();
        }
    }

    /// Since when no client shows the terminal, or `None` while one does.
    pub fn unshown_since(&self) -> Option<i64> {
        let screen = lock(&self.screen);
        screen.viewers.is_empty().then_some(screen.unshown_at)
    }

    pub fn holds(&self, client: &Client) -> bool {
        lock(&self.screen).holder.as_ref() == Some(client)
    }

    /// Whether `client` holds the lock and shows the terminal nowhere.
    pub fn held_unshown(&self, client: &Client) -> bool {
        let screen = lock(&self.screen);
        screen.holder.as_ref() == Some(client) && !screen.viewers.contains_key(client)
    }

    /// Frees the lock when `client` holds it, shows the terminal nowhere, and has not shown it again since `shows`
    /// was read; reports whether it did.
    pub fn free_if_unshown(&self, client: &Client, shows: u64) -> bool {
        let mut screen = lock(&self.screen);
        let unshown = screen.holder.as_ref() == Some(client)
            && !screen.viewers.contains_key(client)
            && screen.shows.get(client).copied().unwrap_or(0) == shows;
        if unshown {
            screen.holder = None;
        }
        unshown
    }

    /// Frees the lock when `client` holds it, however it shows the terminal; reports whether it did.
    pub fn free(&self, client: &Client) -> bool {
        let mut screen = lock(&self.screen);
        let held = screen.holder.as_ref() == Some(client);
        if held {
            screen.holder = None;
        }
        held
    }

    /// Gives `client` the lock from whoever holds it, sized to `cols`×`rows`; reports whether the screen changed.
    pub fn take(&self, client: &Client, cols: u16, rows: u16) -> Result<bool, Refusal> {
        let mut screen = lock(&self.screen);
        let resized = self.set_size(&mut screen, cols, rows)?;
        let moved = screen.holder.as_ref() != Some(client);
        screen.holder = Some(client.clone());
        Ok(resized || moved)
    }

    fn set_size(&self, screen: &mut Screen, cols: u16, rows: u16) -> Result<bool, Error> {
        if cols == 0 || rows == 0 {
            return Err(Error::invalid(format!("a terminal is at least 1×1, not {cols}×{rows}")));
        }
        if (screen.cols, screen.rows) == (cols, rows) {
            return Ok(false);
        }
        self.host.call::<()>(&HostRequest::Resize { id: self.id.clone(), cols, rows })?;
        (screen.cols, screen.rows) = (cols, rows);
        Ok(true)
    }

    pub fn check_holder(&self, client: &Client) -> Result<(), Refusal> {
        lock(&self.screen).check_holder(client)
    }

    /// Writes only while `client` still holds the lock it claimed earlier.
    pub fn write_as(&self, client: &Client, bytes: &[u8]) -> Result<(), Refusal> {
        self.check_holder(client)?;
        if !self.running() {
            return Err(Error::conflict(format!("{} has exited", self.id)).into());
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

    /// Resizes for the holder alone; reports whether the size changed.
    pub fn resize(&self, client: &Client, cols: u16, rows: u16) -> Result<bool, Refusal> {
        let mut screen = lock(&self.screen);
        screen.check_holder(client)?;
        Ok(self.set_size(&mut screen, cols, rows)?)
    }

    pub fn reset_size(&self) -> Result<(), Error> {
        let (cols, rows) = INITIAL_SIZE;
        self.host.call(&HostRequest::Resize { id: self.id.clone(), cols, rows })
    }

    pub fn activity(&self) -> Result<Activity, Error> {
        self.host.call(&HostRequest::Activity { id: self.id.clone() })
    }

    pub fn terminate(&self) -> Result<(), Error> {
        self.host.call(&HostRequest::Terminate { id: self.id.clone() })
    }

    /// Ends the terminal and drops its scrollback; a terminal the host no longer has is already gone.
    pub fn forget(&self) -> Result<(), Error> {
        match self.host.forget(&self.id) {
            Err(Error::NotFound { .. }) => Ok(()),
            forgot => forgot,
        }
    }

    pub async fn attach(&self) -> Result<local::Stream, Error> {
        self.host.attach(&self.id).await
    }
}
