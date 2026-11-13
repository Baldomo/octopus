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

use octopus_proto::types::{MIC_FORMAT, PtyId};

use crate::{lock, report};
use octopus_proto::Error;

use crate::Server;
use crate::client::Client;
use crate::terminals::Terminal;

/// The most audio a terminal buffers ahead of its programs; a holder sending faster loses its oldest bytes.
const BUFFERED_SECONDS: f64 = 0.25;

fn buffered_bytes() -> usize {
    (MIC_FORMAT.bytes_per_second() as f64 * BUFFERED_SECONDS) as usize
}

type Changed = Box<dyn Fn(&PtyId, bool) + Send + Sync>;

/// What the platform shares with the server: which terminals listen, and the audio waiting for each.
pub(crate) struct Shared {
    listening: Mutex<HashSet<PtyId>>,
    audio: Mutex<HashMap<PtyId, VecDeque<u8>>>,
    changed: Changed,
}

impl Shared {
    /// Takes the terminals now listening, reporting each that started or stopped.
    pub(crate) fn set_listening(&self, now: HashSet<PtyId>) {
        let before = std::mem::replace(&mut *lock(&self.listening), now.clone());
        for stopped in before.difference(&now) {
            lock(&self.audio).remove(stopped);
            (self.changed)(stopped, false);
        }
        for started in now.difference(&before) {
            (self.changed)(started, true);
        }
    }

    pub fn listening(&self, id: &PtyId) -> bool {
        lock(&self.listening).contains(id)
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

/// Gives back the input a stopped server's Octopus Microphone stood in for, as `server uninstall` does.
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
    pub fn start(enabled: bool, terminal_host: &Path, changed: impl Fn(&PtyId, bool) + Send + Sync + 'static) -> Self {
        let shared =
            Arc::new(Shared { listening: Mutex::default(), audio: Mutex::default(), changed: Box::new(changed) });
        let platform = enabled
            .then(|| platform::Platform::start(&directory(terminal_host), terminal_host, Arc::clone(&shared)))
            .and_then(|started| started.map_err(|error| report!("no virtual microphones: {error:#}")).ok());
        Self { shared, platform }
    }

    /// Readies the virtual microphone of a terminal about to start, returning the variables that point its
    /// programs at it.
    pub fn prepare(&self, id: &PtyId) -> Vec<(String, String)> {
        let Some(platform) = &self.platform else { return Vec::new() };
        platform.prepare(id).unwrap_or_else(|error| {
            report!("no virtual microphone for {id}: {error:#}");
            Vec::new()
        })
    }

    fn on_platform(&self, act: impl FnOnce(&platform::Platform)) {
        if let Some(platform) = &self.platform {
            act(platform);
        }
    }

    /// Keeps the virtual microphones of the running terminals a restarted server adopts, and drops the rest.
    pub fn adopt(&self, terminals: &[PtyId]) {
        self.on_platform(|platform| platform.adopt(terminals));
    }

    pub fn remove(&self, id: &PtyId) {
        self.on_platform(|platform| platform.remove(id));
        let others = lock(&self.shared.listening).iter().filter(|known| *known != id).cloned().collect();
        self.shared.set_listening(others);
    }

    pub fn started(&self, id: &PtyId) {
        self.on_platform(|platform| platform.started(id));
    }

    /// The terminals whose lock a client connected over QUIC holds.
    pub(crate) fn remote_held(&self, terminals: HashSet<PtyId>) {
        self.on_platform(|platform| platform.remote_held(terminals));
    }
}

impl Server {
    pub(crate) fn listening_changed(&self, id: &PtyId, listening: bool) {
        let mut state = lock(&self.state);
        let task = state.terminal_by_id(id).map(|(task, _)| task);
        let changed =
            if listening { task.is_some() && state.listening.insert(id.clone()) } else { state.listening.remove(id) };
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
        let shared = Shared { listening: Mutex::default(), audio: Mutex::default(), changed: Box::new(|_, _| {}) };
        let id = PtyId("pty_1".into());
        shared.push(&id, &[1, 2]);
        shared.set_listening(HashSet::from([id.clone()]));
        shared.push(&id, &[3, 4]);
        let mut out = [9u8; 4];
        shared.pull(&id, &mut out);
        assert_eq!(out, [3, 4, 0, 0]);
        shared.push(&id, &vec![7u8; buffered_bytes() + 6]);
        assert_eq!(lock(&shared.audio)[&id].len(), buffered_bytes());
    }
}
