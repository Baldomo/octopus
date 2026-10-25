mod browser;
mod checkouts;
mod files;
mod listener;
mod login;
mod pairing;
mod pty;
mod service;
mod store;
mod tasks;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, Weak, mpsc};
use std::time::{SystemTime, UNIX_EPOCH};

use octopus_proto::device::{DeviceKey, PublicKey};
use octopus_proto::ops::{
    AgentReport, BrowserOpen, Clients, Done, Entries, FileList, FileSearch, FileWrite, HarnessSet, Matches, Offer,
    Paired, PairedClient, PairingForget, PairingList, PairingOffer, PairingPair, Revised, SessionHello, SessionPing,
    SessionSync, Shown, Snapshot, TaskArchive, TaskCreate, TaskCreated, TaskFocus, TaskGet, TaskMove, TaskRename,
    TaskSetSession, TermResize, TermSend, TermTakeLock, Welcome, WorkspaceAdd, WorkspaceCreated, WorkspaceList,
    WorkspaceRemove, WorkspaceRename, Workspaces, Written,
};
use octopus_proto::pairing::Side;
use octopus_proto::types::{
    AGENT_PANEL, Agent, AgentStatus, BrowserPanel, Checkout, ENV_SERVER, ENV_TASK, Panel, PtyId, SessionId, Task,
    TaskId, web_page,
};
use octopus_proto::{Error, Event, Op, Outcome, Push, Request, parse_args};
use tokio::sync::broadcast;

pub use checkouts::WORKTREE_LIFETIME_MS;
pub use listener::serve;
use pairing::Offers;
use service::{HostClient, Terminal};
pub use service::{TerminalHost, run_terminal_host};
use store::{Harness, Store, TaskRow, TerminalRow};
use tasks::{MoveEffect, apply_move};

const PUSH_BACKLOG: usize = 4096;
/// Stands for the task's session id in a harness's start and resume args.
const SESSION: &str = "{session}";

enum AgentSlot {
    Running { terminal: Arc<Terminal>, status: AgentStatus },
    HarnessNotInstalled { install: String },
    HarnessUnknown,
}

struct State {
    store: Store,
    agents: HashMap<TaskId, AgentSlot>,
    /// The store's browser panels, which every task view carries.
    browsers: HashMap<TaskId, Vec<BrowserPanel>>,
    watchers: HashMap<TaskId, notify::RecommendedWatcher>,
}

pub struct Server {
    me: Weak<Server>,
    name: String,
    device: DeviceKey,
    offers: Mutex<Offers>,
    worktrees: PathBuf,
    host: Arc<HostClient>,
    state: Mutex<State>,
    clients: Mutex<HashMap<String, usize>>,
    pushes: broadcast::Sender<Push>,
    file_changes: mpsc::Sender<(TaskId, String)>,
}

#[derive(Debug, Default)]
pub struct Sweep {
    pub removed: Vec<TaskId>,
    pub failed: Vec<(TaskId, Error)>,
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

pub(crate) fn now() -> i64 {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).expect("the clock reads after 1970");
    i64::try_from(elapsed.as_millis()).expect("milliseconds since 1970 fit in i64")
}

/// Writes one diagnostic line. The terminal host outlives the shell that launched it, and `eprintln!` panics once
/// that stderr closes.
macro_rules! report {
    ($($message:tt)*) => {{
        use std::io::Write as _;
        let _stderr_closed = writeln!(std::io::stderr(), "octopus: {}", format_args!($($message)*));
    }};
}
pub(crate) use report;

fn call<O: Op>(args: &serde_json::Value, handler: impl FnOnce(O) -> Result<O::Ok, Error>) -> Outcome {
    Outcome::from(parse_args::<O>(args).and_then(handler))
}

fn nonempty(what: &str, text: &str) -> Result<(), Error> {
    if text.trim().is_empty() { Err(Error::invalid(format!("{what} is non-empty"))) } else { Ok(()) }
}

/// A workspace root: an absolute path to a directory, as its canonical spelling.
fn directory(root: &str) -> Result<String, Error> {
    if !Path::new(root).is_absolute() {
        return Err(Error::invalid(format!("root {root} is not an absolute path")));
    }
    let canonical = dunce::canonicalize(root).map_err(|error| Error::invalid(format!("root {root}: {error}")))?;
    if !canonical.is_dir() {
        return Err(Error::invalid(format!("root {root} is not a directory")));
    }
    Ok(canonical.to_string_lossy().into_owned())
}

impl State {
    /// Ends the task's agent terminal and drops its scrollback.
    fn forget_agent(&mut self, task: &TaskId) {
        if let Some(AgentSlot::Running { terminal, .. }) = self.agents.remove(task)
            && let Err(error) = terminal.forget()
        {
            report!("forget the agent terminal of {task}: {error}");
        }
    }

    fn agent_terminal(&self, task: &TaskId) -> Option<&Arc<Terminal>> {
        match self.agents.get(task) {
            Some(AgentSlot::Running { terminal, .. }) => Some(terminal),
            _ => None,
        }
    }

    fn panels(&self, task: &TaskId) -> Vec<Panel> {
        self.agent_terminal(task)
            .map(|terminal| Panel {
                id: terminal.id.clone(),
                name: AGENT_PANEL.to_string(),
                running: terminal.running(),
                lock_holder: terminal.holder(),
            })
            .into_iter()
            .collect()
    }

    fn find_terminal(&self, task: &TaskId, key: &str) -> Result<Arc<Terminal>, Error> {
        let found = self.panels(task).into_iter().find(|panel| panel.answers_to(key));
        found
            .and_then(|panel| self.agent_terminal(task).filter(|terminal| terminal.id == panel.id).cloned())
            .ok_or_else(|| Error::not_found(format!("panel {key} of {task}")))
    }

    fn terminal_by_id(&self, id: &PtyId) -> Option<(TaskId, Arc<Terminal>)> {
        self.agents.iter().find_map(|(task, slot)| match slot {
            AgentSlot::Running { terminal, .. } if &terminal.id == id => Some((task.clone(), Arc::clone(terminal))),
            _ => None,
        })
    }

    fn view(&self, row: &TaskRow) -> Task {
        let agent = match self.agents.get(&row.id) {
            None => Agent::NotStarted,
            Some(AgentSlot::Running { terminal, status }) if terminal.running() => Agent::Running { status: *status },
            Some(AgentSlot::Running { .. }) => Agent::Exited,
            Some(AgentSlot::HarnessNotInstalled { install }) => Agent::HarnessNotInstalled { install: install.clone() },
            Some(AgentSlot::HarnessUnknown) => Agent::HarnessUnknown,
        };
        Task {
            id: row.id.clone(),
            workspace: row.workspace.clone(),
            parent: row.parent.clone(),
            title: row.title.clone(),
            state: row.state,
            checkout: row.checkout,
            harness: row.harness.clone(),
            session: row.session.clone(),
            directory: row.directory.clone(),
            branch: row.branch.clone(),
            focused_at: row.focused_at,
            ended_at: row.ended_at,
            archived_at: row.archived_at,
            rev: row.rev,
            agent,
            panels: self.panels(&row.id),
            browsers: self.browsers.get(&row.id).cloned().unwrap_or_default(),
        }
    }
}

impl Server {
    /// Reattaches the hosted terminals the store records and ends the ones it doesn't.
    pub fn open(name: &str, store: &Path, worktrees: &Path, host: &TerminalHost) -> Result<Arc<Self>, Error> {
        let mut store = Store::open(store)?;
        let device = store.device_key()?;
        let browsers = store.browsers()?;
        let host = Arc::new(HostClient::reach_or_start(host)?);
        let mut hosted: HashMap<PtyId, bool> =
            host.list()?.into_iter().map(|hosted| (hosted.id, hosted.running)).collect();
        let recorded = store.reattach(&hosted.keys().cloned().collect())?;
        let (pushes, _) = broadcast::channel(PUSH_BACKLOG);
        let (file_changes, changed) = mpsc::channel::<(TaskId, String)>();
        let server = Arc::new_cyclic(|me| Self {
            me: me.clone(),
            name: name.to_string(),
            device,
            offers: Mutex::default(),
            worktrees: worktrees.to_path_buf(),
            host,
            state: Mutex::new(State { store, agents: HashMap::new(), browsers, watchers: HashMap::new() }),
            clients: Mutex::new(HashMap::new()),
            pushes,
            file_changes,
        });
        for row in recorded {
            let running = hosted.remove(&row.id).expect("reattach keeps only hosted terminals");
            let slot = server.adopt(&row.task, row.id, running);
            lock(&server.state).agents.insert(row.task, slot);
        }
        for orphan in hosted.into_keys() {
            report!("ending terminal {orphan}, which this server's store does not record");
            server.host.forget(&orphan)?;
        }
        // Watcher callbacks only send here: a callback that owned the server could drop it on the watcher's own thread.
        let publisher = Weak::clone(&server.me);
        std::thread::spawn(move || {
            for (task, path) in changed {
                let Some(server) = publisher.upgrade() else { break };
                server.file_changed(&task, &path);
            }
        });
        Ok(server)
    }

    /// The key this server proves to every client over QUIC.
    pub fn device(&self) -> &DeviceKey {
        &self.device
    }

    pub(crate) fn paired_name(&self, client: &PublicKey) -> Result<Option<String>, Error> {
        lock(&self.state).store.client_pinned_to(client)
    }

    /// Pins the client's key under its name once it proves an unspent code.
    pub(crate) fn pair(&self, client: &PublicKey, op: PairingPair) -> Result<Paired, Error> {
        if op.client.is_empty() {
            return Err(Error::invalid("a client name is non-empty"));
        }
        let server = self.device.public();
        let mut state = lock(&self.state);
        state.store.refuse_namesake(&op.client, client)?;
        let code = lock(&self.offers).redeem(now(), &op.proof, &server, client).ok_or(Error::CodeRefused)?;
        state.store.pin_client(&op.client, client)?;
        Ok(Paired { server: self.name.clone(), proof: code.prove(Side::Server, &server, client) })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Push> {
        self.pushes.subscribe()
    }

    pub fn handle(&self, client: &str, request: &Request) -> Outcome {
        let args = &request.args;
        match request.op.as_str() {
            SessionHello::NAME => call::<SessionHello>(args, |_| self.welcome()),
            SessionSync::NAME => call::<SessionSync>(args, |_| self.snapshot()),
            SessionPing::NAME => call::<SessionPing>(args, |_| Ok(Done {})),
            PairingForget::NAME => {
                call::<PairingForget>(args, |op| lock(&self.state).store.forget_client(&op.client).map(|()| Done {}))
            }
            PairingOffer::NAME => call::<PairingOffer>(args, |_| Ok::<Offer, Error>(lock(&self.offers).offer(now()))),
            PairingList::NAME => call::<PairingList>(args, |_| self.pairing_list()),
            WorkspaceAdd::NAME => call::<WorkspaceAdd>(args, |op| self.workspace_add(op)),
            WorkspaceList::NAME => {
                call::<WorkspaceList>(args, |_| Ok(Workspaces { workspaces: lock(&self.state).store.workspaces()? }))
            }
            WorkspaceRename::NAME => call::<WorkspaceRename>(args, |op| self.workspace_rename(op)),
            WorkspaceRemove::NAME => call::<WorkspaceRemove>(args, |op| self.workspace_remove(op)),
            HarnessSet::NAME => call::<HarnessSet>(args, |op| self.harness_set(op)),
            AgentReport::NAME => call::<AgentReport>(args, |op| self.agent_report(op)),
            TaskCreate::NAME => call::<TaskCreate>(args, |op| self.task_create(op)),
            TaskMove::NAME => call::<TaskMove>(args, |op| self.task_move(op)),
            TaskRename::NAME => call::<TaskRename>(args, |op| self.task_rename(op)),
            TaskSetSession::NAME => call::<TaskSetSession>(args, |op| self.task_set_session(op)),
            TaskArchive::NAME => call::<TaskArchive>(args, |op| self.task_archive(op)),
            TaskFocus::NAME => call::<TaskFocus>(args, |op| self.task_focus(op)),
            TaskGet::NAME => call::<TaskGet>(args, |op| {
                let state = lock(&self.state);
                Ok(state.view(&state.store.task(&op.task)?))
            }),
            TermSend::NAME => call::<TermSend>(args, |op| self.term_send(client, op)),
            TermTakeLock::NAME => call::<TermTakeLock>(args, |op| self.term_take_lock(client, op)),
            TermResize::NAME => call::<TermResize>(args, |op| self.term_resize(client, op)),
            FileList::NAME => call::<FileList>(args, |op| {
                Ok(Entries { entries: files::list(&self.checkout_dir(&op.task)?, &op.path)? })
            }),
            FileWrite::NAME => call::<FileWrite>(args, |op| {
                Ok(Written { bytes: files::write(&self.checkout_dir(&op.task)?, &op.path, op.content.as_bytes())? })
            }),
            BrowserOpen::NAME => call::<BrowserOpen>(args, |op| self.browser_open(op)),
            FileSearch::NAME => call::<FileSearch>(args, |op| {
                Ok(Matches { matches: files::search_literal(&self.checkout_dir(&op.task)?, &op.pattern) })
            }),
            op => Outcome::Err(Error::UnknownOp { op: op.to_string() }),
        }
    }

    /// Callers hold the state lock, so pushes leave in rev order.
    fn publish(&self, _state: &State, rev: u64, event: Event) {
        let _no_subscribers = self.pushes.send(Push { rev, event });
    }

    fn publish_task(&self, state: &State, row: &TaskRow) {
        self.publish(state, row.rev, Event::TaskUpdated { task: Box::new(state.view(row)) });
    }

    fn touch_task(&self, state: &mut State, task: &TaskId) -> Result<u64, Error> {
        let row = state.store.touch_task(task)?;
        self.publish_task(state, &row);
        Ok(row.rev)
    }

    fn welcome(&self) -> Result<Welcome, Error> {
        Ok(Welcome { server: self.name.clone(), rev: lock(&self.state).store.rev()? })
    }

    fn snapshot(&self) -> Result<Snapshot, Error> {
        let state = lock(&self.state);
        let tasks = state.store.tasks()?.iter().map(|row| state.view(row)).collect();
        Ok(Snapshot { rev: state.store.rev()?, workspaces: state.store.workspaces()?, tasks })
    }

    fn workspace_add(&self, op: WorkspaceAdd) -> Result<WorkspaceCreated, Error> {
        nonempty("a workspace name", &op.name)?;
        let root = directory(&op.root)?;
        let mut state = lock(&self.state);
        let (workspace, rev) = state.store.add_workspace(&op.name, &root)?;
        let id = workspace.id.clone();
        self.publish(&state, rev, Event::WorkspaceUpdated { workspace });
        Ok(WorkspaceCreated { workspace: id, rev })
    }

    fn workspace_rename(&self, op: WorkspaceRename) -> Result<Revised, Error> {
        nonempty("a workspace name", &op.name)?;
        let mut state = lock(&self.state);
        let (workspace, rev) = state.store.rename_workspace(&op.workspace, &op.name)?;
        self.publish(&state, rev, Event::WorkspaceUpdated { workspace });
        Ok(Revised { rev })
    }

    /// Runs under the state lock throughout, so no task starts in the workspace while it goes.
    fn workspace_remove(&self, op: WorkspaceRemove) -> Result<Revised, Error> {
        let mut state = lock(&self.state);
        let workspace = state.store.workspace(&op.workspace)?;
        let tasks = state.store.workspace_tasks(&workspace.id)?;
        let live = tasks.iter().filter(|task| task.state.is_live()).count();
        if live > 0 {
            return Err(Error::conflict(format!("{} has {live} live tasks; end them first", workspace.name)));
        }
        // A process still holding a worktree open would keep Windows from removing it.
        for task in &tasks {
            state.watchers.remove(&task.id);
            state.browsers.remove(&task.id);
            state.forget_agent(&task.id);
        }
        let root = PathBuf::from(&workspace.root);
        for task in tasks.iter().filter(|task| task.checkout == Checkout::Worktree) {
            let directory = PathBuf::from(&task.directory);
            if directory.exists() {
                checkouts::remove(&root, &directory)?;
            }
        }
        let (rerooted, rev) = state.store.remove_workspace(&workspace.id)?;
        for row in &rerooted {
            self.publish_task(&state, row);
        }
        self.publish(&state, rev, Event::WorkspaceRemoved { workspace: workspace.id });
        Ok(Revised { rev })
    }

    fn pairing_list(&self) -> Result<Clients, Error> {
        let pinned = lock(&self.state).store.clients()?;
        let connected = lock(&self.clients);
        let clients = pinned
            .into_iter()
            .map(|(name, seen_at)| PairedClient { connected: connected.contains_key(&name), name, seen_at })
            .collect();
        Ok(Clients { clients })
    }

    fn harness_set(&self, op: HarnessSet) -> Result<Revised, Error> {
        if op.argv.is_empty() {
            return Err(Error::invalid("argv is empty"));
        }
        let harness =
            Harness { argv: op.argv, start: op.start, resume: op.resume, install: op.install, hooks: op.hooks };
        let mut state = lock(&self.state);
        let rev = state.store.set_harness(&op.id, &harness)?;
        self.publish(&state, rev, Event::HarnessUpdated { harness: op.id });
        Ok(Revised { rev })
    }

    fn agent_report(&self, op: AgentReport) -> Result<Revised, Error> {
        let mut state = lock(&self.state);
        let row = state.store.task(&op.task)?;
        if !state.store.harness(&row.harness)?.is_some_and(|harness| harness.hooks) {
            return Err(Error::conflict(format!("harness {} declares no hooks", row.harness)));
        }
        match state.agents.get_mut(&row.id) {
            Some(AgentSlot::Running { terminal, status }) if terminal.running() => *status = op.status,
            _ => return Err(Error::conflict(format!("the agent of {} is not running", row.id))),
        }
        Ok(Revised { rev: self.touch_task(&mut state, &row.id)? })
    }

    fn task_create(&self, op: TaskCreate) -> Result<TaskCreated, Error> {
        if let Some(session) = &op.session {
            nonempty("a session id", &session.0)?;
        }
        let workspace = {
            let state = lock(&self.state);
            if let Some(parent) = &op.parent {
                state.store.task(parent)?;
            }
            state.store.workspace(&op.workspace)?
        };
        let id = TaskId::random();
        let root = PathBuf::from(&workspace.root);
        let (directory, branch) = match op.checkout {
            Checkout::Root => (root, None),
            Checkout::Worktree => {
                let directory = self.worktrees.join(&workspace.name).join(&id.0);
                let branch = checkouts::branch_for(&root, &op.title, &id)?;
                checkouts::create(&root, &directory, &branch)?;
                (directory, Some(branch))
            }
        };
        let mut state = lock(&self.state);
        let row = state.store.create_task(TaskRow {
            id,
            workspace: op.workspace,
            parent: op.parent,
            title: op.title,
            state: op.state,
            checkout: op.checkout,
            harness: op.harness,
            session: op.session,
            directory: directory.to_string_lossy().to_string(),
            branch,
            focused_at: None,
            ended_at: tasks::ended_at(op.state, now()),
            archived_at: None,
            rev: 0,
        })?;
        self.publish_task(&state, &row);
        Ok(TaskCreated { task: row.id, rev: row.rev })
    }

    fn task_move(&self, op: TaskMove) -> Result<Revised, Error> {
        let mut state = lock(&self.state);
        let mut row = state.store.task(&op.task)?;
        let reopening = !row.state.is_live() && op.state.is_live();
        if reopening && let Some(branch) = &row.branch {
            let directory = PathBuf::from(&row.directory);
            if !directory.exists() {
                let root = PathBuf::from(state.store.workspace(&row.workspace)?.root);
                checkouts::create(&root, &directory, branch)?;
            }
        }
        let effect = apply_move(&mut row, op.state, now());
        let rev = state.store.save_task(&mut row)?;
        if reopening {
            state.forget_agent(&row.id);
        }
        if effect == MoveEffect::StopProcesses {
            state.watchers.remove(&row.id);
            if let Some(terminal) = state.agent_terminal(&row.id)
                && let Err(error) = terminal.terminate()
            {
                report!("stop the agent of {}: {error}", row.id);
            }
        }
        self.publish_task(&state, &row);
        Ok(Revised { rev })
    }

    fn task_rename(&self, op: TaskRename) -> Result<Revised, Error> {
        nonempty("a task title", &op.title)?;
        let mut state = lock(&self.state);
        let mut row = state.store.task(&op.task)?;
        row.title = op.title;
        let rev = state.store.save_task(&mut row)?;
        self.publish_task(&state, &row);
        Ok(Revised { rev })
    }

    fn task_set_session(&self, op: TaskSetSession) -> Result<Revised, Error> {
        nonempty("a session id", &op.session.0)?;
        let mut state = lock(&self.state);
        let mut row = state.store.task(&op.task)?;
        row.session = Some(op.session);
        row.focused_at = None;
        let rev = state.store.save_task(&mut row)?;
        state.forget_agent(&row.id);
        self.publish_task(&state, &row);
        Ok(Revised { rev })
    }

    fn task_archive(&self, op: TaskArchive) -> Result<Revised, Error> {
        let mut state = lock(&self.state);
        let mut row = state.store.task(&op.task)?;
        if row.state.is_live() {
            return Err(Error::conflict(format!("{} is live; end it before archiving it", row.id)));
        }
        row.archived_at = Some(now());
        let rev = state.store.save_task(&mut row)?;
        self.publish_task(&state, &row);
        Ok(Revised { rev })
    }

    /// The login shell runs outside the state lock, so a slow profile stalls no other op; the task is read again
    /// after it, in case another focus or a move landed meanwhile.
    fn task_focus(&self, op: TaskFocus) -> Result<Revised, Error> {
        let (directory, harness) = {
            let mut state = lock(&self.state);
            let row = state.store.task(&op.task)?;
            if !row.state.is_live() {
                return Ok(Revised { rev: row.rev });
            }
            self.watch_checkout(&mut state, &row)?;
            if row.focused_at.is_some() {
                return Ok(Revised { rev: row.rev });
            }
            (PathBuf::from(&row.directory), state.store.harness(&row.harness)?)
        };
        let started = match harness {
            Some(harness) => Some((harness, login::resolve(&directory)?)),
            None => None,
        };
        let mut state = lock(&self.state);
        let mut row = state.store.task(&op.task)?;
        if !row.state.is_live() || row.focused_at.is_some() {
            return Ok(Revised { rev: row.rev });
        }
        let slot = match started {
            Some((harness, env)) => self.start_agent(&mut state, &mut row, harness, env)?,
            None => AgentSlot::HarnessUnknown,
        };
        if matches!(slot, AgentSlot::Running { .. }) {
            row.focused_at = Some(now());
        }
        let rev = state.store.save_task(&mut row)?;
        state.agents.insert(row.id.clone(), slot);
        self.publish_task(&state, &row);
        Ok(Revised { rev })
    }

    /// Looks the program up on the agent's own PATH, so a harness counts as installed exactly when its agent can run.
    fn start_agent(
        &self,
        state: &mut State,
        row: &mut TaskRow,
        harness: Harness,
        mut env: Vec<(String, String)>,
    ) -> Result<AgentSlot, Error> {
        let directory = PathBuf::from(&row.directory);
        let (program, args) = harness.argv.split_first().expect("harness.set rejects an empty argv");
        let Ok(program) = which::which_in(program, login::search_path(&env), &directory) else {
            return Ok(AgentSlot::HarnessNotInstalled { install: harness.install });
        };
        let session_args = if row.session.is_some() { &harness.resume } else { &harness.start };
        if session_args.iter().any(|arg| arg.contains(SESSION)) {
            row.session.get_or_insert_with(|| SessionId(uuid::Uuid::new_v4().to_string()));
        }
        let session = row.session.as_ref().map(|session| session.0.as_str()).unwrap_or_default();
        let argv = std::iter::once(program.to_string_lossy().into_owned())
            .chain(args.iter().cloned())
            .chain(session_args.iter().map(|arg| arg.replace(SESSION, session)))
            .collect();
        env.extend(
            [
                (ENV_TASK, row.id.0.as_str()),
                (ENV_SERVER, self.name.as_str()),
                ("TERM", "xterm-256color"),
                ("COLORTERM", "truecolor"),
            ]
            .map(|(key, value)| (key.to_string(), value.to_string())),
        );
        let id = PtyId::random();
        self.host.spawn(&id, argv, &directory, env)?;
        state.store.record_terminal(&TerminalRow { id: id.clone(), task: row.id.clone() })?;
        Ok(self.adopt(&row.id, id, true))
    }

    fn adopt(&self, task: &TaskId, id: PtyId, running: bool) -> AgentSlot {
        let terminal = Arc::new(Terminal::new(id, Arc::clone(&self.host), running));
        self.publish_on_exit(task.clone(), Arc::clone(&terminal));
        AgentSlot::Running { terminal, status: AgentStatus::Idle }
    }

    fn publish_on_exit(&self, task: TaskId, terminal: Arc<Terminal>) {
        let server = Weak::clone(&self.me);
        std::thread::spawn(move || {
            if let Err(error) = terminal.wait_exit() {
                report!("lost the terminal host while {} ran: {error}", terminal.id);
            }
            let Some(server) = server.upgrade() else { return };
            let mut state = lock(&server.state);
            if state.agent_terminal(&task).is_some_and(|current| current.id == terminal.id)
                && let Err(error) = server.touch_task(&mut state, &task)
            {
                report!("publish the exit of {}: {error}", terminal.id);
            }
        });
    }

    fn browser_open(&self, op: BrowserOpen) -> Result<Shown, Error> {
        web_page(&op.url).map_err(Error::invalid)?;
        let mut state = lock(&self.state);
        state.store.task(&op.task)?;
        let panels = state.browsers.get(&op.task).map(Vec::as_slice).unwrap_or_default();
        let page = BrowserPanel { panel: browser::panel(panels, op.panel)?, url: op.url };
        let row = state.store.show_page(&op.task, &page)?;
        let panels = state.browsers.entry(op.task).or_default();
        match panels.iter_mut().find(|known| known.panel == page.panel) {
            Some(known) => known.url = page.url.clone(),
            None => panels.push(page.clone()),
        }
        self.publish_task(&state, &row);
        Ok(Shown { panel: page.panel, rev: row.rev })
    }

    fn checkout_dir(&self, task: &TaskId) -> Result<PathBuf, Error> {
        Ok(PathBuf::from(lock(&self.state).store.task(task)?.directory))
    }

    pub(crate) fn read_file(&self, task: &TaskId, path: &str) -> Result<Vec<u8>, Error> {
        files::read(&self.checkout_dir(task)?, path)
    }

    fn watch_checkout(&self, state: &mut State, row: &TaskRow) -> Result<(), Error> {
        use notify::Watcher;
        if state.watchers.contains_key(&row.id) {
            return Ok(());
        }
        let directory = PathBuf::from(&row.directory);
        let (changes, task, root) = (self.file_changes.clone(), row.id.clone(), directory.clone());
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
            Ok(event) => {
                for path in event.paths {
                    let Ok(relative) = path.strip_prefix(&root) else { continue };
                    if relative.components().all(|component| component.as_os_str() != ".git") {
                        let _server_gone = changes.send((task.clone(), files::wire_path(relative)));
                    }
                }
            }
            Err(error) => report!("watching {}: {error}", root.display()),
        })
        .map_err(Error::internal)?;
        watcher
            .watch(&directory, notify::RecursiveMode::Recursive)
            .map_err(|error| Error::internal(format!("watch {}: {error}", directory.display())))?;
        state.watchers.insert(row.id.clone(), watcher);
        Ok(())
    }

    fn file_changed(&self, task: &TaskId, path: &str) {
        let state = lock(&self.state);
        match state.store.rev() {
            Ok(rev) => self.publish(&state, rev, Event::FileChanged { task: task.clone(), path: path.to_string() }),
            Err(error) => report!("file change in {task}: {error}"),
        }
    }

    /// The branch of a removed worktree stays.
    pub fn remove_expired_worktrees(&self, now: i64) -> Result<Sweep, Error> {
        let cutoff = now - WORKTREE_LIFETIME_MS;
        let expired = lock(&self.state).store.worktrees_ended_by(cutoff)?;
        let mut sweep = Sweep::default();
        for row in expired {
            let directory = PathBuf::from(&row.directory);
            let root = {
                let state = lock(&self.state);
                let current = state.store.task(&row.id)?;
                if current.ended_at.is_none_or(|ended_at| ended_at > cutoff) || !directory.exists() {
                    continue;
                }
                state.store.workspace(&row.workspace).map(|workspace| PathBuf::from(workspace.root))
            };
            match root.and_then(|root| checkouts::remove(&root, &directory)) {
                Ok(()) => sweep.removed.push(row.id),
                Err(error) => sweep.failed.push((row.id, error)),
            }
        }
        Ok(sweep)
    }

    fn term_send(&self, client: &str, op: TermSend) -> Result<Revised, Error> {
        let mut bytes = op.text.into_bytes();
        if op.enter {
            bytes.push(b'\r');
        }
        let (terminal, rev) = self.claim_terminal(client, &op.task, &op.panel, op.take_lock)?;
        terminal.write_as(client, &bytes)?;
        Ok(Revised { rev })
    }

    fn term_take_lock(&self, client: &str, op: TermTakeLock) -> Result<Revised, Error> {
        Ok(Revised { rev: self.claim_terminal(client, &op.task, &op.panel, true)?.1 })
    }

    /// Writes happen after the state lock is released: a full pty buffer blocks them.
    fn claim_terminal(
        &self,
        client: &str,
        task: &TaskId,
        panel: &str,
        steal: bool,
    ) -> Result<(Arc<Terminal>, u64), Error> {
        let mut state = lock(&self.state);
        let terminal = state.find_terminal(task, panel)?;
        let rev =
            if terminal.claim(client, steal)? { self.touch_task(&mut state, task)? } else { state.store.rev()? };
        Ok((terminal, rev))
    }

    pub(crate) fn pty_input(&self, client: &str, id: &PtyId, bytes: &[u8]) -> Result<(), Error> {
        let terminal = {
            let mut state = lock(&self.state);
            let (task, terminal) = state.terminal_by_id(id).ok_or_else(|| Error::not_found(id))?;
            if terminal.claim(client, false)? {
                self.touch_task(&mut state, &task)?;
            }
            terminal
        };
        terminal.write_as(client, bytes)
    }

    pub(crate) fn terminal(&self, id: &PtyId) -> Option<Arc<Terminal>> {
        lock(&self.state).terminal_by_id(id).map(|(_, terminal)| terminal)
    }

    fn term_resize(&self, client: &str, op: TermResize) -> Result<Revised, Error> {
        let (terminal, rev) = {
            let state = lock(&self.state);
            (state.find_terminal(&op.task, &op.panel)?, state.store.rev()?)
        };
        terminal.resize(client, op.cols, op.rows)?;
        Ok(Revised { rev })
    }

    fn saw(&self, client: &str) {
        if let Err(error) = lock(&self.state).store.client_seen(client, now()) {
            report!("record when {client} was seen: {error}");
        }
    }

    pub(crate) fn client_connected(&self, client: &str) {
        self.saw(client);
        *lock(&self.clients).entry(client.to_string()).or_default() += 1;
    }

    pub(crate) fn client_disconnected(&self, client: &str) {
        self.saw(client);
        {
            let mut clients = lock(&self.clients);
            let count = clients.get_mut(client).expect("every disconnect follows a connect");
            *count -= 1;
            if *count > 0 {
                return;
            }
            clients.remove(client);
        }
        let mut state = lock(&self.state);
        let mut released = Vec::new();
        for (task, slot) in &state.agents {
            if let AgentSlot::Running { terminal, .. } = slot
                && terminal.release(client)
            {
                released.push(task.clone());
            }
        }
        for task in released {
            if let Err(error) = self.touch_task(&mut state, &task) {
                report!("release {client}'s lock on {task}: {error}");
            }
        }
    }
}
