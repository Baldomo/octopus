//! This machine's server, which every launch checks and fixes: registered, running the installed binary, and
//! answering with at least this version.

use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use nix::fcntl::{Flock, FlockArg};
use octopus_client::servers::{Address, Config, Server, Servers};
use octopus_proto::local::{self, Answer};
use octopus_proto::remote;

use crate::ERROR_PREFIX;
use crate::install::{self, Registration, ServerRun, Service};

/// How long a server on its socket has to answer one ask.
const PATIENCE: Duration = Duration::from_secs(1);
/// The rest between asks while the fixed server starts.
const ANSWER_POLL: Duration = Duration::from_millis(100);
/// How long the fixed server has to answer once the service manager starts it.
const ANSWER_DEADLINE: Duration = Duration::from_secs(30);
/// The client name the repair says hello as.
const CLIENT: &str = "repair";

/// A release version, `major.minor.patch`, ordered as numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Version([u64; 3]);

impl Version {
    fn parse(text: &str) -> Option<Self> {
        let mut parts = text.trim().split('.').map(|part| part.parse::<u64>().ok());
        let version = Self([parts.next()??, parts.next()??, parts.next()??]);
        parts.next().is_none().then_some(version)
    }

    fn this() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).expect("cargo versions are major.minor.patch")
    }

    /// What `binary --version` prints, `octopus <version>`.
    fn of(binary: &Path) -> Option<Self> {
        let output = std::process::Command::new(binary).arg("--version").output().ok()?;
        Self::parse(String::from_utf8_lossy(&output.stdout).strip_prefix("octopus ")?)
    }

    /// The version a server answers with; one from before `server.version` answers with none.
    fn answered(answer: &Answer) -> Option<Self> {
        answer.version.as_deref().and_then(Self::parse)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [major, minor, patch] = self.0;
        write!(formatter, "{major}.{minor}.{patch}")
    }
}

/// The check this machine's server failed.
#[derive(Debug, PartialEq)]
enum Fault {
    Legacy,
    Unregistered,
    NoBinary,
    Silent,
    OldServer(Option<Version>),
}

impl fmt::Display for Fault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Legacy => write!(formatter, "an earlier install's service runs it"),
            Self::Unregistered => write!(formatter, "no service runs it"),
            Self::NoBinary => write!(formatter, "its service doesn't run the installed binary as this install does"),
            Self::Silent => write!(formatter, "it doesn't answer"),
            Self::OldServer(Some(version)) => {
                write!(formatter, "it answers as {version}, older than {}", Version::this())
            }
            Self::OldServer(None) => write!(formatter, "it answers with no version"),
        }
    }
}

/// The directory this machine's server keeps everything in, `~/.octopus` within the account's home.
pub struct Octopus {
    home: PathBuf,
    dir: PathBuf,
}

impl Octopus {
    pub fn find() -> anyhow::Result<Self> {
        let home = install::account_home()?;
        Ok(Self { dir: install::octopus_dir(&home), home })
    }

    fn socket(&self) -> PathBuf {
        self.dir.join("server.sock")
    }

    pub fn log(&self) -> PathBuf {
        self.dir.join("server.log")
    }

    fn terminal_host(&self) -> PathBuf {
        self.dir.join("host.sock")
    }

    /// Holds the repair lock until the returned guard drops, so launches at once fix the server one at a time.
    fn lock(&self) -> anyhow::Result<Flock<File>> {
        let (file, path) = self.lock_file()?;
        Flock::lock(file, FlockArg::LockExclusive).map_err(|(_, error)| anyhow!("lock {}: {error}", path.display()))
    }

    /// The repair lock, or None while another launch holds it.
    fn try_lock(&self) -> anyhow::Result<Option<Flock<File>>> {
        let (file, path) = self.lock_file()?;
        match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(locked) => Ok(Some(locked)),
            Err((_, nix::errno::Errno::EWOULDBLOCK)) => Ok(None),
            Err((_, error)) => Err(anyhow!("lock {}: {error}", path.display())),
        }
    }

    fn lock_file(&self) -> anyhow::Result<(File, PathBuf)> {
        std::fs::create_dir_all(&self.dir).with_context(|| format!("create {}", self.dir.display()))?;
        let path = self.dir.join("repair.lock");
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("open {}", path.display()))?;
        Ok((file, path))
    }

    /// The name this machine's client lists the server on its socket under.
    fn listed(&self) -> anyhow::Result<Option<String>> {
        let address = Address::Local { socket: self.socket() };
        let servers = Servers::load(&Config::user()?)?.servers;
        Ok(servers.into_iter().find(|server| server.address == address).map(|server| server.name))
    }

    /// This machine's server as its service runs it, `octopus server start`. It goes by the name this machine's
    /// client lists it under, else the computer's.
    pub fn server(&self) -> anyhow::Result<ServerRun> {
        let name = match self.listed()? {
            Some(name) => name,
            None => octopus_client::servers::machine_name()?,
        };
        Ok(ServerRun {
            name,
            socket: self.socket(),
            store: self.dir.join("store.db"),
            worktrees: self.dir.join("worktrees"),
            terminal_host: self.terminal_host(),
            udp: ([0, 0, 0, 0], 0).into(),
            relay: remote::N0_EU_RELAY.parse().expect("n0's relay is a URL"),
            microphone: true,
        })
    }
}

/// This machine's server as a launch finds it.
struct Machine<'a> {
    octopus: &'a Octopus,
    service: Service,
    /// What the server on its socket answers.
    answer: Option<Answer>,
    /// The name this machine's client lists the server under.
    listed: Option<String>,
    /// The units earlier installs registered it under, by the name it answers with and the one this client lists.
    legacy: Vec<(String, PathBuf)>,
}

impl<'a> Machine<'a> {
    /// None when this launch manages no server: on macOS, a binary outside any app.
    fn find(octopus: &'a Octopus) -> anyhow::Result<Option<Self>> {
        let Some(service) = Service::read(&octopus.home)? else { return Ok(None) };
        let answer = local::ask(&octopus.socket(), CLIENT, PATIENCE);
        let listed = octopus.listed()?;
        let mut names: Vec<&String> = answer.iter().map(|answer| &answer.server).chain(&listed).collect();
        names.dedup();
        let legacy = names
            .into_iter()
            .filter_map(|name| install::legacy_unit(&octopus.home, name).map(|unit| (name.clone(), unit)))
            .collect();
        Ok(Some(Self { octopus, service, answer, listed, legacy }))
    }

    /// Whether a service, this install's or an earlier one's, runs the server.
    fn installed(&self) -> bool {
        !self.legacy.is_empty() || self.service.registered()
    }

    /// Lists the server under the computer's name, unless this client already lists a server on its socket, and
    /// returns the name it is listed under.
    fn list(&self) -> anyhow::Result<String> {
        if let Some(name) = &self.listed {
            return Ok(name.clone());
        }
        let config = Config::user()?;
        let mut servers = Servers::load(&config)?;
        let name = octopus_client::servers::machine_name()?;
        servers.add(Server { name: name.clone(), address: Address::Local { socket: self.octopus.socket() } })?;
        servers.save(&config)?;
        Ok(name)
    }

    /// The first check, in order, that the server fails. A server answering with a newer version than this binary
    /// is repaired by that version's launches, which read its answers as this older binary may not.
    fn check(&self) -> Option<Fault> {
        let this = Version::this();
        let answered = self.answer.as_ref().and_then(Version::answered);
        if answered.is_some_and(|version| version > this) {
            return None;
        }
        if !self.legacy.is_empty() {
            return Some(Fault::Legacy);
        }
        if !self.service.registered() {
            return Some(Fault::Unregistered);
        }
        if self.service.binary().is_none() {
            return Some(Fault::NoBinary);
        }
        if self.answer.is_none() {
            return Some(Fault::Silent);
        }
        answered.is_none_or(|version| version < this).then_some(Fault::OldServer(answered))
    }

    /// Retires the units of earlier installs, registers the server from the installed binary when it is newer than
    /// this one and from this one otherwise, and waits until it answers with at least that version. A server the
    /// service manager sees exit first fails the fix with the last error line it wrote to the log, and one still
    /// silent at the deadline fails it too.
    fn fix(&self) -> anyhow::Result<Registration> {
        for (name, unit) in &self.legacy {
            install::retire(name, unit)?;
        }
        let this = Version::this();
        let installed = self.service.binary().map(|binary| (Version::of(&binary), binary));
        let (expected, source) = match installed {
            Some((Some(version), binary)) if version > this => (version, binary),
            _ => (this, install::binary::this()?),
        };
        let program = self.service.put(&source)?;
        let log = self.octopus.log();
        let logged = std::fs::metadata(&log).map_or(0, |written| written.len());
        let registration = install::install(&self.service, &program)?;
        let started = Instant::now();
        loop {
            let answer = local::ask(&self.octopus.socket(), CLIENT, PATIENCE);
            let answered = answer.as_ref().and_then(Version::answered);
            if answered.is_some_and(|version| version >= expected) {
                return Ok(registration);
            }
            if install::exited()? {
                bail!(last_error(&log, logged).unwrap_or_else(|| "the server exited before it answered".into()));
            }
            if started.elapsed() >= ANSWER_DEADLINE {
                match answer {
                    Some(_) => bail!(
                        "the server answered as {}, not {expected}",
                        answered.map_or_else(|| "a version from before versions".into(), |version| version.to_string())
                    ),
                    None => bail!("the server didn't answer within {} seconds", ANSWER_DEADLINE.as_secs()),
                }
            }
            std::thread::sleep(ANSWER_POLL);
        }
    }
}

/// Holding the repair lock, finds the server again, as another launch may have fixed it meanwhile, and fixes it
/// when a check still fails, telling `fixing` which.
fn fix_held(octopus: &Octopus, fixing: impl FnOnce(&Fault)) -> anyhow::Result<()> {
    let Some(machine) = Machine::find(octopus)? else { return Ok(()) };
    if let Some(fault) = machine.check() {
        fixing(&fault);
        machine.fix()?;
    }
    Ok(())
}

/// The last error line the server wrote to `log` past its first `from` bytes.
fn last_error(log: &Path, from: u64) -> Option<String> {
    let mut file = std::fs::File::open(log).ok()?;
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut written = Vec::new();
    file.read_to_end(&mut written).ok()?;
    String::from_utf8_lossy(&written).lines().rev().find_map(|line| line.strip_prefix(ERROR_PREFIX)).map(str::to_string)
}

/// The app's launch: lists this machine's server, and returns its fix when a check fails, which the window runs
/// again on every retry.
#[cfg(feature = "desktop")]
pub fn app() -> anyhow::Result<Option<octopus_desktop::Install>> {
    let octopus = Octopus::find()?;
    let Some(machine) = Machine::find(&octopus)? else { return Ok(None) };
    let server = machine.list()?;
    if machine.check().is_none() {
        return Ok(None);
    }
    let log = octopus.log();
    Ok(Some(octopus_desktop::Install {
        server,
        first: !machine.installed(),
        log,
        run: Box::new(move || {
            let _locked = octopus.lock()?;
            fix_held(&octopus, |_| {})
        }),
    }))
}

/// A CLI launch: fixes this machine's server when it is installed and a check fails, saying so on stderr. Under a
/// HOME other than the account's, no service is this machine's.
pub fn cli() {
    let Ok(octopus) = Octopus::find() else { return };
    let repaired = Machine::find(&octopus).and_then(|machine| {
        let Some(machine) = machine else { return Ok(()) };
        if !machine.installed() || machine.check().is_none() {
            return Ok(());
        }
        let Some(_locked) = octopus.try_lock()? else {
            eprintln!("octopus: another launch is repairing this machine's server");
            return Ok(());
        };
        fix_held(&octopus, |fault| eprintln!("octopus: repairing this machine's server: {fault}"))
    });
    if let Err(error) = repaired {
        eprintln!("octopus: repair this machine's server: {error:#}");
    }
}

fn manages_none() -> anyhow::Error {
    anyhow!("on macOS the octopus app manages this machine's server; open it")
}

/// `octopus install`: lists this machine's server and fixes it whatever the checks find.
pub fn install() -> anyhow::Result<Registration> {
    let octopus = Octopus::find()?;
    let _locked = octopus.lock()?;
    let machine = Machine::find(&octopus)?.ok_or_else(manages_none)?;
    machine.list()?;
    machine.fix().map_err(|error| anyhow!("install this machine's server: {error:#}"))
}

/// `octopus uninstall`: unregisters this machine's server, and any unit an earlier install left. On macOS the input
/// the Octopus Microphone stood in for comes back, and the plug-in stays for `octopus microphone uninstall`.
pub fn uninstall() -> anyhow::Result<Registration> {
    let octopus = Octopus::find()?;
    let _locked = octopus.lock()?;
    let machine = Machine::find(&octopus)?.ok_or_else(manages_none)?;
    for (name, unit) in &machine.legacy {
        install::retire(name, unit)?;
    }
    let registration = install::uninstall(&machine.service)?;
    #[cfg(target_os = "macos")]
    if let Err(error) = octopus_server::give_back_input(&octopus.terminal_host()) {
        eprintln!("octopus: give back the default input: {error:#}");
    }
    Ok(registration)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_order_as_numbers_and_refuse_anything_else() {
        assert!(Version::parse("0.10.0") > Version::parse("0.9.9"));
        assert_eq!(Version::parse("1.2.3\n"), Some(Version([1, 2, 3])));
        for text in ["", "1.2", "1.2.3.4", "1.2.x", "v1.2.3"] {
            assert_eq!(Version::parse(text), None, "{text:?}");
        }
    }
}
