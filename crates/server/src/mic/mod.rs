//! Each terminal's virtual microphone: its programs record from it, and the lock holder's audio feeds it.

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod platform;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nix::unistd::Pid;
use octopus_proto::types::{Listening, MIC_FORMAT, PtyId};

use crate::{lock, report};
use octopus_proto::Error;

use crate::Server;
use crate::client::Client;
use crate::host::HostClient;
use crate::terminals::Terminal;

/// The most audio a terminal buffers ahead of its programs; a holder sending faster loses its oldest bytes.
const BUFFERED_SECONDS: f64 = 0.25;

fn buffered_bytes() -> usize {
    (MIC_FORMAT.bytes_per_second() as f64 * BUFFERED_SECONDS) as usize
}

type Changed = Box<dyn Fn(&PtyId, Listening) + Send + Sync>;

/// The program each terminal started, which leads the terminal's session: a process belongs to the terminal whose
/// session it runs in.
pub(crate) struct Sessions {
    host: HostClient,
    leaders: Mutex<HashMap<PtyId, Pid>>,
}

impl Sessions {
    /// A terminal whose program has already exited has no leader to learn.
    fn learn(&self, id: &PtyId) {
        match self.host.program(id) {
            Ok(Some(program)) => {
                lock(&self.leaders).insert(id.clone(), Pid::from_raw(program));
            }
            Ok(None) => {}
            Err(error) => report!("find the program of {id}: {error}"),
        }
    }

    fn forget(&self, id: &PtyId) {
        lock(&self.leaders).remove(id);
    }

    pub(crate) fn terminal_of(&self, pid: Pid) -> Option<PtyId> {
        let leader = nix::unistd::getsid(Some(pid)).ok()?;
        lock(&self.leaders).iter().find(|(_, known)| **known == leader).map(|(id, _)| id.clone())
    }
}

/// What the platform shares with the server: which terminals listen, the audio waiting for each, and who holds them.
pub(crate) struct Shared {
    listening: Mutex<HashMap<PtyId, Listening>>,
    audio: Mutex<HashMap<PtyId, VecDeque<u8>>>,
    changed: Changed,
    pub(crate) sessions: Sessions,
    /// The terminals whose lock a client connected over QUIC holds.
    pub(crate) remote_held: Mutex<HashSet<PtyId>>,
}

impl Shared {
    fn new(terminal_host: &Path, changed: Changed) -> Self {
        Self {
            listening: Mutex::default(),
            audio: Mutex::default(),
            changed,
            sessions: Sessions { host: HostClient::at(terminal_host), leaders: Mutex::default() },
            remote_held: Mutex::default(),
        }
    }

    /// Takes how each terminal that listens now listens, `Yes` or `NoMicrophone`, the rest listening `No`, and reports
    /// each that changed.
    pub(crate) fn set_listening(&self, now: HashMap<PtyId, Listening>) {
        let before = std::mem::replace(&mut *lock(&self.listening), now.clone());
        for (id, _) in before.iter().filter(|(id, _)| !now.contains_key(*id)) {
            lock(&self.audio).remove(id);
            (self.changed)(id, Listening::No);
        }
        for (id, listening) in now.iter().filter(|(id, listening)| before.get(*id) != Some(*listening)) {
            if *listening != Listening::Yes {
                lock(&self.audio).remove(id);
            }
            (self.changed)(id, *listening);
        }
    }

    /// Whether the holder's audio reaches the terminal's programs.
    pub fn listening(&self, id: &PtyId) -> bool {
        lock(&self.listening).get(id) == Some(&Listening::Yes)
    }

    /// Fills `out` with the terminal's waiting audio, then silence.
    #[cfg(any(target_os = "linux", test))]
    pub fn pull(&self, id: &PtyId, out: &mut [u8]) {
        drain(&mut lock(&self.audio), id, out);
    }

    /// `pull` for a real-time thread: reports false, filling nothing, when the audio is locked elsewhere.
    #[cfg(target_os = "macos")]
    pub(crate) fn try_pull(&self, id: &PtyId, out: &mut [u8]) -> bool {
        let Some(mut audio) = unpoisoned(self.audio.try_lock()) else { return false };
        drain(&mut audio, id, out);
        true
    }

    fn push(&self, id: &PtyId, bytes: &[u8]) {
        if !self.listening(id) {
            return;
        }
        let mut audio = lock(&self.audio);
        let waiting = audio.entry(id.clone()).or_default();
        waiting.extend(bytes);
        let excess = waiting.len().saturating_sub(buffered_bytes());
        let whole_samples = excess.div_ceil(MIC_FORMAT.bytes_per_sample) * MIC_FORMAT.bytes_per_sample;
        waiting.drain(..whole_samples.min(waiting.len()));
    }
}

/// A lock taken without waiting; a poisoned one is taken all the same.
#[cfg(target_os = "macos")]
fn unpoisoned<T>(locked: std::sync::TryLockResult<T>) -> Option<T> {
    match locked {
        Ok(guard) => Some(guard),
        Err(std::sync::TryLockError::Poisoned(poison)) => Some(poison.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => None,
    }
}

fn drain(audio: &mut HashMap<PtyId, VecDeque<u8>>, id: &PtyId, out: &mut [u8]) {
    let taken = audio.get_mut(id).map_or(0, |waiting| {
        let taken = waiting.len().min(out.len());
        for (slot, byte) in out.iter_mut().zip(waiting.drain(..taken)) {
            *slot = byte;
        }
        taken
    });
    out[taken..].fill(0);
}

/// Gives back the input a stopped server's Octopus Microphone stood in for, as `octopus uninstall` does.
#[cfg(target_os = "macos")]
pub fn give_back_input(terminal_host: &Path) -> anyhow::Result<()> {
    platform::give_back_input(&directory(terminal_host))
}

/// Where a server whose terminal host listens at `terminal_host` keeps its microphones' files.
pub fn directory(terminal_host: &Path) -> PathBuf {
    PathBuf::from(format!("{}.mic", terminal_host.display()))
}

pub(crate) struct Microphones {
    shared: Arc<Shared>,
    /// None when this server gives its terminals no virtual microphone.
    platform: Option<platform::Platform>,
}

impl Microphones {
    /// A platform that cannot start leaves every terminal on the machine's own input.
    pub fn start(
        enabled: bool,
        terminal_host: &Path,
        changed: impl Fn(&PtyId, Listening) + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared::new(terminal_host, Box::new(changed)));
        let platform = enabled
            .then(|| platform::Platform::start(&directory(terminal_host), Arc::clone(&shared)))
            .and_then(|started| started.map_err(|error| report!("no virtual microphones: {error:#}")).ok());
        Self { shared, platform }
    }

    /// Readies the virtual microphone of a terminal about to start, returning the variables that point its
    /// programs at it.
    pub fn prepare(&self, id: &PtyId) -> Vec<(String, String)> {
        self.platform.as_ref().map_or_else(Vec::new, |platform| platform.prepare(id))
    }

    fn on_platform(&self, act: impl FnOnce(&platform::Platform)) {
        if let Some(platform) = &self.platform {
            act(platform);
        }
    }

    /// Keeps the virtual microphones of the running terminals a restarted server adopts, and drops the rest.
    pub fn adopt(&self, terminals: &[PtyId]) {
        self.on_platform(|platform| {
            terminals.iter().for_each(|id| self.shared.sessions.learn(id));
            platform.adopt(terminals);
        });
    }

    pub fn remove(&self, id: &PtyId) {
        self.on_platform(|platform| {
            self.shared.sessions.forget(id);
            platform.remove(id);
        });
        let others = lock(&self.shared.listening)
            .iter()
            .filter(|(known, _)| *known != id)
            .map(|(known, listening)| (known.clone(), *listening))
            .collect();
        self.shared.set_listening(others);
    }

    pub fn started(&self, id: &PtyId) {
        self.on_platform(|_| self.shared.sessions.learn(id));
    }

    pub(crate) fn remote_held(&self, terminals: HashSet<PtyId>) {
        self.on_platform(|_| *lock(&self.shared.remote_held) = terminals);
    }
}

impl Server {
    pub(crate) fn listening_changed(&self, id: &PtyId, listening: Listening) {
        let mut state = lock(&self.state);
        let task = state.terminal_by_id(id).map(|(task, _)| task);
        let changed = match listening {
            Listening::No => state.listening.remove(id).is_some(),
            heard => task.is_some() && state.listening.insert(id.clone(), heard) != Some(heard),
        };
        let Some(task) = task else { return };
        if changed && let Err(error) = self.touch_task(&mut state, &task) {
            report!("publish whether {id} is listening: {error}");
        }
    }

    /// Audio from `client` for the terminal's virtual microphone, taken only while `client` holds the lock.
    pub(crate) fn hear(&self, client: &Client, terminal: &Terminal, bytes: &[u8]) -> Result<(), Error> {
        self.check_holder(client, terminal)?;
        self.mic.shared.push(&terminal.id, bytes);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terminal_hears_its_holder_then_silence_and_never_more_than_the_buffer() {
        let shared = Shared::new(Path::new("/nowhere/host.sock"), Box::new(|_, _| {}));
        let id = PtyId("pty_1".into());
        shared.push(&id, &[1, 2]);
        shared.set_listening(HashMap::from([(id.clone(), Listening::Yes)]));
        shared.push(&id, &[3, 4]);
        let mut out = [9u8; 4];
        shared.pull(&id, &mut out);
        assert_eq!(out, [3, 4, 0, 0]);
        shared.push(&id, &vec![7u8; buffered_bytes() + 6]);
        assert_eq!(lock(&shared.audio)[&id].len(), buffered_bytes());
    }
}
