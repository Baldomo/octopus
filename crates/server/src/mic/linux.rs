//! One pipe source per terminal on the user's sound server, PulseAudio or PipeWire's pulse server, driven by `pactl`.
//! A source's module reads its FIFO; the server writes the holder's audio into it while a program records.

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use octopus_proto::types::{MIC_FORMAT, PtyId};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::Shared;
use crate::{lock, report};

const SOURCE_PREFIX: &str = "octopus_";
/// How much audio each write to a source carries.
const TICK: Duration = Duration::from_millis(10);
/// How long the server waits for the sound server to come back once `pactl subscribe` ends.
const RESUBSCRIBE: Duration = Duration::from_secs(1);
/// A sound server slower than this to answer is treated as wedged.
const PACTL_PATIENCE: Duration = Duration::from_secs(2);
const PACTL_POLL: Duration = Duration::from_millis(5);

#[derive(Deserialize)]
struct Module {
    index: u64,
    argument: Option<String>,
}

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
}

pub struct Platform {
    inner: Arc<Inner>,
}

impl Platform {
    pub fn start(dir: &Path, _terminal_host: &Path, shared: Arc<Shared>) -> anyhow::Result<Self> {
        pactl(&["info"]).context("reach the user's sound server")?;
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let inner = Arc::new(Inner {
            fifos: dir.to_path_buf(),
            shared,
            terminals: Mutex::default(),
            feeding: Mutex::default(),
        });
        Ok(Self { inner })
    }

    pub fn prepare(&self, id: &PtyId) -> anyhow::Result<Vec<(String, String)>> {
        let mut terminals = lock(&self.inner.terminals);
        self.inner.load(id)?;
        terminals.insert(id.clone());
        let name = source(id);
        Ok(vec![("PULSE_SOURCE".to_string(), name.clone()), ("PIPEWIRE_NODE".to_string(), name)])
    }

    /// Follows the sound server from here on: until the adopted terminals are known, it would drop their sources.
    pub fn adopt(&self, terminals: &[PtyId]) {
        *lock(&self.inner.terminals) = terminals.iter().cloned().collect();
        let watcher = Arc::clone(&self.inner);
        std::thread::spawn(move || watcher.watch());
    }

    pub fn remove(&self, id: &PtyId) {
        lock(&self.inner.terminals).remove(id);
        self.inner.reconcile();
    }

    pub fn started(&self, _id: &PtyId) {}

    pub fn remote_held(&self, _terminals: HashSet<PtyId>) {}

    pub fn streaming(&self, _id: &PtyId, _streaming: bool) {}
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
        let prefix = format!("file={}/", self.fifos.display());
        let mut loaded = HashMap::new();
        for module in pactl_list::<Module>("modules")? {
            let argument = module.argument.unwrap_or_default();
            if let Some(id) = argument.split_whitespace().find_map(|word| word.strip_prefix(&prefix)) {
                loaded.insert(PtyId(id.to_string()), module.index.to_string());
            }
        }
        Ok(loaded)
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

    /// Follows the sound server's recording streams for as long as the server runs; a sound server that restarts
    /// gets every source back.
    fn watch(self: Arc<Self>) {
        loop {
            match Command::new("pactl").arg("subscribe").stdout(Stdio::piped()).stdin(Stdio::null()).spawn() {
                Ok(mut subscribed) => {
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
                Err(error) => report!("follow the sound server: {error}"),
            }
            self.shared.set_listening(HashSet::new());
            std::thread::sleep(RESUBSCRIBE);
        }
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
        self.shared.set_listening(listening.clone());
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
