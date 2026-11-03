//! A task's terminals as the server sees them: the agent's and its shells, who holds each one's lock, and the ops
//! that open, close, write to and size them.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use octopus_proto::Error;
use octopus_proto::local::{self, BlockingStream};
use octopus_proto::ops::{Opened, Revised, TermClaim, TermClose, TermOpen, TermResize, TermSend, TermTakeLock};
use octopus_proto::types::{AGENT_PANEL, ENV_SERVER, ENV_TASK, HOLDER_GRACE_MS, LockHolder, Panel, PtyId, TaskId};

use crate::client::Client;
use crate::host::HostClient;
use crate::pty::{Activity, INITIAL_SIZE};
use crate::store::PanelRow;
use crate::{Server, State, files, lock, login, now, report};

const WAIT_RETRY: Duration = Duration::from_secs(1);

pub(crate) struct Shell {
    pub(crate) name: String,
    pub(crate) terminal: Arc<Terminal>,
}

/// The server's handle on a hosted terminal: the server holds the client write lock, the host holds the process
/// and its output.
pub(crate) struct Terminal {
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
pub(crate) enum Refusal {
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
    pub(crate) fn wait_exit(&self) -> Result<(), Error> {
        let waited = loop {
            match self.host.wait_exit(&self.id) {
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

    pub(crate) fn unshow(&self, client: &Client) {
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
    pub(crate) fn unshown_since(&self) -> Option<i64> {
        let screen = lock(&self.screen);
        screen.viewers.is_empty().then_some(screen.unshown_at)
    }

    pub fn holds(&self, client: &Client) -> bool {
        lock(&self.screen).holder.as_ref() == Some(client)
    }

    /// Whether `client` holds the lock and shows the terminal nowhere.
    pub(crate) fn held_unshown(&self, client: &Client) -> bool {
        let screen = lock(&self.screen);
        screen.holder.as_ref() == Some(client) && !screen.viewers.contains_key(client)
    }

    /// Frees the lock when `client` holds it, shows the terminal nowhere, and has not shown it again since `shows`
    /// was read; reports whether it did.
    pub(crate) fn free_if_unshown(&self, client: &Client, shows: u64) -> bool {
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
        self.host.resize(&self.id, cols, rows)?;
        (screen.cols, screen.rows) = (cols, rows);
        Ok(true)
    }

    pub(crate) fn check_holder(&self, client: &Client) -> Result<(), Refusal> {
        lock(&self.screen).check_holder(client)
    }

    /// Writes only while `client` still holds the lock it claimed earlier.
    pub(crate) fn write_as(&self, client: &Client, bytes: &[u8]) -> Result<(), Refusal> {
        self.check_holder(client)?;
        if !self.running() {
            return Err(Error::conflict(format!("{} has exited", self.id)).into());
        }
        let mut input = lock(&self.input);
        let mut stream = match input.take() {
            Some(stream) => stream,
            None => self.host.input(&self.id)?,
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

    pub(crate) fn reset_size(&self) -> Result<(), Error> {
        let (cols, rows) = INITIAL_SIZE;
        self.host.resize(&self.id, cols, rows)
    }

    pub(crate) fn activity(&self) -> Result<Activity, Error> {
        self.host.activity(&self.id)
    }

    pub(crate) fn terminate(&self) -> Result<(), Error> {
        self.host.terminate(&self.id)
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

impl State {
    /// Ends every terminal of the task and drops their scrollback.
    pub(crate) fn forget_terminals(&mut self, task: &TaskId) {
        self.forget_agent(task);
        for shell in self.shells.remove(task).unwrap_or_default() {
            if let Err(error) = shell.terminal.forget() {
                report!("forget the shell {} of {task}: {error}", shell.terminal.id);
            }
        }
    }

    /// The task's terminals in panel order, each with its panel's name.
    pub(crate) fn terminals(&self, task: &TaskId) -> impl Iterator<Item = (&str, &Arc<Terminal>)> {
        let agent = self.agent_terminal(task).map(|terminal| (AGENT_PANEL, terminal));
        let shells = self.shells.get(task).into_iter().flatten().map(|shell| (shell.name.as_str(), &shell.terminal));
        agent.into_iter().chain(shells)
    }

    pub(crate) fn every_terminal(&self) -> impl Iterator<Item = (&TaskId, &Arc<Terminal>)> {
        let agents = self.agents.iter().filter_map(|(task, slot)| slot.terminal().map(|terminal| (task, terminal)));
        let shells =
            self.shells.iter().flat_map(|(task, shells)| shells.iter().map(move |shell| (task, &shell.terminal)));
        agents.chain(shells)
    }

    pub(crate) fn panels(&self, task: &TaskId) -> Vec<Panel> {
        self.terminals(task)
            .map(|(name, terminal)| {
                let screen = terminal.screen();
                Panel {
                    id: terminal.id.clone(),
                    name: name.to_string(),
                    running: terminal.running(),
                    lock_holder: screen.holder.map(|holder| self.lock_holder(holder)),
                    cols: screen.cols,
                    rows: screen.rows,
                    listening: self.listening.contains(&terminal.id),
                }
            })
            .collect()
    }

    /// A paired holder is labelled with the name its key is pinned under now, or its key once it is no longer pinned.
    fn lock_holder(&self, holder: Client) -> LockHolder {
        match holder {
            Client::Local(name) => LockHolder { name, key: None },
            Client::Paired(key) => {
                let name = match self.store.client_pinned_to(&key) {
                    Ok(Some(name)) => name,
                    Ok(None) => {
                        report!("{key} holds a lock though it is no longer pinned");
                        key.to_string()
                    }
                    Err(error) => {
                        report!("look up the name {key} is pinned under: {error}");
                        key.to_string()
                    }
                };
                LockHolder { name, key: Some(key) }
            }
        }
    }

    fn find_terminal(&self, task: &TaskId, key: &str) -> Result<Arc<Terminal>, Error> {
        self.terminals(task)
            .find(|(name, terminal)| *name == key || terminal.id.0 == key)
            .map(|(_, terminal)| Arc::clone(terminal))
            .ok_or_else(|| Error::not_found(format!("panel {key} of {task}")))
    }

    pub(crate) fn terminal_by_id(&self, id: &PtyId) -> Option<(TaskId, Arc<Terminal>)> {
        self.every_terminal()
            .find(|(_, terminal)| &terminal.id == id)
            .map(|(task, terminal)| (task.clone(), Arc::clone(terminal)))
    }

    fn take_shell(&mut self, task: &TaskId, id: &PtyId) -> Option<Shell> {
        let shells = self.shells.get_mut(task)?;
        let at = shells.iter().position(|shell| &shell.terminal.id == id)?;
        Some(shells.remove(at))
    }
}

impl Server {
    pub(crate) fn touch_terminals(
        &self,
        state: &mut State,
        mut pick: impl FnMut(&Terminal) -> bool,
    ) -> Result<(), Error> {
        let picked: Vec<TaskId> =
            state.every_terminal().filter(|(_, terminal)| pick(terminal)).map(|(task, _)| task.clone()).collect();
        for task in picked {
            self.touch_task(state, &task)?;
        }
        Ok(())
    }

    /// Callers hold the state lock until the terminal joins it, so its exit never lands before it does.
    pub(crate) fn spawn_panel(
        &self,
        state: &mut State,
        panel: &PanelRow,
        argv: Vec<String>,
        directory: &Path,
        mut env: Vec<(String, String)>,
    ) -> Result<Arc<Terminal>, Error> {
        env.extend(
            [
                (ENV_TASK, panel.task.0.as_str()),
                (ENV_SERVER, self.name.as_str()),
                ("TERM", "xterm-256color"),
                ("COLORTERM", "truecolor"),
            ]
            .map(|(key, value)| (key.to_string(), value.to_string())),
        );
        env.extend(self.mic.prepare(&panel.id));
        self.host.spawn(&panel.id, argv, directory, env).inspect_err(|_| self.mic.remove(&panel.id))?;
        self.mic.started(&panel.id);
        state.store.record_panel(panel)?;
        Ok(self.adopt(&panel.task, panel.id.clone(), true))
    }

    pub(crate) fn adopt(&self, task: &TaskId, id: PtyId, running: bool) -> Arc<Terminal> {
        let terminal = Arc::new(Terminal::new(id, Arc::clone(&self.host), running));
        self.publish_on_exit(task.clone(), Arc::clone(&terminal));
        terminal
    }

    pub(crate) fn term_open(&self, op: TermOpen) -> Result<Opened, Error> {
        let shell = login::shell()?;
        let mut state = lock(&self.state);
        let row = state.store.task(&op.task)?;
        if !row.state.is_live() {
            return Err(Error::conflict(format!("{} is ended; reopen it to open a shell", row.id)));
        }
        let directory = files::directory(Path::new(&row.directory), &op.directory)?;
        let stem = shell.file_stem().unwrap_or(shell.as_os_str()).to_string_lossy().into_owned();
        let taken: HashSet<&str> = state.terminals(&op.task).map(|(name, _)| name).chain([AGENT_PANEL]).collect();
        let name = std::iter::once(stem.clone())
            .chain((2..).map(|count| format!("{stem}-{count}")))
            .find(|name| !taken.contains(name.as_str()))
            .expect("an unbounded run of names holds one the task's panels lack");
        let panel = PanelRow { id: PtyId::random(), task: op.task.clone(), name: name.clone() };
        let argv = vec![shell.to_string_lossy().into_owned(), "-l".into()];
        let terminal = self.spawn_panel(&mut state, &panel, argv, &directory, Vec::new())?;
        state.shells.entry(op.task.clone()).or_default().push(Shell { name, terminal });
        Ok(Opened { panel: panel.id, rev: self.touch_task(&mut state, &op.task)? })
    }

    pub(crate) fn term_close(&self, op: TermClose) -> Result<Revised, Error> {
        let mut state = lock(&self.state);
        let terminal = state.find_terminal(&op.task, &op.panel)?;
        if state.agent_terminal(&op.task).is_some_and(|agent| agent.id == terminal.id) {
            return Err(Error::conflict(format!("the agent panel of {} closes only with its agent", op.task)));
        }
        let shell = state.take_shell(&op.task, &terminal.id).expect("a panel other than the agent's is a shell");
        Ok(Revised { rev: self.drop_shell(&mut state, &op.task, &shell)? })
    }

    fn drop_shell(&self, state: &mut State, task: &TaskId, shell: &Shell) -> Result<u64, Error> {
        if let Err(error) = shell.terminal.forget() {
            report!("end the shell {} of {task}: {error}", shell.terminal.id);
        }
        state.store.forget_panel(&shell.terminal.id)?;
        self.touch_task(state, task)
    }

    /// An exited shell leaves its task's list; an exited agent stays, showing its scrollback, until a focus starts it
    /// again.
    fn terminal_exited(&self, state: &mut State, task: &TaskId, id: &PtyId) -> Result<(), Error> {
        if state.agent_terminal(task).is_some_and(|current| &current.id == id) {
            return self.touch_task(state, task).map(drop);
        }
        let Some(shell) = state.take_shell(task, id) else { return Ok(()) };
        self.drop_shell(state, task, &shell).map(drop)
    }

    fn publish_on_exit(&self, task: TaskId, terminal: Arc<Terminal>) {
        let server = Weak::clone(&self.me);
        std::thread::spawn(move || {
            if let Err(error) = terminal.wait_exit() {
                report!("lost the terminal host while {} ran: {error}", terminal.id);
            }
            let Some(server) = server.upgrade() else { return };
            server.mic.remove(&terminal.id);
            let mut state = lock(&server.state);
            if let Err(error) = server.terminal_exited(&mut state, &task, &terminal.id) {
                report!("publish the exit of {}: {error}", terminal.id);
            }
        });
    }

    pub(crate) fn check_holder(&self, client: &Client, terminal: &Terminal) -> Result<(), Error> {
        terminal.check_holder(client).map_err(|refusal| self.refused(terminal, refusal))
    }

    /// Words a refusal for the client, naming a holder by its label.
    fn refused(&self, terminal: &Terminal, refusal: Refusal) -> Error {
        match refusal {
            Refusal::HeldBy(holder) => Error::PtyLocked { holder: lock(&self.state).lock_holder(holder).name },
            Refusal::Free => Error::conflict(format!("this client holds no lock on {}; take it first", terminal.id)),
            Refusal::Failed(error) => error,
        }
    }

    /// Writes happen after the state lock is released: a full pty buffer blocks them. A take keeps the size.
    pub(crate) fn term_send(&self, client: &Client, op: TermSend) -> Result<Revised, Error> {
        let mut bytes = op.text.into_bytes();
        if op.enter {
            bytes.push(b'\r');
        }
        let (terminal, rev) = self.on_screen(client, &op.task, &op.panel, |terminal| {
            let screen = terminal.screen();
            if op.take_lock { terminal.take(client, screen.cols, screen.rows) } else { Ok(false) }
        })?;
        terminal.write_as(client, &bytes).map_err(|refusal| self.refused(&terminal, refusal))?;
        Ok(Revised { rev })
    }

    pub(crate) fn term_claim(&self, client: &Client, op: TermClaim) -> Result<Revised, Error> {
        let (_, rev) =
            self.on_screen(client, &op.task, &op.panel, |terminal| terminal.claim(client, op.cols, op.rows))?;
        Ok(Revised { rev })
    }

    pub(crate) fn term_take_lock(&self, client: &Client, op: TermTakeLock) -> Result<Revised, Error> {
        let (_, rev) =
            self.on_screen(client, &op.task, &op.panel, |terminal| terminal.take(client, op.cols, op.rows))?;
        Ok(Revised { rev })
    }

    pub(crate) fn term_resize(&self, client: &Client, op: TermResize) -> Result<Revised, Error> {
        let (_, rev) =
            self.on_screen(client, &op.task, &op.panel, |terminal| terminal.resize(client, op.cols, op.rows))?;
        Ok(Revised { rev })
    }

    /// Runs `change` on a panel's screen outside the state lock, since it may call the terminal host, then publishes
    /// the task when the holder or size moved. A lock `client` now holds without showing the terminal, as the CLI's,
    /// frees after the grace. A paired client forgotten while `change` ran keeps nothing it took: the forget and this
    /// check each hold the state lock, so whichever runs second frees the lock.
    fn on_screen(
        &self,
        client: &Client,
        task: &TaskId,
        panel: &str,
        change: impl FnOnce(&Terminal) -> Result<bool, Refusal>,
    ) -> Result<(Arc<Terminal>, u64), Error> {
        let terminal = lock(&self.state).find_terminal(task, panel)?;
        let changed = change(&terminal).map_err(|refusal| self.refused(&terminal, refusal))?;
        let rev = {
            let mut state = lock(&self.state);
            if let Client::Paired(key) = client
                && state.store.client_pinned_to(key)?.is_none()
            {
                if terminal.free(client) {
                    self.touch_task(&mut state, task)?;
                }
                return Err(Error::Unpaired);
            }
            if changed { self.touch_task(&mut state, task)? } else { state.store.rev()? }
        };
        if terminal.held_unshown(client) {
            self.free_after_grace(client, Arc::clone(&terminal));
        }
        Ok((terminal, rev))
    }

    /// A client shows a terminal while one of its data streams of it is open. Closing the last one frees the lock it
    /// holds there at once; losing it with the connection frees the lock after the grace.
    pub(crate) fn stopped_showing(&self, client: &Client, terminal: Arc<Terminal>, closed_cleanly: bool) {
        terminal.unshow(client);
        if closed_cleanly {
            self.free_if_unshown(client, &terminal, terminal.shows(client));
        } else {
            self.free_after_grace(client, terminal);
        }
    }

    fn free_if_unshown(&self, client: &Client, terminal: &Terminal, shows: u64) {
        let mut state = lock(&self.state);
        let Some((task, _)) = state.terminal_by_id(&terminal.id) else { return };
        if terminal.free_if_unshown(client, shows)
            && let Err(error) = self.touch_task(&mut state, &task)
        {
            report!("free {client}'s lock on {}: {error}", terminal.id);
        }
    }

    fn free_after_grace(&self, client: &Client, terminal: Arc<Terminal>) {
        let (server, client, shows) = (Weak::clone(&self.me), client.clone(), terminal.shows(client));
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(HOLDER_GRACE_MS));
            if let Some(server) = server.upgrade() {
                server.free_if_unshown(&client, &terminal, shows);
            }
        });
    }

    /// A holder that disconnects keeps its locks through the grace, so a dropped connection that comes back in time
    /// and shows its terminals again keeps them.
    pub(crate) fn client_disconnected(&self, client: &Client) {
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
        let held: Vec<Arc<Terminal>> = lock(&self.state)
            .every_terminal()
            .filter(|(_, terminal)| terminal.holds(client))
            .map(|(_, terminal)| Arc::clone(terminal))
            .collect();
        for terminal in held {
            self.free_after_grace(client, terminal);
        }
    }

    pub(crate) fn terminal(&self, id: &PtyId) -> Option<Arc<Terminal>> {
        lock(&self.state).terminal_by_id(id).map(|(_, terminal)| terminal)
    }
}
