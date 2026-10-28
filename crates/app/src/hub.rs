use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use octopus_client::cache::{Cache, PushEffect};
use octopus_client::servers::{Address, Server, Servers};
use octopus_client::{CallError, Connection, Identity};
use octopus_proto::device::PublicKey;
use octopus_proto::ops::{SessionSync, WorkspaceRename, WorkspaceSetColor};
use octopus_proto::types::{Color, WorkspaceId};
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
    /// What the window calls the server: "local" for one on the local socket, else its name.
    pub label: String,
    /// The relay a remote server is dialled through; none for a server on the local socket.
    pub relay: Option<String>,
    pub reachable: bool,
    /// When `reachable` last changed, in milliseconds since 1970.
    pub since: i64,
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
    since: i64,
    supervisor: tauri::async_runtime::JoinHandle<()>,
}

impl Drop for Link {
    fn drop(&mut self) {
        self.supervisor.abort();
    }
}

#[derive(Default)]
struct State {
    links: BTreeMap<String, Link>,
    /// A server's cache outlives its connection, so an unreachable server's tasks stay listed.
    caches: BTreeMap<String, Cache>,
    files_changed: BTreeMap<TaskRef, BTreeSet<String>>,
}

impl State {
    fn label<'a>(&self, server: &'a str) -> &'a str {
        match self.links.get(server).map(|link| &link.address) {
            Some(Address::Local { .. }) => "local",
            _ => server,
        }
    }
}

fn now() -> i64 {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).expect("the clock reads after 1970");
    i64::try_from(elapsed.as_millis()).expect("milliseconds since 1970 fit in i64")
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

impl Hub {
    pub fn label(&self, server: &str) -> String {
        lock(&self.state).label(server).to_string()
    }

    pub fn failure(&self, server: &str, error: CallError) -> String {
        let server = self.label(server);
        match error {
            CallError::Server(error) => format!("{server}: {error}"),
            CallError::Transport(error) => format!("{server}: {error:#}"),
        }
    }

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

    /// A link that goes aborts its supervisor at its next await, so a step it already started may find its link gone.
    fn supervise(self: &Arc<Self>, server: Server) {
        let mut state = lock(&self.state);
        let (hub, name, address) = (Arc::clone(self), server.name.clone(), server.address.clone());
        let supervisor = tauri::async_runtime::spawn(async move {
            loop {
                let problem = match hub.session(&server).await {
                    Ok(closed) => closed,
                    Err(error) => format!("{error:#}"),
                };
                hub.lost(&server.name, problem);
                tokio::time::sleep(RECONNECT).await;
            }
        });
        state.links.insert(name, Link { address, connection: None, problem: None, since: now(), supervisor });
        self.servers_changed.notify_one();
    }

    /// Stops reaching the server and drops its tasks from the lists.
    fn unsupervise(&self, server: &str) {
        let mut state = lock(&self.state);
        state.links.remove(server);
        state.caches.remove(server);
        self.model_changed.notify_one();
        self.servers_changed.notify_one();
    }

    /// Runs one connection until it ends, keeping the server's cache current, and resolves to why it closed.
    async fn session(&self, server: &Server) -> anyhow::Result<String> {
        let connection = Arc::new(Connection::connect(&server.address, &self.me).await?);
        if let Err(error) = Servers::refresh(&server.name, connection.server_addresses()) {
            eprintln!("octopus app: keep the addresses {} reported: {error:#}", server.name);
        }
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
        let Some(link) = state.links.get_mut(server) else { return Ok(()) };
        if link.connection.is_none() {
            link.since = now();
        }
        link.connection = Some(Arc::clone(connection));
        link.problem = None;
        state.caches.insert(server.to_string(), Cache::from_snapshot(snapshot));
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
        let Some(cache) = state.caches.get_mut(server) else { return };
        if cache.apply(&push) == PushEffect::Applied {
            self.model_changed.notify_one();
        }
    }

    fn lost(&self, server: &str, problem: String) {
        let mut state = lock(&self.state);
        let Some(link) = state.links.get_mut(server) else { return };
        if link.connection.take().is_some() {
            link.since = now();
        }
        link.problem = Some(problem);
        self.servers_changed.notify_one();
    }

    /// Re-pairing a listed server, to pin a new key, is the CLI's `pairing accept`; this window keeps the pins it
    /// started with.
    pub fn refuse_listed(&self, key: &PublicKey) -> anyhow::Result<()> {
        let state = lock(&self.state);
        let listed = state
            .links
            .iter()
            .find(|(_, link)| matches!(&link.address, Address::Remote { key: listed, .. } if listed == key));
        match listed {
            Some((name, _)) => {
                anyhow::bail!("{key} is listed as {name}; re-pair it with `octopus pairing accept`")
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

    /// Lists the server under a new name and reaches it afresh under that name.
    pub fn rename_server(self: &Arc<Self>, from: &str, to: &str) -> anyhow::Result<()> {
        if lock(&self.state).links.contains_key(to) {
            anyhow::bail!("a server named {to} is already in the list");
        }
        let mut servers = Servers::load()?;
        let renamed = servers.rename(from, to)?;
        servers.save()?;
        self.unsupervise(from);
        self.supervise(renamed);
        Ok(())
    }

    pub fn forget_server(&self, server: &str) -> anyhow::Result<()> {
        let mut servers = Servers::load()?;
        servers.forget(server)?;
        servers.save()?;
        self.unsupervise(server);
        Ok(())
    }

    /// Renames the section's workspace on every server of the section, refusing before any rename when a server
    /// would split the section: one that is unreachable, or one that already holds a workspace named `to`.
    pub async fn rename_section(&self, from: &str, to: &str) -> Result<(), String> {
        let taken = |server: &str, cache: &Cache| {
            (from != to && cache.workspaces.values().any(|workspace| workspace.name == to))
                .then(|| format!("{server} already has {to}"))
        };
        let rename = |workspace: WorkspaceId| WorkspaceRename { workspace, name: to.into() };
        self.across_section(from, &format!("renamed {from} to {to}"), taken, rename).await
    }

    /// Colors the section's workspace on every server of the section, refusing before any while one is unreachable.
    pub async fn color_section(&self, section: &str, color: Color) -> Result<(), String> {
        let recolor = |workspace: WorkspaceId| WorkspaceSetColor { workspace, color };
        self.across_section(section, &format!("colored {section} {}", color.name()), |_, _| None, recolor).await
    }

    /// Sends `op` for the section's workspace on each of its servers, refusing before any when one is unreachable or
    /// `refuses` its cache, saying why; a failure partway names the servers that already `did` it.
    async fn across_section<O: Op>(
        &self,
        section: &str,
        did: &str,
        refuses: impl Fn(&str, &Cache) -> Option<String>,
        op: impl Fn(WorkspaceId) -> O,
    ) -> Result<(), String> {
        let ops = {
            let state = lock(&self.state);
            let mut ops = Vec::new();
            for (server, cache) in &state.caches {
                let Some(workspace) = cache.workspaces.values().find(|workspace| workspace.name == section) else {
                    continue;
                };
                let label = state.label(server);
                if state.links.get(server).is_none_or(|link| link.connection.is_none()) {
                    return Err(format!(
                        "{label} is unreachable, so {section} would change on only some of its servers"
                    ));
                }
                if let Some(refusal) = refuses(label, cache) {
                    return Err(refusal);
                }
                ops.push((server.clone(), label.to_string(), op(workspace.id.clone())));
            }
            ops
        };
        let mut done = Vec::new();
        for (server, label, op) in ops {
            if let Err(error) = self.call(&server, &op).await {
                return Err(match done.as_slice() {
                    [] => error,
                    done => format!("{error}; {} already {did}", done.join(", ")),
                });
            }
            done.push(label);
        }
        Ok(())
    }

    pub async fn call<O: Op>(&self, server: &str, op: &O) -> Result<O::Ok, String> {
        self.connection(server)?.call(op).await.map_err(|error| self.failure(server, error))
    }

    /// Whether the server runs on another machine, where its ports are reached through a tunnel.
    pub fn is_another_machine(&self, server: &str) -> bool {
        lock(&self.state).links.get(server).is_some_and(|link| matches!(link.address, Address::Remote { .. }))
    }

    pub fn connection(&self, server: &str) -> Result<Arc<Connection>, String> {
        let state = lock(&self.state);
        match state.links.get(server) {
            Some(Link { connection: Some(connection), .. }) => Ok(Arc::clone(connection)),
            Some(Link { problem, .. }) => {
                Err(format!("{} is unreachable: {}", state.label(server), problem.as_deref().unwrap_or("connecting")))
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
        let state = lock(&self.state);
        state
            .links
            .iter()
            .map(|(name, link)| ServerView {
                name: name.clone(),
                label: state.label(name).to_string(),
                relay: match &link.address {
                    Address::Local { .. } => None,
                    Address::Remote { relay, .. } => Some(relay.to_string()),
                },
                reachable: link.connection.is_some(),
                since: link.since,
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
