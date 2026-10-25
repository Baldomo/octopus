#![allow(dead_code)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::Duration;

use octopus_client::servers::Address;
use octopus_client::{Connection, Identity};
use octopus_proto::device::DeviceKey;
use octopus_proto::local::Listener;
use octopus_proto::ops::{TaskCreate, TaskGet, WorkspaceAdd};
use octopus_proto::quic;
use octopus_proto::types::{Checkout, Task, TaskId, TaskState, WorkspaceId};
use octopus_server::{Server, TerminalHost, serve};
use tokio::io::{AsyncRead, AsyncReadExt};

pub const SERVER: &str = "mac-mini";
pub const OCTOPUS: &str = env!("CARGO_BIN_EXE_octopus");

pub struct Running {
    pub server: Arc<Server>,
    pub socket: PathBuf,
    /// Where QUIC listens, on loopback.
    pub quic: SocketAddr,
}

/// The local socket `name` of the test in `dir`; on Windows a named pipe, which lives outside the filesystem.
pub fn address(dir: &Path, name: &str) -> PathBuf {
    if cfg!(windows) {
        let test = dir.file_name().expect("a tempdir has a name").to_string_lossy();
        PathBuf::from(format!(r"\\.\pipe\octopus-{test}-{name}"))
    } else {
        dir.join(name)
    }
}

/// Serves `dir`'s store on `socket`, keeping terminals in the host at `host`. A second call on the same `dir` and
/// `host` plays a server restart; a fresh `host` plays a machine restart: the server finds none of its terminals.
pub fn start(dir: &Path, socket: &str, host: &str) -> Running {
    let socket = address(dir, socket);
    let host = TerminalHost { program: OCTOPUS.into(), address: address(dir, host) };
    let server = Server::open(SERVER, &dir.join("store.db"), &dir.join("worktrees"), &host).expect("open server");
    let remote = quic::listen(([127, 0, 0, 1], 0).into(), server.device()).expect("listen on QUIC");
    let quic = remote.local_addr().expect("the QUIC address");
    tokio::spawn(serve(Arc::clone(&server), Listener::bind(&socket).expect("bind"), remote));
    Running { server, socket, quic }
}

/// A client named `name` with a key of its own.
pub fn identity(name: &str) -> Identity {
    Identity { name: name.into(), key: DeviceKey::generate() }
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
        .arg(address(dir, "server.sock"))
        .arg("--store")
        .arg(dir.join("store.db"))
        .arg("--worktrees")
        .arg(dir.join("worktrees"))
        .arg("--terminal-host")
        .arg(address(dir, "host.sock"))
        .args(["--quic", "127.0.0.1:0"]);
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
        let process = Self { child, socket: address(dir, "server.sock") };
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

pub fn shell_var(var: &str) -> String {
    if cfg!(windows) { format!("%{var}%") } else { format!("${var}") }
}

/// An agent that prints `<line>`, then stays open reading input, which the terminal echoes.
pub fn echo_agent(line: &str) -> Vec<String> {
    if cfg!(windows) {
        vec!["cmd".into(), "/q".into(), "/k".into(), format!("echo {line}")]
    } else {
        vec!["sh".into(), "-c".into(), format!("echo {line}; exec cat")]
    }
}

pub async fn workspace(connection: &Connection, root: &Path) -> WorkspaceId {
    let add = WorkspaceAdd { name: "octopus".into(), root: root.to_string_lossy().to_string() };
    connection.call(&add).await.expect("workspace.add").workspace
}

pub async fn create(connection: &Connection, workspace: &WorkspaceId, checkout: Checkout, harness: &str) -> TaskId {
    let create = TaskCreate {
        workspace: workspace.clone(),
        title: "Strict serde".into(),
        parent: None,
        state: TaskState::Pinned,
        checkout,
        harness: harness.into(),
        session: None,
    };
    connection.call(&create).await.expect("task.create").task
}

pub async fn task(connection: &Connection, task: &TaskId) -> Task {
    connection.call(&TaskGet { task: task.clone() }).await.expect("task.get")
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
