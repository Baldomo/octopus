#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::Duration;

use octopus_client::account::Book;
use octopus_client::servers::Address;
use octopus_client::{CallError, Connection, Identity};
use octopus_proto::device::DeviceKey;
use octopus_proto::local::Listener;
use octopus_proto::ops::{HarnessSet, TaskCreate, TaskFocus, TaskGet, TermSend, WorkspaceAdd};
use octopus_proto::remote::{self, RelayUrl};
use octopus_proto::types::{
    CheckoutKind, Color, Hooks, Panel, ReadDir, SessionStart, Stream, Task, TaskId, TaskState, WorkspaceId,
};
use octopus_server::{Server, TerminalHost, serve};
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncReadExt};

pub const SERVER: &str = "mac-mini";
pub const OCTOPUS: &str = env!("CARGO_BIN_EXE_octopus");

pub struct Running {
    pub server: Arc<Server>,
    pub socket: PathBuf,
    /// The relay the server is homed on, served on loopback over plain HTTP for as long as the server runs.
    pub relay: iroh_relay::server::Server,
}

/// A relay URL that refuses every connection, for a server whose tests reach it over the local socket alone.
const NO_RELAY: &str = "http://127.0.0.1:9/";

/// Serves `dir`'s store on `socket`, keeping terminals in the host at `host`. A second call on the same `dir` and
/// `host` plays a server restart; a fresh `host` plays a machine restart: the server finds none of its terminals.
pub async fn start(dir: &Path, socket: &str, host: &str) -> Running {
    start_with(dir, socket, host, false).await
}

/// A server started in a fresh directory, which lives as long as the returned `TempDir`.
pub async fn server() -> (TempDir, Running) {
    let dir = tempfile::tempdir().expect("tempdir");
    let running = start(dir.path(), "server.sock", "host.sock").await;
    (dir, running)
}

/// As `start`, giving each terminal a virtual microphone when `microphone` holds.
pub async fn start_with(dir: &Path, socket: &str, host: &str, microphone: bool) -> Running {
    let mut config = iroh_relay::server::ServerConfig::default();
    config.relay = Some(iroh_relay::server::RelayConfig::new(([127, 0, 0, 1], 0)));
    let relay = iroh_relay::server::Server::spawn(config).await.expect("spawn a relay");
    let url: RelayUrl = format!("http://{}", relay.http_addr().expect("the relay serves HTTP")).parse().expect("a URL");
    let socket = dir.join(socket);
    let host = TerminalHost { program: OCTOPUS.into(), address: dir.join(host) };
    let server = Server::open(SERVER, &dir.join("store.db"), &dir.join("worktrees"), &host, url.clone(), microphone)
        .expect("open server");
    let endpoint = remote::listen(([127, 0, 0, 1], 0).into(), url, server.device()).await.expect("bind iroh");
    tokio::spawn(serve(Arc::clone(&server), Listener::bind(&socket).expect("bind"), endpoint));
    Running { server, socket, relay }
}

/// A client named `name` with a key of its own.
pub fn identity(name: &str) -> Identity {
    let key = DeviceKey::generate();
    let account = Book::memory(&key);
    Identity::new(name.into(), key, account).expect("an identity")
}

pub fn local(socket: &Path) -> Address {
    Address::Local { socket: socket.to_path_buf() }
}

/// An `octopus server run` process, killed when dropped.
pub struct ServerProcess {
    child: Child,
    pub socket: PathBuf,
}

fn server_run(dir: &Path) -> Command {
    let mut command = Command::new(OCTOPUS);
    command
        .args(["server", "run", "--name", SERVER, "--socket"])
        .arg(dir.join("server.sock"))
        .arg("--store")
        .arg(dir.join("store.db"))
        .arg("--worktrees")
        .arg(dir.join("worktrees"))
        .arg("--terminal-host")
        .arg(dir.join("host.sock"))
        .args(["--udp", "127.0.0.1:0", "--relay", NO_RELAY, "--microphone", "false"]);
    command
}

impl ServerProcess {
    pub async fn run(dir: &Path) -> Self {
        Self::serve(dir, server_run(dir)).await
    }

    /// Started the way launchd starts a service: a bare PATH, and `home` as the account's home.
    pub async fn launched(dir: &Path, home: &Path) -> Self {
        let mut command = server_run(dir);
        command.env_clear().env("HOME", home).env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
        Self::serve(dir, command).await
    }

    async fn serve(dir: &Path, mut command: Command) -> Self {
        let child = command.spawn().expect("spawn octopus server run");
        let process = Self { child, socket: dir.join("server.sock") };
        let probe = identity("probe");
        eventually("the server answering", async || Connection::connect(&local(&process.socket), &probe).await.is_ok())
            .await;
        process
    }

    pub async fn connect(&self, client: &str) -> Connection {
        Connection::connect(&local(&self.socket), &identity(client)).await.expect("connect")
    }

    /// Ends the process the way a crash does, with no chance to clean up.
    pub fn kill(mut self) {
        self.child.kill().expect("kill the server");
        self.child.wait().expect("reap the server");
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _already_dead = self.child.kill();
        let _reaped = self.child.wait();
    }
}

pub async fn connect(running: &Running, client: &str) -> Connection {
    Connection::connect(&local(&running.socket), &identity(client)).await.expect("connect")
}

/// An agent that prints `<line>`, then stays open reading input, which the terminal echoes.
pub fn echo_agent(line: &str) -> Vec<String> {
    vec!["sh".into(), "-c".into(), format!("echo {line}; exec cat")]
}

/// A harness that is always installed and names no session.
pub fn harness(id: &str, argv: Vec<String>, hooks: Hooks) -> HarnessSet {
    HarnessSet { id: id.into(), argv, install: "true".into(), start: vec![], resume: vec![], hooks }
}

pub async fn set_harness(connection: &Connection, id: &str, argv: Vec<String>, hooks: Hooks) {
    connection.call(&harness(id, argv, hooks)).await.expect("harness.set");
}

pub async fn focus(connection: &Connection, id: &TaskId) {
    connection.call(&TaskFocus { task: id.clone(), args: vec![] }).await.expect("task.focus");
}

/// The stream of the task's agent terminal.
pub async fn agent(connection: &Connection, id: &TaskId) -> Stream {
    Stream::Pty(task(connection, id).await.panels[0].id.clone())
}

/// Types `text` and enter into `panel`, taking its lock.
pub fn send(id: &TaskId, panel: &str, text: &str) -> TermSend {
    TermSend { task: id.clone(), panel: panel.into(), text: text.into(), enter: true, take_lock: true }
}

/// Writes `script` as the login profile of both sh-family shells and zsh under `home`.
pub fn write_login_profile(home: &Path, script: &str) {
    for file in [".profile", ".zprofile"] {
        std::fs::write(home.join(file), script).expect("profile");
    }
}

pub async fn workspace(connection: &Connection, root: &Path) -> WorkspaceId {
    let add = WorkspaceAdd { name: "octopus".into(), root: root.to_string_lossy().to_string(), color: Color::Auto };
    connection.call(&add).await.expect("workspace.add").workspace
}

pub async fn create(connection: &Connection, workspace: &WorkspaceId, checkout: CheckoutKind, harness: &str) -> TaskId {
    let create = TaskCreate {
        workspace: workspace.clone(),
        title: "Strict serde".into(),
        parent: None,
        state: TaskState::Pinned,
        checkout,
        harness: harness.into(),
        session: SessionStart::Fresh,
    };
    connection.call(&create).await.expect("task.create").task
}

pub async fn task(connection: &Connection, task: &TaskId) -> Task {
    connection.call(&TaskGet { task: task.clone() }).await.expect("task.get")
}

pub fn refused<T: std::fmt::Debug>(outcome: Result<T, CallError>) -> octopus_proto::Error {
    match outcome {
        Err(CallError::Server(error)) => error,
        other => panic!("expected the server to refuse, got {other:?}"),
    }
}

/// The label of the panel's lock holder, the way every client shows it.
pub fn holder_name(panel: &Panel) -> Option<&str> {
    panel.lock_holder.as_ref().map(|holder| holder.name.as_str())
}

/// A read's file size and the bytes it sent.
pub async fn read(
    connection: &Connection,
    dir: ReadDir,
    path: &str,
    start: u64,
    len: u64,
) -> Result<(u64, Vec<u8>), CallError> {
    let (size, mut opened) = connection.read(dir, path.into(), start, len).await?;
    let mut bytes = Vec::new();
    opened.read_to_end(&mut bytes).await.expect("read the file");
    Ok((size, bytes))
}

/// Everything the stream sent until `needle` arrived.
pub async fn read_until(stream: &mut (impl AsyncRead + Unpin), needle: &str) -> String {
    let mut seen = Vec::new();
    let mut buffer = [0u8; 4096];
    let found = tokio::time::timeout(Duration::from_secs(5), async {
        while !String::from_utf8_lossy(&seen).contains(needle) {
            let read = stream.read(&mut buffer).await.expect("read stream");
            assert!(read > 0, "stream ended before {needle:?}; saw {:?}", String::from_utf8_lossy(&seen));
            seen.extend_from_slice(&buffer[..read]);
        }
    })
    .await;
    assert!(found.is_ok(), "no {needle:?} within 5 s; saw {:?}", String::from_utf8_lossy(&seen));
    String::from_utf8_lossy(&seen).into_owned()
}

pub async fn eventually(what: &str, mut check: impl AsyncFnMut() -> bool) {
    let reached = tokio::time::timeout(Duration::from_secs(8), async {
        while !check().await {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(reached.is_ok(), "{what} did not happen within 8 s");
}
