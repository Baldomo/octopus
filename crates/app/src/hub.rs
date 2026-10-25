use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use octopus_client::cache::{Cache, PushEffect};
use octopus_client::servers::{Address, Server, Servers};
use octopus_client::{CallError, Connection, Identity};
use octopus_proto::ops::SessionSync;
use octopus_proto::{Event, Op, Push};
use serde::Serialize;
use tauri::AppHandle;
use tokio::sync::Notify;
use tokio::sync::broadcast::error::RecvError;
use ts_rs::TS;

use crate::task_lists::{Lists, TaskRef, lists};
use crate::window::{TaskView, task_views};
use crate::{emit, lock, saturate};

pub const MODEL_EVENT: &str = "model";
pub const SERVERS_EVENT: &str = "servers";
pub const FILES_CHANGED_EVENT: &str = "files_changed";

const RECONNECT: Duration = Duration::from_secs(1);
/// How often a server's chip and status bar pick up its connection's newest round trip.
const RTT_REFRESH: Duration = Duration::from_secs(2);
const MODEL_FRAME: Duration = Duration::from_millis(16);
const FILES_BATCH: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct ServerView {
    pub name: String,
    pub reachable: bool,
    /// The round trip of the connection's newest heartbeat.
    pub rtt_ms: Option<u32>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Model {
    /// Raised by every model built, so the UI drops one that arrives after a newer one.
    pub version: u64,
    pub lists: Lists,
    pub tasks: BTreeMap<String, TaskView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct FilesChanged {
    pub key: String,
    pub paths: Vec<String>,
}

struct Link {
    address: Address,
    connection: Option<Arc<Connection>>,
    problem: Option<String>,
}

#[derive(Default)]
struct State {
    links: BTreeMap<String, Link>,
    /// A server's cache outlives its connection, so an unreachable server's tasks stay listed.
    caches: BTreeMap<String, Cache>,
    files_changed: BTreeMap<TaskRef, BTreeSet<String>>,
}

impl State {
    fn link(&mut self, server: &str) -> &mut Link {
        self.links.get_mut(server).expect("supervise lists a server before its session runs")
    }
}

pub struct Hub {
    pub me: Identity,
    app: AppHandle,
    state: Mutex<State>,
    versions: AtomicU64,
    model_changed: Notify,
    servers_changed: Notify,
    files_changed: Notify,
}

pub fn failure(server: &str, error: CallError) -> String {
    match error {
        CallError::Server(error) => format!("{server}: {error}"),
        CallError::Transport(error) => format!("{server}: {error:#}"),
    }
}

impl Hub {
    pub fn start(app: AppHandle, me: Identity, servers: Vec<Server>) -> Arc<Self> {
        let hub = Arc::new(Self {
            me,
            app,
            state: Mutex::default(),
            versions: AtomicU64::new(0),
            model_changed: Notify::new(),
            servers_changed: Notify::new(),
            files_changed: Notify::new(),
        });
        tauri::async_runtime::spawn(Arc::clone(&hub).emit_models());
        tauri::async_runtime::spawn(Arc::clone(&hub).emit_servers());
        tauri::async_runtime::spawn(Arc::clone(&hub).emit_files_changed());
        for server in servers {
            hub.supervise(server);
        }
        hub
    }

    fn supervise(self: &Arc<Self>, server: Server) {
        let link = Link { address: server.address.clone(), connection: None, problem: None };
        lock(&self.state).links.insert(server.name.clone(), link);
        self.servers_changed.notify_one();
        let hub = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            loop {
                let problem = match hub.session(&server).await {
                    Ok(closed) => closed,
                    Err(error) => format!("{error:#}"),
                };
                hub.lost(&server.name, problem);
                tokio::time::sleep(RECONNECT).await;
            }
        });
    }

    /// Runs one connection until it ends, keeping the server's cache current, and resolves to why it closed.
    async fn session(&self, server: &Server) -> anyhow::Result<String> {
        let connection = Arc::new(Connection::connect(&server.address, &self.me).await?);
        let mut pushes = connection.pushes();
        self.resync(&server.name, &connection).await?;
        let mut refresh = tokio::time::interval(RTT_REFRESH);
        loop {
            tokio::select! {
                closed = connection.closed() => return Ok(closed),
                push = pushes.recv() => match push {
                    Ok(push) => self.apply(&server.name, push),
                    Err(RecvError::Lagged(_)) => self.resync(&server.name, &connection).await?,
                    Err(RecvError::Closed) => unreachable!("the connection owns its push sender"),
                },
                _ = refresh.tick() => self.servers_changed.notify_one(),
            }
        }
    }

    async fn resync(&self, server: &str, connection: &Arc<Connection>) -> anyhow::Result<()> {
        let since_rev = lock(&self.state).caches.get(server).map(|cache| cache.rev);
        let snapshot = connection.call(&SessionSync { since_rev }).await?;
        let mut state = lock(&self.state);
        state.caches.insert(server.to_string(), Cache::from_snapshot(snapshot));
        let link = state.link(server);
        link.connection = Some(Arc::clone(connection));
        link.problem = None;
        self.model_changed.notify_one();
        self.servers_changed.notify_one();
        Ok(())
    }

    fn apply(&self, server: &str, push: Push) {
        let mut state = lock(&self.state);
        if let Event::FileChanged { task, path } = push.event {
            let at = TaskRef { server: server.to_string(), task };
            state.files_changed.entry(at).or_default().insert(path);
            self.files_changed.notify_one();
            return;
        }
        let cache = state.caches.get_mut(server).expect("a session resyncs before it applies pushes");
        if cache.apply(&push) == PushEffect::Applied {
            self.model_changed.notify_one();
        }
    }

    fn lost(&self, server: &str, problem: String) {
        let mut state = lock(&self.state);
        let link = state.link(server);
        link.connection = None;
        link.problem = Some(problem);
        self.servers_changed.notify_one();
    }

    /// Re-pairing a listed server, to pin a new key, is the CLI's `pairing accept`; this window keeps the pins it
    /// started with.
    pub fn refuse_listed(&self, endpoint: &str) -> anyhow::Result<()> {
        let state = lock(&self.state);
        let listed = state
            .links
            .iter()
            .find(|(_, link)| matches!(&link.address, Address::Quic { endpoint: listed, .. } if listed == endpoint));
        match listed {
            Some((name, _)) => {
                anyhow::bail!("{endpoint} is listed as {name}; re-pair it with `octopus pairing accept`")
            }
            None => Ok(()),
        }
    }

    pub fn add(self: &Arc<Self>, server: Server) -> anyhow::Result<()> {
        if lock(&self.state).links.contains_key(&server.name) {
            anyhow::bail!("a server named {} is already in the list", server.name);
        }
        Servers::remember(server.clone())?;
        self.supervise(server);
        Ok(())
    }

    pub async fn call<O: Op>(&self, server: &str, op: &O) -> Result<O::Ok, String> {
        self.connection(server)?.call(op).await.map_err(|error| failure(server, error))
    }

    /// Whether the server runs on another machine, where its ports are reached through a tunnel.
    pub fn is_another_machine(&self, server: &str) -> bool {
        lock(&self.state).links.get(server).is_some_and(|link| link.address.is_another_machine())
    }

    pub fn connection(&self, server: &str) -> Result<Arc<Connection>, String> {
        match lock(&self.state).links.get(server) {
            Some(Link { connection: Some(connection), .. }) => Ok(Arc::clone(connection)),
            Some(Link { problem, .. }) => {
                Err(format!("{server} is unreachable: {}", problem.as_deref().unwrap_or("connecting")))
            }
            None => Err(format!("no server named {server} in the list")),
        }
    }

    pub fn model(&self) -> Model {
        let state = lock(&self.state);
        let version = self.versions.fetch_add(1, Ordering::Relaxed);
        Model { version, lists: lists(&state.caches), tasks: task_views(&state.caches) }
    }

    pub fn servers(&self) -> Vec<ServerView> {
        lock(&self.state)
            .links
            .iter()
            .map(|(name, link)| ServerView {
                name: name.clone(),
                reachable: link.connection.is_some(),
                rtt_ms: link
                    .connection
                    .as_ref()
                    .and_then(|connection| connection.rtt())
                    .map(|rtt| saturate(rtt.as_millis())),
                problem: link.problem.clone(),
            })
            .collect()
    }

    async fn emit_models(self: Arc<Self>) {
        loop {
            self.model_changed.notified().await;
            emit(&self.app, MODEL_EVENT, self.model());
            tokio::time::sleep(MODEL_FRAME).await;
        }
    }

    async fn emit_servers(self: Arc<Self>) {
        loop {
            self.servers_changed.notified().await;
            emit(&self.app, SERVERS_EVENT, self.servers());
        }
    }

    async fn emit_files_changed(self: Arc<Self>) {
        loop {
            self.files_changed.notified().await;
            tokio::time::sleep(FILES_BATCH).await;
            let changed: Vec<FilesChanged> = std::mem::take(&mut lock(&self.state).files_changed)
                .into_iter()
                .map(|(at, paths)| FilesChanged { key: at.key(), paths: paths.into_iter().collect() })
                .collect();
            emit(&self.app, FILES_CHANGED_EVENT, changed);
        }
    }
}
