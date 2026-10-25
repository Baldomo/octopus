use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use octopus_client::cache::{Cache, PushEffect};
use octopus_client::servers::{Server, Servers};
use octopus_client::{CallError, Connection};
use octopus_proto::ops::{SessionSync, WorkspaceList};
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
const PING: Duration = Duration::from_secs(2);
const PING_TIMEOUT: Duration = Duration::from_secs(5);
const MODEL_FRAME: Duration = Duration::from_millis(16);
const FILES_BATCH: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct ServerView {
    pub name: String,
    pub reachable: bool,
    /// The round trip of the newest `workspace.list`, which stands in for a ping.
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

#[derive(Default)]
struct Link {
    connection: Option<Arc<Connection>>,
    rtt: Option<Duration>,
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
    pub me: String,
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
    pub fn start(app: AppHandle, me: String, servers: Vec<Server>) -> Arc<Self> {
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
        lock(&self.state).links.insert(server.name.clone(), Link::default());
        self.servers_changed.notify_one();
        let hub = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            loop {
                let problem = match hub.session(&server).await {
                    Ok(()) => "the server closed the connection".to_string(),
                    Err(error) => format!("{error:#}"),
                };
                hub.lost(&server.name, problem);
                tokio::time::sleep(RECONNECT).await;
            }
        });
    }

    /// Runs one connection until it ends, keeping the server's cache current.
    async fn session(&self, server: &Server) -> anyhow::Result<()> {
        let connection = Arc::new(Connection::connect(&server.socket, &self.me).await?);
        let mut pushes = connection.pushes();
        self.resync(&server.name, &connection).await?;
        let mut ping = tokio::time::interval(PING);
        loop {
            tokio::select! {
                () = connection.closed() => return Ok(()),
                push = pushes.recv() => match push {
                    Ok(push) => self.apply(&server.name, push),
                    Err(RecvError::Lagged(_)) => self.resync(&server.name, &connection).await?,
                    Err(RecvError::Closed) => unreachable!("the connection owns its push sender"),
                },
                _ = ping.tick() => self.ping(&server.name, &connection).await?,
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

    async fn ping(&self, server: &str, connection: &Connection) -> anyhow::Result<()> {
        let started = Instant::now();
        tokio::time::timeout(PING_TIMEOUT, connection.call(&WorkspaceList {})).await??;
        lock(&self.state).link(server).rtt = Some(started.elapsed());
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
        link.rtt = None;
        link.problem = Some(problem);
        self.servers_changed.notify_one();
    }

    pub fn add(self: &Arc<Self>, server: Server) -> anyhow::Result<()> {
        let mut servers = Servers::load()?;
        let supervised = lock(&self.state).links.contains_key(&server.name);
        if supervised || servers.servers.iter().any(|known| known.name == server.name) {
            anyhow::bail!("a server named {} is already in the list", server.name);
        }
        servers.add(server.clone())?;
        servers.save()?;
        self.supervise(server);
        Ok(())
    }

    pub async fn call<O: Op>(&self, server: &str, op: &O) -> Result<O::Ok, String> {
        self.connection(server)?.call(op).await.map_err(|error| failure(server, error))
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
                rtt_ms: link.rtt.map(|rtt| saturate(rtt.as_millis())),
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
