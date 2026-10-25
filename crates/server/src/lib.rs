mod browser;
mod checkouts;
mod files;
mod listener;
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
    AgentReport, BrowserOpen, Done, Entries, FileList, FileSearch, FileWrite, HarnessSet, Matches, Offer, Paired,
    PairingForget, PairingOffer, PairingPair, Revised, SessionHello, SessionPing, SessionSync, Shown, Snapshot,
    TaskCreate, TaskCreated, TaskFocus, TaskGet, TaskMove, TermResize, TermSend, TermTakeLock, Welcome, WorkspaceAdd,
    WorkspaceCreated, WorkspaceList, Workspaces, Written,
};
use octopus_proto::pairing::Side;
use octopus_proto::types::{
    AGENT_PANEL, Agent, AgentStatus, BrowserPanel, Checkout, ENV_SERVER, ENV_TASK, Panel, PtyId, Task, TaskId, web_page,
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

impl State {
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
            directory: row.directory.clone(),
            branch: row.branch.clone(),
            focused_at: row.focused_at,
            ended_at: row.ended_at,
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
            WorkspaceAdd::NAME => call::<WorkspaceAdd>(args, |op| self.workspace_add(op)),
            WorkspaceList::NAME => {
                call::<WorkspaceList>(args, |_| Ok(Workspaces { workspaces: lock(&self.state).store.workspaces()? }))
            }
            HarnessSet::NAME => call::<HarnessSet>(args, |op| self.harness_set(op)),
            AgentReport::NAME => call::<AgentReport>(args, |op| self.agent_report(op)),
            TaskCreate::NAME => call::<TaskCreate>(args, |op| self.task_create(op)),
            TaskMove::NAME => call::<TaskMove>(args, |op| self.task_move(op)),
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
        let root =
            dunce::canonicalize(&op.root).map_err(|error| Error::invalid(format!("root {}: {error}", op.root)))?;
        let mut state = lock(&self.state);
        let (workspace, rev) = state.store.add_workspace(&op.name, &root.to_string_lossy())?;
        let id = workspace.id.clone();
        self.publish(&state, rev, Event::WorkspaceAdded { workspace });
        Ok(WorkspaceCreated { workspace: id, rev })
    }

    fn harness_set(&self, op: HarnessSet) -> Result<Revised, Error> {
        if op.argv.is_empty() {
            return Err(Error::invalid("argv is empty"));
        }
        let mut state = lock(&self.state);
        let rev = state.store.set_harness(&op.id, &Harness { argv: op.argv, install: op.install, hooks: op.hooks })?;
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
            directory: directory.to_string_lossy().to_string(),
            branch,
            focused_at: None,
            ended_at: tasks::ended_at(op.state, now()),
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

    fn task_focus(&self, op: TaskFocus) -> Result<Revised, Error> {
        let mut state = lock(&self.state);
        let mut row = state.store.task(&op.task)?;
        if !row.state.is_live() {
            return Ok(Revised { rev: row.rev });
        }
        self.watch_checkout(&mut state, &row)?;
        if row.focused_at.is_some() {
            return Ok(Revised { rev: row.rev });
        }
        let slot = self.start_agent(&mut state, &row)?;
        row.focused_at = Some(now());
        let rev = state.store.save_task(&mut row)?;
        if let Some(AgentSlot::Running { terminal, .. }) = state.agents.insert(row.id.clone(), slot)
            && let Err(error) = terminal.forget()
        {
            report!("forget the previous agent terminal of {}: {error}", row.id);
        }
        self.publish_task(&state, &row);
        Ok(Revised { rev })
    }

    fn start_agent(&self, state: &mut State, row: &TaskRow) -> Result<AgentSlot, Error> {
        let Some(harness) = state.store.harness(&row.harness)? else {
            return Ok(AgentSlot::HarnessUnknown);
        };
        let program = harness.argv.first().expect("harness.set rejects an empty argv");
        if which::which(program).is_err() {
            return Ok(AgentSlot::HarnessNotInstalled { install: harness.install });
        }
        let id = PtyId::random();
        let env = [(ENV_TASK, row.id.0.as_str()), (ENV_SERVER, self.name.as_str())];
        self.host.spawn(&id, &harness.argv, Path::new(&row.directory), &env)?;
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

    pub(crate) fn client_connected(&self, client: &str) {
        *lock(&self.clients).entry(client.to_string()).or_default() += 1;
    }

    pub(crate) fn client_disconnected(&self, client: &str) {
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
