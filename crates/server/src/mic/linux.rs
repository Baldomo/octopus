//! One pipe source per terminal on the user's sound server, PulseAudio or PipeWire's pulse server, driven by `pactl`.
//! A source's module reads its FIFO; the server writes the holder's audio into it while a program records. The
//! server follows the sound server as it comes and goes, and while there is none it watches for programs recording
//! straight from the machine's sound devices.

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use nix::unistd::Pid;
use octopus_proto::types::{Listening, MIC_FORMAT, PtyId};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::Shared;
use crate::{lock, report};

const SOURCE_PREFIX: &str = "octopus_";
/// How much audio each write to a source carries.
const TICK: Duration = Duration::from_millis(10);
/// How often the server looks for a sound server while it has none.
const RESUBSCRIBE: Duration = Duration::from_secs(1);
/// A sound server slower than this to answer is treated as wedged.
const PACTL_PATIENCE: Duration = Duration::from_secs(2);
const PACTL_POLL: Duration = Duration::from_millis(5);

#[derive(Deserialize)]
struct Source {
    index: u64,
    name: String,
}

#[derive(Deserialize)]
struct SourceOutput {
    source: u64,
}

fn source(id: &PtyId) -> String {
    format!("{SOURCE_PREFIX}{id}")
}

fn pactl(args: &[&str]) -> anyhow::Result<String> {
    let mut child = Command::new("pactl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("run pactl")?;
    let drain = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        let mut pipe = pipe.expect("pactl's output is piped");
        std::thread::spawn(move || {
            let mut text = Vec::new();
            let _cut_short = pipe.read_to_end(&mut text);
            String::from_utf8_lossy(&text).into_owned()
        })
    };
    let stdout = drain(child.stdout.take().map(|pipe| Box::new(pipe) as _));
    let stderr = drain(child.stderr.take().map(|pipe| Box::new(pipe) as _));
    let deadline = Instant::now() + PACTL_PATIENCE;
    let status = loop {
        if let Some(status) = child.try_wait().context("wait on pactl")? {
            break status;
        }
        if Instant::now() > deadline {
            let _already_exited = child.kill();
            let _reaped = child.wait();
            bail!("pactl {} took over {PACTL_PATIENCE:?}", args.join(" "));
        }
        std::thread::sleep(PACTL_POLL);
    };
    let read = |reader: std::thread::JoinHandle<String>| reader.join().expect("reading a pipe does not panic");
    let (stdout, stderr) = (read(stdout), read(stderr));
    if !status.success() {
        bail!("pactl {}: {}", args.join(" "), stderr.trim());
    }
    Ok(stdout)
}

fn pactl_list<T: DeserializeOwned>(what: &str) -> anyhow::Result<Vec<T>> {
    let listed = pactl(&["-f", "json", "list", what])?;
    serde_json::from_str(&listed).with_context(|| format!("read pactl's list of {what}"))
}

struct Inner {
    fifos: PathBuf,
    shared: Arc<Shared>,
    /// The terminals this server gives a source, locked across every change to the sound server's sources.
    terminals: Mutex<HashSet<PtyId>>,
    /// The terminals a thread is writing audio into.
    feeding: Mutex<HashSet<PtyId>>,
    reachable: AtomicBool,
}

pub(crate) struct Platform {
    inner: Arc<Inner>,
}

impl Platform {
    pub fn start(dir: &Path, shared: Arc<Shared>) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let inner = Arc::new(Inner {
            fifos: dir.to_path_buf(),
            shared,
            terminals: Mutex::default(),
            feeding: Mutex::default(),
            reachable: AtomicBool::new(false),
        });
        Ok(Self { inner })
    }

    /// The variables name the terminal's source even while the sound server is away, which makes it on its return.
    pub fn prepare(&self, id: &PtyId) -> Vec<(String, String)> {
        let mut terminals = lock(&self.inner.terminals);
        if self.inner.reachable.load(Ordering::Relaxed)
            && let Err(error) = self.inner.load(id)
        {
            report!("make the virtual microphone of {id}: {error:#}");
        }
        terminals.insert(id.clone());
        let name = source(id);
        vec![("PULSE_SOURCE".to_string(), name.clone()), ("PIPEWIRE_NODE".to_string(), name)]
    }

    /// Follows the sound server from here on: until the adopted terminals are known, it would drop their sources.
    pub fn adopt(&self, terminals: &[PtyId]) {
        *lock(&self.inner.terminals) = terminals.iter().cloned().collect();
        let watcher = Arc::clone(&self.inner);
        std::thread::spawn(move || watcher.watch());
    }

    pub fn remove(&self, id: &PtyId) {
        lock(&self.inner.terminals).remove(id);
        if self.inner.reachable.load(Ordering::Relaxed) {
            self.inner.reconcile();
        }
    }
}

impl Inner {
    fn fifo(&self, id: &PtyId) -> PathBuf {
        self.fifos.join(&id.0)
    }

    /// PulseAudio makes the FIFO itself and refuses one that exists, so a stale one goes first.
    fn load(&self, id: &PtyId) -> anyhow::Result<()> {
        let fifo = self.fifo(id);
        match std::fs::remove_file(&fifo) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(error).with_context(|| format!("remove {}", fifo.display()));
            }
            _ => {}
        }
        pactl(&[
            "load-module",
            "module-pipe-source",
            &format!("source_name={}", source(id)),
            &format!("file={}", fifo.display()),
            "format=s16le",
            &format!("rate={}", MIC_FORMAT.rate),
            &format!("channels={}", MIC_FORMAT.channels),
            "source_properties=device.description=Octopus-microphone",
        ])
        .map(drop)
    }

    /// This server's sources by terminal, each with its module: those whose FIFO lives in this server's directory.
    fn loaded(&self) -> anyhow::Result<HashMap<PtyId, String>> {
        Ok(pipe_modules(&pactl(&["list", "short", "modules"])?, &self.fifos))
    }

    /// Gives each terminal its source and drops every other source of this server's.
    fn reconcile(self: &Arc<Self>) {
        let terminals = lock(&self.terminals);
        let loaded = match self.loaded() {
            Ok(loaded) => loaded,
            Err(error) => return report!("list the virtual microphones: {error:#}"),
        };
        for (id, module) in &loaded {
            if !terminals.contains(id)
                && let Err(error) = pactl(&["unload-module", module])
            {
                report!("drop the virtual microphone of {id}: {error:#}");
            }
        }
        for id in terminals.iter().filter(|id| !loaded.contains_key(*id)) {
            if let Err(error) = self.load(id) {
                report!("restore the virtual microphone of {id}: {error:#}");
            }
        }
        drop(terminals);
        self.refresh();
    }

    /// Follows the sound server's recording streams for as long as the server runs; a sound server that starts, or
    /// restarts, gets every source back. While there is none, a remote holder's terminal recording from a sound
    /// device hears no microphone.
    fn watch(self: Arc<Self>) {
        loop {
            if pactl(&["info"]).is_ok() {
                self.reachable.store(true, Ordering::Relaxed);
                self.follow();
                self.reachable.store(false, Ordering::Relaxed);
            }
            self.shared.set_listening(self.unheard());
            std::thread::sleep(RESUBSCRIBE);
        }
    }

    /// Returns when the sound server goes.
    fn follow(self: &Arc<Self>) {
        let mut subscribed =
            match Command::new("pactl").arg("subscribe").stdout(Stdio::piped()).stdin(Stdio::null()).spawn() {
                Ok(subscribed) => subscribed,
                Err(error) => return report!("follow the sound server: {error}"),
            };
        self.reconcile();
        let events = BufReader::new(subscribed.stdout.take().expect("stdout is piped"));
        for event in events.lines() {
            match event {
                Ok(event) if event.contains("source-output") => self.refresh(),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        let _reaped = subscribed.wait();
    }

    /// The remote holders' terminals with a program recording from one of the machine's sound devices.
    fn unheard(&self) -> HashMap<PtyId, Listening> {
        let held = lock(&self.shared.remote_held).clone();
        if held.is_empty() {
            return HashMap::new();
        }
        capturing()
            .into_iter()
            .filter_map(|pid| self.shared.sessions.terminal_of(pid))
            .filter(|id| held.contains(id))
            .map(|id| (id, Listening::NoMicrophone))
            .collect()
    }

    /// Reads which sources have a recording stream, and feeds each terminal that started listening.
    fn refresh(self: &Arc<Self>) {
        let names: HashMap<u64, String> = match pactl_list::<Source>("sources") {
            Ok(sources) => sources.into_iter().map(|source| (source.index, source.name)).collect(),
            Err(error) => return report!("list the sound server's sources: {error:#}"),
        };
        let recording: HashSet<&str> = match pactl_list::<SourceOutput>("source-outputs") {
            Ok(outputs) => outputs.iter().filter_map(|output| names.get(&output.source).map(String::as_str)).collect(),
            Err(error) => return report!("list the sound server's recordings: {error:#}"),
        };
        let listening: HashSet<PtyId> =
            lock(&self.terminals).iter().filter(|id| recording.contains(source(id).as_str())).cloned().collect();
        self.shared.set_listening(listening.iter().map(|id| (id.clone(), Listening::Yes)).collect());
        for id in listening {
            if lock(&self.feeding).insert(id.clone()) {
                let feeder = Arc::clone(self);
                // A terminal that listens again as its feeder stops keeps the feeder.
                std::thread::spawn(move || {
                    loop {
                        let fed = feeder.feed(&id);
                        lock(&feeder.feeding).remove(&id);
                        if let Err(error) = fed {
                            break report!("feed the virtual microphone of {id}: {error:#}");
                        }
                        if !feeder.shared.listening(&id) || !lock(&feeder.feeding).insert(id.clone()) {
                            break;
                        }
                    }
                });
            }
        }
    }

    /// Writes the terminal's audio, or silence, into its source in real time while it listens. The FIFO opens
    /// without blocking, so a source gone meanwhile fails rather than waits.
    fn feed(&self, id: &PtyId) -> anyhow::Result<()> {
        let fifo = self.fifo(id);
        let mut pipe = OpenOptions::new()
            .write(true)
            .custom_flags(nix::libc::O_NONBLOCK)
            .open(&fifo)
            .with_context(|| format!("open {}", fifo.display()))?;
        nix::fcntl::fcntl(&pipe, nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::empty()))
            .with_context(|| format!("block on {}", fifo.display()))?;
        let mut chunk = vec![0u8; MIC_FORMAT.bytes_per_second() * TICK.as_millis() as usize / 1000];
        let mut next = Instant::now();
        while self.shared.listening(id) {
            self.shared.pull(id, &mut chunk);
            pipe.write_all(&chunk).with_context(|| format!("write {}", fifo.display()))?;
            next += TICK;
            std::thread::sleep(next.saturating_duration_since(Instant::now()));
        }
        Ok(())
    }
}

/// The modules in `listed`, `pactl list short modules`, whose FIFO lives in `fifos`, by terminal. Each line is the
/// module's index, name and argument, separated by tabs; PulseAudio's JSON leaves the index out.
fn pipe_modules(listed: &str, fifos: &Path) -> HashMap<PtyId, String> {
    let prefix = format!("file={}/", fifos.display());
    listed
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let (index, argument) = (fields.next()?, fields.nth(1)?);
            let id = argument.split_whitespace().find_map(|word| word.strip_prefix(&prefix))?;
            Some((PtyId(id.to_string()), index.to_string()))
        })
        .collect()
}

/// The processes holding a sound device's capture stream open, `/dev/snd/pcmC<card>D<device>c`.
fn capturing() -> Vec<Pid> {
    let Ok(processes) = std::fs::read_dir("/proc") else { return Vec::new() };
    processes
        .flatten()
        .filter_map(|process| process.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| {
            let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else { return false };
            fds.flatten().any(|fd| std::fs::read_link(fd.path()).is_ok_and(|target| captures(&target)))
        })
        .map(Pid::from_raw)
        .collect()
}

fn captures(device: &Path) -> bool {
    device.to_str().and_then(|path| path.strip_prefix("/dev/snd/pcmC")).is_some_and(|rest| {
        rest.strip_suffix('c').and_then(|numbers| numbers.split_once('D')).is_some_and(|(card, device)| {
            [card, device].iter().all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_finds_its_own_pipe_sources_among_the_modules() {
        let listed = "0\tmodule-device-restore\t\tn/a\n\
                      12\tmodule-pipe-source\tsource_name=octopus_pty_a file=/run/mic/pty_a format=s16le\t1\n\
                      13\tmodule-pipe-source\tsource_name=octopus_pty_b file=/other/mic/pty_b\tn/a\n";
        let loaded = pipe_modules(listed, Path::new("/run/mic"));
        assert_eq!(loaded, HashMap::from([(PtyId("pty_a".into()), "12".to_string())]));
    }

    #[test]
    fn only_a_sound_device_s_capture_stream_counts_as_recording() {
        for (path, capture) in [
            ("/dev/snd/pcmC0D0c", true),
            ("/dev/snd/pcmC12D3c", true),
            ("/dev/snd/pcmC0D0p", false),
            ("/dev/snd/controlC0", false),
            ("/dev/snd/pcmCD0c", false),
            ("/tmp/dev/snd/pcmC0D0c", false),
        ] {
            assert_eq!(captures(Path::new(path)), capture, "{path}");
        }
    }
}
