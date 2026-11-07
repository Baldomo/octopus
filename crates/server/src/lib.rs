mod accounts;
mod agents;
mod browser;
mod checkouts;
mod client;
mod emulator;
mod files;
mod harness;
mod host;
mod listener;
mod login;
mod mic;
mod pairing;
mod pty;
mod store;
mod tasks;
mod terminals;
mod workspaces;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak, mpsc};

use octopus_proto::account::AccountId;
use octopus_proto::device::{DeviceKey, PublicKey};
pub(crate) use octopus_proto::lock;
pub use octopus_proto::now;
use octopus_proto::ops::{
    AccountSync, AgentReport, BrowserClose, BrowserMove, BrowserOpen, BrowserReport, Done, Entries, FileCopy,
    FileCreate, FileDelete, FileList, FileMove, FileSearch, FileWrite, HarnessSet, Matches, Offer, PairingForget,
    PairingList, PairingOffer, SessionHello, SessionPing, SessionSync, Snapshot, TaskArchive, TaskCreate, TaskFocus,
    TaskGet, TaskMove, TaskRename, TaskSetParent, TaskSetSession, TermClaim, TermClose, TermOpen, TermResize, TermSend,
    TermTakeLock, Welcome, WorkspaceAdd, WorkspaceList, WorkspaceRemove, WorkspaceRename, WorkspaceSetColor,
    Workspaces, Written,
};
use octopus_proto::remote::{Connection, RelayUrl};
use octopus_proto::types::{AGENT_PANEL, AgentStatus, BrowserPanel, PtyId, TaskId};
use octopus_proto::{Error, Event, Op, Outcome, Push, Request, parse_args};
use tokio::sync::broadcast;

pub use agents::AGENT_RECLAIM_MS;
use agents::AgentSlot;
pub use checkouts::{Sweep, WORKTREE_LIFETIME_MS};
use client::Client;
use host::HostClient;
pub use host::{TerminalHost, run_terminal_host};
pub use listener::serve;
use mic::Microphones;
#[cfg(target_os = "macos")]
pub use mic::give_back_input;
use pairing::Offers;
use store::{Store, TaskRow};
use terminals::Shell;

const PUSH_BACKLOG: usize = 4096;

struct State {
    store: Store,
    agents: HashMap<TaskId, AgentSlot>,
    /// Each task's shell panels, in the order they opened.
    shells: HashMap<TaskId, Vec<Shell>>,
    /// The store's browser panels, which every task view carries.
    browsers: HashMap<TaskId, Vec<BrowserPanel>>,
    watchers: HashMap<TaskId, notify::RecommendedWatcher>,
    /// The terminals a program records from the virtual microphone of.
    listening: HashSet<PtyId>,
    trusted: HashMap<AccountId, accounts::Trusted>,
}

pub struct Server {
    me: Weak<Server>,
    name: String,
    device: DeviceKey,
    relay: RelayUrl,
    offers: Mutex<Offers>,
    worktrees: PathBuf,
    /// The `octopus` binary, through which an agent's hooks report.
    octopus: PathBuf,
    host: Arc<HostClient>,
    state: Mutex<State>,
    clients: Mutex<HashMap<Client, usize>>,
    connections: Mutex<HashMap<PublicKey, Vec<Connection>>>,
    mic: Microphones,
    pushes: broadcast::Sender<Push>,
    file_changes: mpsc::Sender<(TaskId, String)>,
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

impl Server {
    /// Reattaches the hosted terminals the store records and ends the ones it doesn't. With `microphone`, every
    /// terminal gets a virtual microphone its lock holder feeds.
    pub fn open(
        name: &str,
        store: &Path,
        worktrees: &Path,
        host: &TerminalHost,
        relay: RelayUrl,
        microphone: bool,
    ) -> Result<Arc<Self>, Error> {
        let mut store = Store::open(store)?;
        let device = store.device_key()?;
        let browsers = store.browsers()?;
        let trusted = accounts::load(store.accounts()?);
        let (host_program, host_address) = (host.program.clone(), host.address.clone());
        let host = Arc::new(HostClient::reach_or_start(host)?);
        let mut hosted: HashMap<PtyId, bool> =
            host.list()?.into_iter().map(|hosted| (hosted.id, hosted.running)).collect();
        let recorded = store.reattach(&hosted.keys().cloned().collect())?;
        let (pushes, _) = broadcast::channel(PUSH_BACKLOG);
        let (file_changes, changed) = mpsc::channel::<(TaskId, String)>();
        let (listening_changes, listening_changed) = mpsc::channel::<(PtyId, bool)>();
        let mic = Microphones::start(microphone, &host_address, move |id, listening| {
            let _server_gone = listening_changes.send((id.clone(), listening));
        });
        let server = Arc::new_cyclic(|me| Self {
            me: me.clone(),
            name: name.to_string(),
            device,
            relay,
            offers: Mutex::default(),
            worktrees: worktrees.to_path_buf(),
            octopus: host_program,
            host,
            state: Mutex::new(State {
                store,
                agents: HashMap::new(),
                shells: HashMap::new(),
                browsers,
                watchers: HashMap::new(),
                listening: HashSet::new(),
                trusted,
            }),
            clients: Mutex::new(HashMap::new()),
            connections: Mutex::new(HashMap::new()),
            mic,
            pushes,
            file_changes,
        });
        let mut adopted = Vec::new();
        let mut state = lock(&server.state);
        for row in recorded {
            let running = hosted.remove(&row.id).expect("reattach keeps only hosted terminals");
            if running {
                adopted.push(row.id.clone());
            }
            let terminal = server.adopt(&row.task, row.id, running);
            if running && let Err(error) = terminal.reset_size() {
                report!("reset the size of {}: {error}", terminal.id);
            }
            if row.name == AGENT_PANEL {
                state.agents.insert(row.task, AgentSlot::Spawned { terminal, status: AgentStatus::Idle, since: now() });
            } else {
                state.shells.entry(row.task).or_default().push(Shell { name: row.name, terminal });
            }
        }
        drop(state);
        for orphan in hosted.into_keys() {
            report!("ending terminal {orphan}, which this server's store does not record");
            server.host.forget(&orphan)?;
        }
        server.mic.adopt(&adopted);
        let listener = Weak::clone(&server.me);
        std::thread::spawn(move || {
            for (id, listening) in listening_changed {
                let Some(server) = listener.upgrade() else { break };
                server.listening_changed(&id, listening);
            }
        });
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

    pub fn device(&self) -> &DeviceKey {
        &self.device
    }

    pub fn relay(&self) -> &RelayUrl {
        &self.relay
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Push> {
        self.pushes.subscribe()
    }

    pub fn handle(&self, client: &Client, request: &Request) -> Outcome {
        let args = &request.args;
        match request.op.as_str() {
            SessionHello::NAME => call::<SessionHello>(args, |_| self.welcome()),
            SessionSync::NAME => call::<SessionSync>(args, |_| self.snapshot()),
            SessionPing::NAME => call::<SessionPing>(args, |_| Ok(Done {})),
            PairingForget::NAME => call::<PairingForget>(args, |op| self.pairing_forget(op)),
            PairingOffer::NAME => call::<PairingOffer>(args, |_| Ok::<Offer, Error>(self.offer())),
            PairingList::NAME => call::<PairingList>(args, |_| self.pairing_list()),
            AccountSync::NAME => call::<AccountSync>(args, |op| self.account_sync(client, op)),
            WorkspaceAdd::NAME => call::<WorkspaceAdd>(args, |op| self.workspace_add(op)),
            WorkspaceList::NAME => {
                call::<WorkspaceList>(args, |_| Ok(Workspaces { workspaces: lock(&self.state).store.workspaces()? }))
            }
            WorkspaceRename::NAME => call::<WorkspaceRename>(args, |op| self.workspace_rename(op)),
            WorkspaceSetColor::NAME => call::<WorkspaceSetColor>(args, |op| self.workspace_set_color(op)),
            WorkspaceRemove::NAME => call::<WorkspaceRemove>(args, |op| self.workspace_remove(op)),
            HarnessSet::NAME => call::<HarnessSet>(args, |op| self.harness_set(op)),
            AgentReport::NAME => call::<AgentReport>(args, |op| self.agent_report(op)),
            TaskCreate::NAME => call::<TaskCreate>(args, |op| self.task_create(op)),
            TaskMove::NAME => call::<TaskMove>(args, |op| self.task_move(op)),
            TaskSetParent::NAME => call::<TaskSetParent>(args, |op| self.task_set_parent(op)),
            TaskRename::NAME => call::<TaskRename>(args, |op| self.task_rename(op)),
            TaskSetSession::NAME => call::<TaskSetSession>(args, |op| self.task_set_session(op)),
            TaskArchive::NAME => call::<TaskArchive>(args, |op| self.task_archive(op)),
            TaskFocus::NAME => call::<TaskFocus>(args, |op| self.task_focus(op)),
            TaskGet::NAME => call::<TaskGet>(args, |op| {
                let state = lock(&self.state);
                Ok(state.view(&state.store.task(&op.task)?))
            }),
            TermOpen::NAME => call::<TermOpen>(args, |op| self.term_open(op)),
            TermClose::NAME => call::<TermClose>(args, |op| self.term_close(op)),
            TermSend::NAME => call::<TermSend>(args, |op| self.term_send(client, op)),
            TermClaim::NAME => call::<TermClaim>(args, |op| self.term_claim(client, op)),
            TermTakeLock::NAME => call::<TermTakeLock>(args, |op| self.term_take_lock(client, op)),
            TermResize::NAME => call::<TermResize>(args, |op| self.term_resize(client, op)),
            FileList::NAME => call::<FileList>(args, |op| {
                Ok(Entries { entries: files::list(&self.checkout_dir(&op.task)?, &op.path)? })
            }),
            FileWrite::NAME => call::<FileWrite>(args, |op| {
                Ok(Written { bytes: files::write(&self.checkout_dir(&op.task)?, &op.path, op.content.as_bytes())? })
            }),
            FileCreate::NAME => call::<FileCreate>(args, |op| {
                files::create(&self.checkout_dir(&op.task)?, &op.path, op.kind).map(|()| Done {})
            }),
            FileMove::NAME => call::<FileMove>(args, |op| {
                files::rename(&self.checkout_dir(&op.task)?, &op.from, &op.to).map(|()| Done {})
            }),
            FileCopy::NAME => call::<FileCopy>(args, |op| {
                files::copy(&self.checkout_dir(&op.task)?, &op.from, &op.to).map(|()| Done {})
            }),
            FileDelete::NAME => call::<FileDelete>(args, |op| {
                files::delete(&self.checkout_dir(&op.task)?, &op.path, op.trash).map(|()| Done {})
            }),
            BrowserOpen::NAME => call::<BrowserOpen>(args, |op| self.browser_open(op)),
            BrowserReport::NAME => call::<BrowserReport>(args, |op| self.browser_report(op)),
            BrowserClose::NAME => call::<BrowserClose>(args, |op| self.browser_close(op)),
            BrowserMove::NAME => call::<BrowserMove>(args, |op| self.browser_move(op)),
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

    /// Every change of a lock touches its task, so whether a remote client holds one is decided here.
    fn touch_task(&self, state: &mut State, task: &TaskId) -> Result<u64, Error> {
        let row = state.store.touch_task(task)?;
        self.publish_task(state, &row);
        let held = state
            .every_terminal()
            .filter(|(_, terminal)| matches!(terminal.screen().holder, Some(Client::Paired(_))))
            .map(|(_, terminal)| terminal.id.clone())
            .collect();
        self.mic.remote_held(held);
        Ok(row.rev)
    }

    fn welcome(&self) -> Result<Welcome, Error> {
        let rev = lock(&self.state).store.rev()?;
        Ok(Welcome { server: self.name.clone(), rev })
    }

    fn snapshot(&self) -> Result<Snapshot, Error> {
        let state = lock(&self.state);
        let tasks = state.store.tasks()?.iter().map(|row| state.view(row)).collect();
        Ok(Snapshot { rev: state.store.rev()?, workspaces: state.store.workspaces()?, tasks })
    }
}
