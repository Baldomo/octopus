use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use octopus_client::cache::{Cache, PushEffect};
use octopus_client::servers::{self, Address, Config, Server, Servers};
use octopus_client::tunnel::{Tunnel, listen_on_localhost, loopback_port};
use octopus_client::{CallError, Connection, Identity};
use octopus_proto::account::{Account, Change, Link as AccountLink};
use octopus_proto::device::PublicKey;
use octopus_proto::ops::{Revised, SessionSync, TaskFocus, WorkspaceRename, WorkspaceSetColor};
use octopus_proto::pairing::Invite;
use octopus_proto::settings::{AgentFlags, Current, Settings};
use octopus_proto::types::{Color, LockHolder, Task, Workspace, WorkspaceId};
use octopus_proto::{Event, Op, Push, now};
use serde::Serialize;
use tauri::{AppHandle, Url};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{Notify, mpsc, oneshot, watch};
use ts_rs::TS;

use crate::microphone::{AskAccess, Capture, Listener, MICROPHONE_DENIED_EVENT, Microphone};
use crate::task_lists::{Lists, TaskRef, Waking, lists};
use crate::task_view::{TaskView, task_views};
use crate::{emit, lock, saturate};

pub(crate) const MODEL_EVENT: &str = "model";
pub(crate) const SERVERS_EVENT: &str = "servers";
pub(crate) const FILES_CHANGED_EVENT: &str = "files_changed";
pub(crate) const ACCOUNT_EVENT: &str = "account";

/// A server is dialled again no sooner than this after its last dial began, unless a resume or redial asks now.
const RECONNECT: Duration = Duration::from_secs(1);
const RESUME_PATIENCE: Duration = Duration::from_secs(2);
/// How long a resume, or a call made while its server reconnects, waits for the connection.
const RECONNECT_PATIENCE: Duration = Duration::from_secs(3);
/// How often a server's chip and status bar pick up its connection's newest round trip.
const RTT_REFRESH: Duration = Duration::from_secs(2);
const MODEL_FRAME: Duration = Duration::from_millis(16);
const FILES_BATCH: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub(crate) struct ServerView {
    pub name: String,
    /// What the window calls the server: "local" for one on the local socket, else its name.
    pub label: String,
    /// The relay a remote server is dialled through; none for a server on the local socket.
    pub relay: Option<String>,
    /// The key a remote server is pinned by, in hex; none for a server on the local socket.
    pub key: Option<String>,
    pub reachable: bool,
    /// When `reachable` last changed, in milliseconds since 1970.
    pub since: i64,
    /// The round trip of the connection's newest heartbeat.
    pub rtt_ms: Option<u32>,
    pub problem: Option<String>,
}

/// This client's account as the window shows it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub(crate) struct AccountView {
    /// The first eight hex digits of the account's id.
    pub id: String,
    /// Whether the account still holds this client.
    pub member: bool,
    pub devices: Vec<DeviceView>,
    pub settings: Current,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub(crate) struct DeviceView {
    pub name: String,
    /// The device's key, in hex.
    pub key: String,
    /// Whether it is this client.
    pub this: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Model {
    /// Raised by every model built, so the UI drops one that arrives after a newer one.
    pub version: u64,
    pub lists: Lists,
    pub tasks: BTreeMap<String, TaskView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub(crate) struct FilesChanged {
    pub key: String,
    pub paths: Vec<String>,
}

struct Link {
    address: Address,
    connection: Option<Arc<Connection>>,
    problem: Option<String>,
    since: i64,
    /// When the link last lost a connection it held, on a clock that only moves forward.
    dropped: Option<tokio::time::Instant>,
    resyncs: mpsc::UnboundedSender<oneshot::Sender<Result<(), String>>>,
    supervisor: tauri::async_runtime::JoinHandle<()>,
}

impl Link {
    fn reachable(&self) -> bool {
        self.connection.as_ref().is_some_and(|connection| !connection.is_closed())
    }

    /// Unreachable through a connection that has just closed or dropped, which the client may reach again shortly.
    fn dropped_lately(&self) -> bool {
        !self.reachable()
            && (self.connection.is_some() || self.dropped.is_some_and(|at| at.elapsed() < RECONNECT_PATIENCE))
    }
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
    waking: Waking,
}

impl State {
    /// Whether `me` holds the lock `holder` holds on `server`.
    fn held_here(&self, me: &Identity, server: &str, holder: &LockHolder) -> bool {
        self.links.get(server).is_some_and(|link| me.holds(holder, &link.address))
    }

    fn label<'a>(&self, server: &'a str) -> &'a str {
        match self.links.get(server).map(|link| &link.address) {
            Some(Address::Local { .. }) => "local",
            _ => server,
        }
    }
}

pub struct Hub {
    pub me: Identity,
    config: Config,
    app: AppHandle,
    state: Mutex<State>,
    /// Held through each reconcile, so two never interleave their supervising.
    reconciling: Mutex<()>,
    versions: AtomicU64,
    model_changed: Notify,
    servers_changed: Notify,
    files_changed: Notify,
    /// Bumped to dial every unconnected server again now, ending the dials under way.
    dials: watch::Sender<u64>,
    /// Counts the connections made, so a wait for one misses none; bumped with the state unlocked, which its waits read.
    connections: watch::Sender<u64>,
    microphone: Arc<Microphone>,
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

    pub fn start(
        app: AppHandle,
        config: Config,
        me: Identity,
        servers: Servers,
        microphone_access: AskAccess,
    ) -> Arc<Self> {
        let denied = app.clone();
        let denied_changed = Box::new(move |is: bool| emit(&denied, MICROPHONE_DENIED_EVENT, is));
        let hub = Arc::new(Self {
            me,
            config,
            microphone: Microphone::new(microphone_access, Box::new(Capture::microphone), denied_changed),
            app,
            state: Mutex::default(),
            reconciling: Mutex::default(),
            versions: AtomicU64::new(0),
            model_changed: Notify::new(),
            servers_changed: Notify::new(),
            files_changed: Notify::new(),
            dials: watch::Sender::new(0),
            connections: watch::Sender::new(0),
        });
        tauri::async_runtime::spawn(Arc::clone(&hub).emit_models());
        tauri::async_runtime::spawn(Arc::clone(&hub).emit_servers());
        tauri::async_runtime::spawn(Arc::clone(&hub).emit_files_changed());
        tauri::async_runtime::spawn(Arc::clone(&hub).follow_account());
        for server in servers.reachable(&hub.me) {
            hub.supervise(server);
        }
        hub
    }

    /// Reaches the servers the account lists as it changes, and sends every change to each server reached.
    async fn follow_account(self: Arc<Self>) {
        let mut changes = self.me.account.changes();
        while changes.changed().await.is_ok() {
            if let Err(error) = self.reconcile() {
                eprintln!("octopus app: follow the account: {error:#}");
            }
            emit(&self.app, ACCOUNT_EVENT, self.account());
            let connected: Vec<Arc<Connection>> =
                lock(&self.state).links.values().filter_map(|link| link.connection.clone()).collect();
            for connection in connected {
                let hub = Arc::clone(&self);
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = connection.sync_account(&hub.me).await {
                        eprintln!("octopus app: sync the account: {error:#}");
                    }
                });
            }
        }
    }

    /// Reaches each server this client reaches now and stops reaching the rest.
    fn reconcile(self: &Arc<Self>) -> anyhow::Result<()> {
        let _reconciling = lock(&self.reconciling);
        let wanted = Servers::load(&self.config)?.reachable(&self.me);
        let stale: Vec<String> = lock(&self.state)
            .links
            .iter()
            .filter(|(name, link)| !wanted.iter().any(|server| server.name == **name && server.address == link.address))
            .map(|(name, _)| name.clone())
            .collect();
        for name in stale {
            self.unsupervise(&name);
        }
        for server in wanted {
            if !lock(&self.state).links.contains_key(&server.name) {
                self.supervise(server);
            }
        }
        Ok(())
    }

    /// Dials from a new endpoint from now on and ends every dial under way. An endpoint can stay deaf after the network
    /// changes under it, as when a phone moves off Wi-Fi with the app in the background.
    pub async fn redial(&self) {
        self.me.forget_dialer().await;
        self.dials.send_modify(|dials| *dials += 1);
    }

    /// For a client the OS lets run again after freezing it or cutting its network: ends the dials under way and dials
    /// every unconnected server now, closes each connection whose server misses a ping within `RESUME_PATIENCE`, and
    /// resolves once each of `servers` is reachable. When one is still unreachable `RECONNECT_PATIENCE` after the call,
    /// every server is redialled from a new endpoint, since the network may have changed meanwhile.
    pub async fn resume(&self, servers: &[String]) {
        let deadline = tokio::time::Instant::now() + RECONNECT_PATIENCE;
        self.dials.send_modify(|dials| *dials += 1);
        let connections: Vec<_> = lock(&self.state).links.values().filter_map(|link| link.connection.clone()).collect();
        let mut checks = tokio::task::JoinSet::new();
        for connection in connections {
            checks.spawn(async move { connection.check(RESUME_PATIENCE).await });
        }
        checks.join_all().await;
        if !self.until_reachable(servers, deadline).await {
            self.redial().await;
        }
    }

    /// Resolves to whether each of `servers` is reachable by `deadline`.
    async fn until_reachable(&self, servers: &[String], deadline: tokio::time::Instant) -> bool {
        let mut made = self.connections.subscribe();
        let reached = made.wait_for(|_| servers.iter().all(|server| self.reachable(server)));
        tokio::time::timeout_at(deadline, reached).await.is_ok_and(|reached| reached.is_ok())
    }

    /// Whether the server is connected, or not on the list at all.
    fn reachable(&self, server: &str) -> bool {
        lock(&self.state).links.get(server).is_none_or(Link::reachable)
    }

    pub(crate) fn account(&self) -> AccountView {
        let state = self.me.account.state();
        let me = self.me.key.public();
        let mut devices: Vec<DeviceView> = state
            .members
            .iter()
            .map(|member| DeviceView { name: member.name.clone(), key: member.key.hex(), this: member.key == me })
            .collect();
        devices.sort_by_key(|device| !device.this);
        let (id, settings) = (self.me.account.account().id().short(), self.me.account.settings().current());
        AccountView { id, member: state.is_member(&me), devices, settings }
    }

    /// A link that goes aborts its supervisor at its next await, so a step it already started may find its link gone.
    fn supervise(self: &Arc<Self>, server: Server) {
        let mut state = lock(&self.state);
        let (hub, name, address) = (Arc::clone(self), server.name.clone(), server.address.clone());
        let (resyncs, mut asked) = mpsc::unbounded_channel();
        let mut dials = self.dials.subscribe();
        let supervisor = tauri::async_runtime::spawn(async move {
            loop {
                dials.borrow_and_update();
                let dialled = tokio::time::Instant::now();
                let problem = match hub.session(&server, &mut asked, &mut dials).await {
                    Ok(closed) => closed,
                    Err(error) => format!("{error:#}"),
                };
                while let Ok(answer) = asked.try_recv() {
                    let _asker_gone = answer.send(Err(format!("{}: {problem}", hub.label(&server.name))));
                }
                hub.lost(&server.name, problem);
                tokio::select! {
                    () = tokio::time::sleep_until(dialled + RECONNECT) => {}
                    _ = dials.changed() => {}
                }
            }
        });
        state.links.insert(
            name,
            Link { address, connection: None, problem: None, since: now(), dropped: None, resyncs, supervisor },
        );
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
    async fn session(
        &self,
        server: &Server,
        asked: &mut mpsc::UnboundedReceiver<oneshot::Sender<Result<(), String>>>,
        dials: &mut watch::Receiver<u64>,
    ) -> anyhow::Result<String> {
        let reach = || async {
            let connection = Arc::new(Connection::connect(&server.address, &self.me).await?);
            let pushes = connection.pushes();
            self.resync(&server.name, &connection).await?;
            anyhow::Ok((connection, pushes))
        };
        let (connection, mut pushes) = loop {
            tokio::select! {
                reached = reach() => break reached?,
                _ = dials.changed() => {}
            }
        };
        let mut refresh = tokio::time::interval(RTT_REFRESH);
        loop {
            tokio::select! {
                closed = connection.closed() => return Ok(closed),
                push = pushes.recv() => match push {
                    Ok(Push { event: Event::AccountUpdated { account }, .. })
                        if account == self.me.account.account().id() =>
                    {
                        connection.sync_account(&self.me).await?
                    }
                    Ok(push) => self.apply(&server.name, push),
                    Err(RecvError::Lagged(_)) => self.resync(&server.name, &connection).await?,
                    Err(RecvError::Closed) => unreachable!("the connection owns its push sender"),
                },
                _ = refresh.tick() => self.servers_changed.notify_one(),
                Some(answer) = asked.recv() => {
                    let resynced = self.resync(&server.name, &connection).await;
                    let problem = resynced.as_ref().map_err(|error| format!("{}: {error:#}", self.label(&server.name)));
                    let _asker_gone = answer.send(problem.map(drop));
                    resynced?;
                }
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
        drop(state);
        self.connections.send_modify(|made| *made += 1);
        Ok(())
    }

    /// Resyncs every connected server at once, each in its own session so no push lands between its snapshot and
    /// the cache, failing with each server's problem; an unconnected one is already redialled every second.
    pub async fn refresh(&self) -> Result<(), String> {
        let answers: Vec<_> = lock(&self.state)
            .links
            .values()
            .filter(|link| link.connection.is_some())
            .filter_map(|link| {
                let (answer, answered) = oneshot::channel();
                link.resyncs.send(answer).ok().map(|()| answered)
            })
            .collect();
        let mut problems = Vec::new();
        for answered in answers {
            if let Ok(Err(problem)) = answered.await {
                problems.push(problem);
            }
        }
        if problems.is_empty() { Ok(()) } else { Err(problems.join("; ")) }
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
            link.dropped = Some(tokio::time::Instant::now());
        }
        link.problem = Some(problem);
        state.waking.retain(|at, _| at.server != server);
        self.servers_changed.notify_one();
        self.model_changed.notify_one();
    }

    /// The name this client lists the server pinned to `key` under, if it lists one.
    pub fn listed_as(&self, key: &PublicKey) -> Option<String> {
        let state = lock(&self.state);
        state
            .links
            .iter()
            .find(|(_, link)| matches!(&link.address, Address::Remote { key: listed, .. } if listed == key))
            .map(|(name, _)| name.clone())
    }

    /// Pairs with the server `invite` names, lists it in the account, and resolves to its name.
    pub async fn pair(self: &Arc<Self>, invite: &Invite) -> anyhow::Result<String> {
        let server = servers::pair(invite, &self.me).await?;
        self.reconcile()?;
        Ok(self.listed_as(&invite.server).unwrap_or(server.name))
    }

    /// Renames the server on this machine's list or in the account, and reaches it afresh under that name.
    pub(crate) fn rename_server(self: &Arc<Self>, from: &str, to: &str) -> anyhow::Result<()> {
        servers::rename(&self.config, &self.me, from, to)?;
        self.reconcile()
    }

    pub(crate) fn forget_server(self: &Arc<Self>, server: &str) -> anyhow::Result<()> {
        servers::forget(&self.config, &self.me, server)?;
        self.reconcile()
    }

    /// A fresh link to this account, held by the server on this machine when it is reached, else by the first one
    /// reached.
    pub(crate) async fn link(&self) -> anyhow::Result<(AccountLink, i64)> {
        let connection = lock(&self.state)
            .links
            .values()
            .filter_map(|link| Some((link.connection.clone()?, matches!(link.address, Address::Remote { .. }))))
            .min_by_key(|(_, remote)| *remote)
            .map(|(connection, _)| connection)
            .ok_or_else(|| anyhow::anyhow!("no server is reachable to hold the link"))?;
        let (invite, secret, expires_at) = self.me.account.update(|kept| Ok(kept.account.invite(&self.me.key)))?;
        connection.sync_account(&self.me).await?;
        let server = connection.server();
        let account = self.me.account.account().id();
        Ok((AccountLink { server: server.key, relay: server.relay.clone(), account, invite, secret }, expires_at))
    }

    pub(crate) fn remove_device(&self, key: &str) -> anyhow::Result<()> {
        let state = self.me.account.state();
        let member = state.members.iter().find(|member| member.key.hex() == key);
        let member = member.ok_or_else(|| anyhow::anyhow!("no device {key} in this account"))?;
        self.me.account.append(&self.me.key, Change::Remove { device: member.key })
    }

    /// Moves this client to a fresh account of its own, with the servers of the one it leaves or with none.
    pub(crate) fn start_afresh(&self, keep_servers: bool) -> anyhow::Result<()> {
        let fresh = Account::fresh(self.me.key.public());
        let settings = Settings::new(fresh.id());
        self.me.account.move_to(&self.me.key, fresh, &settings, &self.me.name(), keep_servers)
    }

    /// Renames the section's workspace on every server of the section, refusing before any rename when a server
    /// would split the section: one that is unreachable, or one that already holds a workspace named `to`.
    pub(crate) async fn rename_section(&self, from: &str, to: &str) -> Result<(), String> {
        let taken = |server: &str, cache: &Cache| {
            (from != to && cache.workspaces.values().any(|workspace| workspace.name == to))
                .then(|| format!("{server} already has {to}"))
        };
        let rename = |workspace: WorkspaceId| WorkspaceRename { workspace, name: to.into() };
        self.across_section(from, &format!("renamed {from} to {to}"), taken, rename).await
    }

    /// Colors the section's workspace on every server of the section, refusing before any while one is unreachable.
    pub(crate) async fn color_section(&self, section: &str, color: Color) -> Result<(), String> {
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
        let servers: Vec<String> = {
            let state = lock(&self.state);
            let holds = |(_, cache): &(&String, &Cache)| workspace_named(cache, section).is_some();
            state.caches.iter().filter(holds).map(|(server, _)| server.clone()).collect()
        };
        self.wait_out_drops(&servers).await;
        let ops = {
            let state = lock(&self.state);
            let mut ops = Vec::new();
            for (server, cache) in &state.caches {
                let Some(workspace) = workspace_named(cache, section) else { continue };
                let label = state.label(server);
                if state.links.get(server).is_none_or(|link| !link.reachable()) {
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

    pub fn task(&self, at: &TaskRef) -> Result<Task, String> {
        let state = lock(&self.state);
        let task = state.caches.get(&at.server).and_then(|cache| cache.tasks.get(&at.task));
        task.cloned().ok_or_else(|| format!("{} has no task {}", state.label(&at.server), at.task))
    }

    pub async fn call<O: Op>(&self, server: &str, op: &O) -> Result<O::Ok, String> {
        self.connected(server).await?.call(op).await.map_err(|error| self.failure(server, error))
    }

    /// The server's connection, waiting up to `RECONNECT_PATIENCE` for the next one when it has just dropped, so a call
    /// made as a connection drops goes out on the next one.
    pub async fn connected(&self, server: &str) -> Result<Arc<Connection>, String> {
        self.wait_out_drops(&[server.to_string()]).await;
        self.connection(server)
    }

    /// Waits, until at most `RECONNECT_PATIENCE` from now, for each of `servers` whose connection has just dropped.
    async fn wait_out_drops(&self, servers: &[String]) {
        let dropped: Vec<String> = {
            let state = lock(&self.state);
            let dropped_lately = |server: &&String| state.links.get(*server).is_some_and(Link::dropped_lately);
            servers.iter().filter(dropped_lately).cloned().collect()
        };
        self.until_reachable(&dropped, tokio::time::Instant::now() + RECONNECT_PATIENCE).await;
    }

    /// Whether the server runs on another machine, where its ports are reached through a tunnel.
    pub fn is_another_machine(&self, server: &str) -> bool {
        lock(&self.state).links.get(server).is_some_and(|link| matches!(link.address, Address::Remote { .. }))
    }

    /// The port a page reaches through a tunnel: a loopback port of a server on another machine.
    pub fn tunneled_port(&self, server: &str, page: &Url) -> Option<u16> {
        loopback_port(page).filter(|_| self.is_another_machine(server))
    }

    /// Carries this client's `localhost:<port>` to the same port on `server`'s machine.
    pub async fn tunnel(self: &Arc<Self>, server: &str, port: u16) -> Result<Tunnel, String> {
        let listeners = listen_on_localhost(port).await.map_err(|error| error.to_string())?;
        let (hub, dialed) = (Arc::clone(self), server.to_string());
        Ok(Tunnel::new(
            port,
            listeners,
            Arc::new(move || {
                let (hub, dialed) = (Arc::clone(&hub), dialed.clone());
                Box::pin(async move { hub.connected(&dialed).await })
            }),
        ))
    }

    fn connection(&self, server: &str) -> Result<Arc<Connection>, String> {
        let state = lock(&self.state);
        match state.links.get(server) {
            Some(link @ Link { connection: Some(connection), .. }) if link.reachable() => Ok(Arc::clone(connection)),
            Some(Link { problem, .. }) => {
                Err(format!("{} is unreachable: {}", state.label(server), problem.as_deref().unwrap_or("connecting")))
            }
            None => Err(format!("no server named {server} in the list")),
        }
    }

    pub fn model(&self) -> Model {
        let mut state = lock(&self.state);
        let State { caches, waking, .. } = &mut *state;
        waking.retain(|at, answered| {
            answered.is_none_or(|rev| caches.get(&at.server).is_some_and(|cache| cache.rev < rev))
        });
        let version = self.versions.fetch_add(1, Ordering::Relaxed);
        let tasks =
            task_views(&state.caches, &state.waking, |server, holder| state.held_here(&self.me, server, holder));
        Model { version, lists: lists(&state.caches, &state.waking), tasks }
    }

    /// Focuses the task with the account's flags for its harness, its agent waking until the server's answer reaches
    /// this client's cache.
    pub async fn focus(&self, at: TaskRef) -> Result<(), String> {
        let harness = self.task(&at)?.harness;
        let args = self.me.account.settings().current().agent_flags.get(&harness).map(AgentFlags::args);
        lock(&self.state).waking.insert(at.clone(), None);
        self.model_changed.notify_one();
        let focused = self.call(&at.server, &TaskFocus { task: at.task.clone(), args: args.unwrap_or_default() }).await;
        match &focused {
            Ok(Revised { rev }) => lock(&self.state).waking.insert(at, Some(*rev)),
            Err(_) => lock(&self.state).waking.remove(&at),
        };
        self.model_changed.notify_one();
        focused.map(drop)
    }

    pub(crate) fn servers(&self) -> Vec<ServerView> {
        let state = lock(&self.state);
        state
            .links
            .iter()
            .map(|(name, link)| {
                let (relay, key) = match &link.address {
                    Address::Local { .. } => (None, None),
                    Address::Remote { relay, key } => (Some(relay.to_string()), Some(key.hex())),
                };
                ServerView {
                    name: name.clone(),
                    label: state.label(name).to_string(),
                    relay,
                    key,
                    reachable: link.reachable(),
                    since: link.since,
                    rtt_ms: link
                        .connection
                        .as_ref()
                        .and_then(|connection| connection.rtt())
                        .map(|rtt| saturate(rtt.as_millis())),
                    problem: link.problem.clone(),
                }
            })
            .collect()
    }

    /// The listening terminals whose lock this client holds, on servers it reaches.
    fn listeners(&self) -> BTreeMap<Listener, Arc<Connection>> {
        let state = lock(&self.state);
        let mut listeners = BTreeMap::new();
        for (server, cache) in &state.caches {
            let Some(Link { connection: Some(connection), .. }) = state.links.get(server) else { continue };
            let held = cache.tasks.values().flat_map(|task| &task.panels).filter(|panel| {
                panel.listening
                    && panel.lock_holder.as_ref().is_some_and(|holder| state.held_here(&self.me, server, holder))
            });
            for panel in held {
                listeners.insert((server.clone(), panel.id.clone()), Arc::clone(connection));
            }
        }
        listeners
    }

    async fn emit_models(self: Arc<Self>) {
        loop {
            self.model_changed.notified().await;
            emit(&self.app, MODEL_EVENT, self.model());
            self.microphone.send_to(self.listeners());
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

fn workspace_named<'c>(cache: &'c Cache, name: &str) -> Option<&'c Workspace> {
    cache.workspaces.values().find(|workspace| workspace.name == name)
}
