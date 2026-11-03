//! Registers `octopus server run` with the platform's user service manager, started at login.

use std::net::{Ipv4Addr, SocketAddr};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, anyhow, bail};
use octopus_client::servers::{Address, Config, Server, Servers};
use octopus_proto::local;
use octopus_proto::remote::{self, RelayUrl};
use serde::Serialize;

#[derive(clap::Args)]
pub struct ServerRun {
    /// The server's name, which agents in its terminals see as OCTOPUS_SERVER.
    #[arg(long)]
    pub name: String,
    /// The Unix socket clients on this machine connect to.
    #[arg(long)]
    pub socket: PathBuf,
    /// The SQLite file the server keeps its state in.
    #[arg(long)]
    pub store: PathBuf,
    /// Where task worktrees live: <worktrees>/<workspace>/<task>.
    #[arg(long)]
    pub worktrees: PathBuf,
    /// Socket of the terminal host, the process that keeps this server's terminals running across restarts;
    /// started if nothing answers there.
    #[arg(long)]
    pub terminal_host: PathBuf,
    /// The UDP address clients on other machines reach this server on directly, e.g. 0.0.0.0:7300, which takes IPv6
    /// on the same port too.
    #[arg(long)]
    pub udp: SocketAddr,
    /// The iroh relay this server is homed on, which clients on other machines dial it through: one of n0's,
    /// e.g. https://euc1-1.relay.n0.iroh.link./, or one of your own.
    #[arg(long, value_parser = remote::relay)]
    pub relay: RelayUrl,
    /// Give each terminal a virtual microphone that its lock holder feeds.
    #[arg(long, action = clap::ArgAction::Set)]
    pub microphone: bool,
}

impl ServerRun {
    /// A server that dialled its own socket for its host would wait on itself forever. Compared without case,
    /// as macOS resolves names.
    pub fn check(&self) -> anyhow::Result<()> {
        let (socket, host) = (self.socket.display().to_string(), self.terminal_host.display().to_string());
        if socket.eq_ignore_ascii_case(&host) {
            bail!("--socket and --terminal-host must differ");
        }
        Ok(())
    }

    fn flags(&self) -> [(&'static str, String); 8] {
        [
            ("--name", self.name.clone()),
            ("--socket", self.socket.display().to_string()),
            ("--store", self.store.display().to_string()),
            ("--worktrees", self.worktrees.display().to_string()),
            ("--terminal-host", self.terminal_host.display().to_string()),
            ("--udp", self.udp.to_string()),
            ("--relay", self.relay.to_string()),
            ("--microphone", self.microphone.to_string()),
        ]
    }
}

pub struct Service {
    id: String,
    run: ServerRun,
    program: PathBuf,
    log: PathBuf,
}

#[derive(Serialize)]
pub struct Registration {
    pub service: String,
    pub unit: PathBuf,
}

#[derive(Serialize)]
pub struct Installed {
    #[serde(flatten)]
    pub registration: Registration,
    pub microphone: microphone::Outcome,
}

/// Every service manager accepts ASCII letters, digits, '-', '_' and '.', so each other character becomes '-'.
fn service_id(name: &str) -> anyhow::Result<String> {
    if name.is_empty() {
        bail!("--name must not be empty");
    }
    let safe = |c: char| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' };
    Ok(format!("octopus-{}", name.chars().map(safe).collect::<String>()))
}

fn absolute(path: PathBuf) -> anyhow::Result<PathBuf> {
    std::path::absolute(&path).with_context(|| format!("resolve {}", path.display()))
}

impl Service {
    /// Service managers start in their own working directory, so every path is made absolute. A service listens
    /// beyond loopback, since a loopback address leaves clients on other machines the relay alone.
    pub fn new(run: ServerRun, program: PathBuf, log: PathBuf) -> anyhow::Result<Self> {
        if run.udp.ip().is_loopback() {
            bail!("--udp {} is loopback; install with --udp 0.0.0.0:{}", run.udp, run.udp.port());
        }
        let id = service_id(&run.name)?;
        let run = ServerRun {
            name: run.name,
            socket: absolute(run.socket)?,
            store: absolute(run.store)?,
            worktrees: absolute(run.worktrees)?,
            terminal_host: absolute(run.terminal_host)?,
            udp: run.udp,
            relay: run.relay,
            microphone: run.microphone,
        };
        run.check()?;
        let service = Self { id, run, program: absolute(program)?, log: absolute(log)? };
        let mut words = service.argv().into_iter().chain([service.log.display().to_string()]);
        if let Some(word) = words.find(|word| word.chars().any(char::is_control)) {
            bail!("{word:?} holds a control character, which no unit file can carry");
        }
        Ok(service)
    }

    fn argv(&self) -> Vec<String> {
        let mut argv = vec![self.program.display().to_string(), "server".into(), "run".into()];
        for (flag, value) in self.run.flags() {
            argv.extend([flag.to_string(), value]);
        }
        argv
    }
}

#[cfg(any(target_os = "macos", test))]
fn xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(any(target_os = "macos", test))]
fn plist_word(word: &str) -> String {
    format!("\t\t<string>{}</string>\n", xml(word))
}

/// `AbandonProcessGroup` keeps launchd from ending the terminal host along with the server.
#[cfg(any(target_os = "macos", test))]
pub fn launchd_plist(service: &Service) -> String {
    let arguments: String = service.argv().iter().map(|argument| plist_word(argument)).collect();
    let (label, log) = (&service.id, xml(&service.log.display().to_string()));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
{arguments}	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<true/>
	<key>AbandonProcessGroup</key>
	<true/>
	<key>StandardOutPath</key>
	<string>{log}</string>
	<key>StandardErrorPath</key>
	<string>{log}</string>
</dict>
</plist>
"#
    )
}

#[cfg(any(target_os = "linux", test))]
fn specifiers(text: &str) -> String {
    text.replace('%', "%%")
}

#[cfg(any(target_os = "linux", test))]
fn systemd_word(word: &str) -> String {
    format!("\"{}\"", specifiers(word).replace('\\', "\\\\").replace('"', "\\\"").replace('$', "$$"))
}

/// Each escape `systemd_word` writes is one marker character and the character it stands for.
#[cfg(any(target_os = "linux", test))]
fn systemd_argument(spelled: &str) -> String {
    let mut argument = String::new();
    let mut characters = spelled.chars();
    while let Some(character) = characters.next() {
        match character {
            '%' | '$' | '\\' => argument.extend(characters.next()),
            _ => argument.push(character),
        }
    }
    argument
}

#[cfg(any(target_os = "macos", test))]
fn plist_argument(spelled: &str) -> String {
    spelled.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// How a unit format spells `server run`'s arguments: each as a word, words joined by a separator.
struct Spelling {
    word: fn(&str) -> String,
    /// The argument a word holds, its escapes undone.
    argument: fn(&str) -> String,
    separator: &'static str,
}

#[cfg(any(target_os = "macos", test))]
const PLIST: Spelling = Spelling { word: plist_word, argument: plist_argument, separator: "" };

#[cfg(any(target_os = "linux", test))]
const SYSTEMD: Spelling = Spelling { word: systemd_word, argument: systemd_argument, separator: " " };

/// Text no argument holds, which shows where a word puts its argument.
const MARK: &str = "\u{0}";

/// Where the value after `flag` sits in `text`: the span of its word, and the argument the word holds.
fn flag_value(text: &str, flag: &str, spelling: &Spelling) -> Option<(Range<usize>, String)> {
    let flag = (spelling.word)(flag) + spelling.separator;
    let marked = (spelling.word)(MARK);
    let (prefix, suffix) = marked.split_once(MARK).expect("a word wraps its text");
    let word_start = text.find(&flag)? + flag.len();
    text[word_start..].starts_with(prefix).then_some(())?;
    let value_start = word_start + prefix.len();
    let value_end = value_start + text[value_start..].find(suffix)?;
    Some((word_start..value_end + suffix.len(), (spelling.argument)(&text[value_start..value_end])))
}

/// The unit `text` as this binary would write it: a loopback `--udp` address becomes every IPv4 address on its port,
/// and a unit without `--microphone` gains `--microphone true` after its relay. None for a unit this binary would
/// write the same.
fn upgraded_unit(text: &str, spelling: &Spelling) -> Option<String> {
    let word = spelling.word;
    let mut unit = text.to_string();
    if let Some((span, address)) = flag_value(text, "--udp", spelling)
        && let Ok(address) = address.parse::<SocketAddr>()
        && address.ip().is_loopback()
    {
        unit.replace_range(span, &word(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, address.port())).to_string()));
    }
    if flag_value(&unit, "--microphone", spelling).is_none()
        && let Some((span, _)) = flag_value(&unit, "--relay", spelling)
    {
        unit.insert_str(span.end, &[String::new(), word("--microphone"), word("true")].join(spelling.separator));
    }
    (unit != text).then_some(unit)
}

/// `KillMode=process` stops the server alone, so its terminal host and every agent outlive a restart.
#[cfg(any(target_os = "linux", test))]
pub fn systemd_unit(service: &Service) -> String {
    let exec: Vec<String> = service.argv().iter().map(|argument| systemd_word(argument)).collect();
    let (name, log) = (&service.run.name, specifiers(&service.log.display().to_string()));
    format!(
        "[Unit]
Description=octopus server {name}

[Service]
ExecStart={exec}
Restart=always
KillMode=process
StandardOutput=append:{log}
StandardError=append:{log}

[Install]
WantedBy=default.target
",
        exec = exec.join(" ")
    )
}

/// Returns the command's trimmed stdout.
fn run(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new(program).args(args).output().with_context(|| format!("run {program}"))?;
    if !output.status.success() {
        bail!("{program} {}: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn remove_unit(path: &Path) -> anyhow::Result<()> {
    std::fs::remove_file(path).with_context(|| format!("remove {}", path.display()))
}

/// The user service manager serves the whole account, so a unit read or registered under any other HOME would run
/// in the account's real session, beside and outliving the account's own server.
fn account_home() -> anyhow::Result<PathBuf> {
    let account = nix::unistd::User::from_uid(nix::unistd::getuid())
        .context("look up this account")?
        .ok_or_else(|| anyhow!("this user has no account entry"))?;
    let home = dirs::home_dir().ok_or_else(|| anyhow!("no home directory"))?;
    if home != account.dir {
        bail!(
            "HOME is {}, not this account's home {}, so octopus manages no service here",
            home.display(),
            account.dir.display()
        );
    }
    Ok(home)
}

fn unit_path(id: &str) -> anyhow::Result<PathBuf> {
    Ok(manager::unit_directory()?.join(format!("{id}{}", manager::EXTENSION)))
}

/// How long an upgrade may hold a unit before another takes it over.
const UPGRADE_PATIENCE: std::time::Duration = std::time::Duration::from_secs(60);

/// Rewrites and registers again each octopus unit that runs this binary and that this binary would write
/// differently. A unit it cannot read is another program's. Every unit has its turn, and the error names
/// each one that could not be registered.
pub fn upgrade_units() -> anyhow::Result<()> {
    let directory = manager::unit_directory()?;
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("read {}", directory.display())),
    };
    let program = std::env::current_exe()?.display().to_string();
    let mut failed = Vec::new();
    for unit in entries.flatten().map(|entry| entry.path()) {
        let name = unit.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(id) = name.strip_suffix(manager::EXTENSION).filter(|id| id.starts_with("octopus-")) else { continue };
        let Ok(text) = std::fs::read_to_string(&unit) else { continue };
        if !manager::runs(&text, &program) {
            continue;
        }
        if let Some(upgraded) = upgraded_unit(&text, manager::SPELLING) {
            let registered = std::fs::write(&unit, upgraded)
                .with_context(|| format!("write {}", unit.display()))
                .and_then(|()| manager::register(id, &unit));
            failed.extend(registered.err().map(|error| format!("{id}: {error:#}")));
        }
    }
    if !failed.is_empty() {
        bail!("register again the services this binary would write differently: {}", failed.join("; "));
    }
    Ok(())
}

/// Upgrades the unit of the service `name`, whose server found it passing flags it no longer takes, and registers it
/// again whatever its file now says, since the service manager may still hold the old copy. A service manager
/// restarting the old server spawns many of these at once, and one does the work.
pub fn upgrade_service(name: &str) -> anyhow::Result<()> {
    let id = service_id(name)?;
    let unit = unit_path(&id)?;
    let text = std::fs::read_to_string(&unit).with_context(|| format!("no service {id} at {}", unit.display()))?;
    let lock = unit.with_extension("upgrading");
    if let Err(error) = std::fs::File::create_new(&lock) {
        let age = std::fs::metadata(&lock).and_then(|lock| lock.modified()).map(|at| at.elapsed().unwrap_or_default());
        if error.kind() != std::io::ErrorKind::AlreadyExists || age.is_ok_and(|age| age < UPGRADE_PATIENCE) {
            return Ok(());
        }
    }
    let upgraded = upgraded_unit(&text, manager::SPELLING)
        .map_or(Ok(()), |upgraded| std::fs::write(&unit, upgraded).with_context(|| format!("write {}", unit.display())))
        .and_then(|()| manager::register(&id, &unit));
    let _lock_gone = std::fs::remove_file(&lock);
    upgraded
}

/// What a server started without `--microphone` says when `server upgrade` cannot give its service one: its unit
/// file `unit` is unreadable, or carries no `--microphone` even once upgraded. None when the upgraded file carries
/// one, since the service manager may still hold an older command line than the file.
fn reinstall_reason(unit: anyhow::Result<String>, spelling: &Spelling) -> Option<String> {
    let reason = match unit {
        Err(error) => format!("{error:#}"),
        Ok(text) => {
            let upgraded = upgraded_unit(&text, spelling).unwrap_or(text);
            if flag_value(&upgraded, "--microphone", spelling).is_some() {
                return None;
            }
            "its unit is too old to upgrade".to_string()
        }
    };
    Some(format!("octopus: this service cannot upgrade itself: {reason}; run `octopus server install` for it again"))
}

/// A service an older binary installed starts `server run` without `--microphone`, which no longer parses. Its
/// server hands the upgrade to `server upgrade` in a process group of its own, which outlives the service manager
/// stopping this one.
pub fn hand_off_old_service(argv: &[String]) {
    use std::os::unix::process::CommandExt;
    let old_server = argv.get(1..3) == Some(&["server".to_string(), "run".to_string()])
        && !argv.iter().any(|word| word == "--microphone");
    if !old_server {
        return;
    }
    let name = argv.windows(2).find(|pair| pair[0] == "--name").map(|pair| pair[1].as_str()).unwrap_or_default();
    let unit = service_id(name)
        .and_then(|id| unit_path(&id))
        .and_then(|unit| std::fs::read_to_string(&unit).with_context(|| format!("read {}", unit.display())));
    if let Some(reason) = reinstall_reason(unit, manager::SPELLING) {
        eprintln!("{reason}");
        return;
    }
    let upgrade = std::env::current_exe().and_then(|program| {
        Command::new(program).args(["server", "upgrade", "--name", name]).process_group(0).stdin(Stdio::null()).spawn()
    });
    match upgrade {
        Ok(_) => {
            eprintln!("octopus: service {name} was installed with older flags; registering it again with today's")
        }
        Err(error) => eprintln!("octopus: start `octopus server upgrade --name {name}`: {error}"),
    }
}

/// The server installs whatever becomes of the Octopus Microphone.
pub fn install(service: &Service) -> anyhow::Result<Installed> {
    let unit = unit_path(&service.id)?;
    for directory in [&unit, &service.log].map(|path| path.parent().expect("an absolute file path has a directory")) {
        std::fs::create_dir_all(directory).with_context(|| format!("create {}", directory.display()))?;
    }
    std::fs::write(&unit, manager::unit(service)).with_context(|| format!("write {}", unit.display()))?;
    let microphone = if service.run.microphone { microphone::install() } else { microphone::Outcome::NotAsked };
    if let microphone::Outcome::Failed { reason } = &microphone {
        eprintln!("octopus: installed without the Octopus Microphone: {reason}");
    }
    manager::register(&service.id, &unit)?;
    Ok(Installed { registration: Registration { service: service.id.clone(), unit }, microphone })
}

#[derive(Serialize)]
pub struct Uninstalled {
    #[serde(flatten)]
    pub registration: Registration,
    pub microphone: microphone::Outcome,
}

/// The terminal host and its agents keep running; they end with the machine. The user's input comes back, and the
/// Octopus Microphone goes with the machine's last server.
pub fn uninstall(name: &str) -> anyhow::Result<Uninstalled> {
    let id = service_id(name)?;
    let unit = unit_path(&id)?;
    let terminal_host = microphone::terminal_host(&unit);
    manager::unregister(&id, &unit)?;
    let microphone = microphone::uninstall(terminal_host.as_deref(), &unit);
    if let microphone::Outcome::Failed { reason } = &microphone {
        eprintln!("octopus: uninstalled, leaving the Octopus Microphone: {reason}");
    }
    Ok(Uninstalled { registration: Registration { service: id, unit }, microphone })
}

/// Registers again each service an older binary installed, then lists this machine's server when this client lists
/// none on the local socket. A failed upgrade is reported after the listing, which goes on without it.
pub fn list_this_machine() -> anyhow::Result<()> {
    let upgraded = upgrade_units();
    list_or_install_this_machine()?;
    upgraded
}

/// The server already answering on this machine's local socket, or else a fresh install named after the computer.
fn list_or_install_this_machine() -> anyhow::Result<()> {
    let config = Config::user()?;
    let mut servers = Servers::load(&config)?;
    if servers.servers.iter().any(|server| matches!(server.address, Address::Local { .. })) {
        return Ok(());
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow!("no home directory"))?.join(".octopus");
    let (name, socket) = (octopus_client::servers::machine_name()?, local_socket(&home, "server"));
    servers.add(Server { name: name.clone(), address: Address::Local { socket: socket.clone() } })?;
    if local::connect_blocking(&socket).is_err() {
        let run = ServerRun {
            name,
            socket,
            store: home.join("store.db"),
            worktrees: home.join("worktrees"),
            terminal_host: local_socket(&home, "host"),
            udp: ([0, 0, 0, 0], 7300).into(),
            relay: remote::N0_EU_RELAY.parse().expect("n0's relay is a URL"),
            microphone: true,
        };
        install(&Service::new(run, std::env::current_exe()?, home.join("server.log"))?)?;
    }
    servers.save(&config)
}

fn local_socket(home: &Path, name: &str) -> PathBuf {
    home.join(format!("{name}.sock"))
}

#[cfg(target_os = "macos")]
mod manager {
    use super::*;

    pub const EXTENSION: &str = ".plist";

    pub fn unit_directory() -> anyhow::Result<PathBuf> {
        Ok(account_home()?.join("Library").join("LaunchAgents"))
    }

    pub const SPELLING: &Spelling = &PLIST;

    pub fn runs(unit: &str, program: &str) -> bool {
        unit.contains(&plist_word(program))
    }

    pub fn unit(service: &Service) -> Vec<u8> {
        launchd_plist(service).into_bytes()
    }

    fn domain() -> String {
        format!("gui/{}", nix::unistd::getuid())
    }

    fn unload_if_loaded(id: &str) -> anyhow::Result<()> {
        let target = format!("{}/{id}", domain());
        if run("launchctl", &["print", &target]).is_ok() {
            run("launchctl", &["bootout", &target])?;
        }
        Ok(())
    }

    pub fn register(id: &str, unit: &Path) -> anyhow::Result<()> {
        unload_if_loaded(id)?;
        run("launchctl", &["bootstrap", &domain(), &unit.display().to_string()]).map(drop)
    }

    pub fn unregister(id: &str, unit: &Path) -> anyhow::Result<()> {
        unload_if_loaded(id)?;
        remove_unit(unit)
    }
}

#[cfg(target_os = "linux")]
mod manager {
    use super::*;

    pub const EXTENSION: &str = ".service";

    pub fn unit_directory() -> anyhow::Result<PathBuf> {
        Ok(account_home()?.join(".config").join("systemd").join("user"))
    }

    pub const SPELLING: &Spelling = &SYSTEMD;

    pub fn runs(unit: &str, program: &str) -> bool {
        unit.contains(&format!("ExecStart={}", systemd_word(program)))
    }

    pub fn unit(service: &Service) -> Vec<u8> {
        systemd_unit(service).into_bytes()
    }

    fn systemctl(args: &[&str]) -> anyhow::Result<()> {
        run("systemctl", &[&["--user"], args].concat()).map(drop)
    }

    pub fn register(id: &str, _unit: &Path) -> anyhow::Result<()> {
        let file = format!("{id}.service");
        systemctl(&["daemon-reload"])?;
        systemctl(&["enable", &file])?;
        systemctl(&["reset-failed", &file])?;
        systemctl(&["restart", &file])
    }

    pub fn unregister(id: &str, unit: &Path) -> anyhow::Result<()> {
        systemctl(&["disable", "--now", &format!("{id}.service")])?;
        remove_unit(unit)?;
        systemctl(&["daemon-reload"])
    }
}

/// The Octopus Microphone, the HAL plug-in a macOS server's terminals record from while a client on another
/// machine holds them.
pub mod microphone {
    use super::*;

    #[derive(Debug, PartialEq, Serialize)]
    #[serde(tag = "state", rename_all = "snake_case")]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code, reason = "only macOS installs a plug-in"))]
    pub enum Outcome {
        /// This install gives terminals no virtual microphone, or the platform needs no plug-in.
        NotAsked,
        /// The same plug-in was already installed, so nothing was asked.
        Unchanged,
        Installed,
        Removed,
        /// Another server on this machine still uses it.
        Kept,
        /// The prompt was declined or the copy failed; the server installed or uninstalled all the same.
        Failed {
            reason: String,
        },
    }

    #[cfg(any(target_os = "macos", test))]
    const BUNDLE: &str = "OctopusMicrophone.driver";

    #[cfg(target_os = "macos")]
    const PLUG_INS: &str = "/Library/Audio/Plug-Ins/HAL";

    #[cfg(target_os = "macos")]
    macro_rules! built {
        ($path:literal) => {
            ($path, include_bytes!(concat!(env!("OUT_DIR"), "/OctopusMicrophone.driver/", $path)) as &[u8])
        };
    }

    #[cfg(target_os = "macos")]
    const FILES: [(&str, &[u8]); 3] = [
        built!("Contents/Info.plist"),
        built!("Contents/MacOS/OctopusMicrophone"),
        built!("Contents/_CodeSignature/CodeResources"),
    ];

    #[cfg(not(target_os = "macos"))]
    pub fn install() -> Outcome {
        Outcome::NotAsked
    }

    #[cfg(not(target_os = "macos"))]
    pub fn terminal_host(_unit: &Path) -> Option<PathBuf> {
        None
    }

    #[cfg(not(target_os = "macos"))]
    pub fn uninstall(_terminal_host: Option<&Path>, _unit: &Path) -> Outcome {
        Outcome::NotAsked
    }

    /// The --terminal-host the launchd agent at `unit` runs its server with.
    #[cfg(target_os = "macos")]
    pub fn terminal_host(unit: &Path) -> Option<PathBuf> {
        let unit = unit.display().to_string();
        let argv = run("plutil", &["-extract", "ProgramArguments", "json", "-o", "-", &unit]).ok()?;
        let argv: Vec<String> = serde_json::from_str(&argv).ok()?;
        argv.windows(2).find(|pair| pair[0] == "--terminal-host").map(|pair| PathBuf::from(&pair[1]))
    }

    /// Gives back the input the server's microphone stood in for, then removes the plug-in once no other octopus
    /// launchd agent remains beside `unit`.
    #[cfg(target_os = "macos")]
    pub fn uninstall(terminal_host: Option<&Path>, unit: &Path) -> Outcome {
        if let Some(terminal_host) = terminal_host
            && let Err(error) = octopus_server::give_back_input(terminal_host)
        {
            eprintln!("octopus: give back the default input: {error:#}");
        }
        let agents = unit.parent().expect("a launchd agent sits in a directory");
        let others = std::fs::read_dir(agents).into_iter().flatten().flatten().any(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.starts_with("octopus-") && name.ends_with(".plist")
        });
        let bundle = Path::new(PLUG_INS).join(BUNDLE);
        if others {
            return Outcome::Kept;
        }
        if !bundle.exists() {
            return Outcome::NotAsked;
        }
        match run("osascript", &["-e", &applescript(&removal(&bundle))]) {
            Ok(_) => Outcome::Removed,
            Err(error) => Outcome::Failed { reason: format!("{error:#}") },
        }
    }

    /// Run as root: coreaudiod drops a plug-in only as it restarts.
    #[cfg(any(target_os = "macos", test))]
    fn removal(bundle: &Path) -> String {
        format!("rm -rf {}; killall coreaudiod || true", quote(bundle))
    }

    /// Asks for an administrator only when the installed plug-in differs from this build's.
    #[cfg(target_os = "macos")]
    pub fn install() -> Outcome {
        let plug_ins = Path::new(PLUG_INS);
        if up_to_date(&plug_ins.join(BUNDLE), &FILES) {
            return Outcome::Unchanged;
        }
        match copy_as_administrator(plug_ins) {
            Ok(()) => Outcome::Installed,
            Err(error) => Outcome::Failed { reason: format!("{error:#}") },
        }
    }

    #[cfg(target_os = "macos")]
    fn copy_as_administrator(plug_ins: &Path) -> anyhow::Result<()> {
        let stage = tempfile::tempdir().context("create a staging directory")?;
        let staged = stage.path().join(BUNDLE);
        for (path, bytes) in FILES {
            let file = staged.join(path);
            std::fs::create_dir_all(file.parent().expect("a bundle file sits in a directory"))?;
            std::fs::write(&file, bytes).with_context(|| format!("stage {}", file.display()))?;
        }
        run("osascript", &["-e", &applescript(&script(&staged, plug_ins))]).map(drop)
    }

    #[cfg(any(target_os = "macos", test))]
    fn up_to_date(bundle: &Path, files: &[(&str, &[u8])]) -> bool {
        files.iter().all(|(path, bytes)| std::fs::read(bundle.join(path)).is_ok_and(|found| found == *bytes))
    }

    #[cfg(any(target_os = "macos", test))]
    fn quote(path: &Path) -> String {
        format!("'{}'", path.display().to_string().replace('\'', r"'\''"))
    }

    /// Run as root: swaps the staged bundle in whole, then restarts coreaudiod, which loads plug-ins only as it
    /// starts. launchd starts it again at once.
    #[cfg(any(target_os = "macos", test))]
    fn script(staged: &Path, plug_ins: &Path) -> String {
        let (installed, fresh) = (plug_ins.join(BUNDLE), plug_ins.join(format!("{BUNDLE}.new")));
        let (staged, plug_ins, installed, fresh) = (quote(staged), quote(plug_ins), quote(&installed), quote(&fresh));
        format!(
            "set -e; mkdir -p {plug_ins}; rm -rf {fresh}; cp -R {staged} {fresh}; chown -R root:wheel {fresh}; \
             chmod -R go-w {fresh}; rm -rf {installed}; mv {fresh} {installed}; killall coreaudiod || true"
        )
    }

    /// One prompt: AppleScript runs the whole script with the administrator's rights.
    #[cfg(any(target_os = "macos", test))]
    fn applescript(script: &str) -> String {
        let escaped = script.replace('\\', "\\\\").replace('"', "\\\"");
        format!("do shell script \"{escaped}\" with administrator privileges")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn only_a_plug_in_differing_from_this_build_is_installed_again() {
            let bundle = tempfile::tempdir().expect("tempdir");
            let files: [(&str, &[u8]); 2] = [("Contents/Info.plist", b"plist"), ("Contents/MacOS/Mic", b"binary")];
            assert!(!up_to_date(bundle.path(), &files), "nothing installed");
            for (path, bytes) in files {
                let file = bundle.path().join(path);
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(file, bytes).unwrap();
            }
            assert!(up_to_date(bundle.path(), &files));
            std::fs::write(bundle.path().join("Contents/MacOS/Mic"), b"older").unwrap();
            assert!(!up_to_date(bundle.path(), &files));
        }

        #[test]
        fn the_install_script_survives_quotes_in_its_paths() {
            let staged = Path::new("/tmp/it's \"staged\"/OctopusMicrophone.driver");
            let script = script(staged, Path::new("/Library/Audio/Plug-Ins/HAL"));
            let quoted = quote(staged);
            let echoed = Command::new("sh").args(["-c", &format!("printf %s {quoted}")]).output().unwrap().stdout;
            assert_eq!(String::from_utf8(echoed).unwrap(), staged.display().to_string());
            assert!(
                script.contains(&format!("cp -R {quoted} '/Library/Audio/Plug-Ins/HAL/OctopusMicrophone.driver.new'"))
            );
            assert!(script.ends_with("killall coreaudiod || true"));
            let applescript = applescript(&script);
            let inner = applescript
                .strip_prefix("do shell script \"")
                .and_then(|rest| rest.strip_suffix("\" with administrator privileges"))
                .unwrap();
            assert_eq!(inner.replace("\\\"", "\"").replace("\\\\", "\\"), script);
            assert!(!inner.replace("\\\"", "").contains('"'), "every quote in the script is escaped");
        }

        #[test]
        fn removing_the_plug_in_restarts_the_audio_service() {
            let bundle = Path::new("/Library/Audio/Plug-Ins/HAL/OctopusMicrophone.driver");
            assert_eq!(removal(bundle), format!("rm -rf {}; killall coreaudiod || true", quote(bundle)));
        }

        #[cfg(target_os = "macos")]
        #[test]
        fn uninstall_finds_the_terminal_host_its_agent_runs_with() {
            let dir = tempfile::tempdir().unwrap();
            let unit = dir.path().join("octopus-mac-mini.plist");
            std::fs::write(&unit, launchd_plist(&crate::install::tests::service("/var/octopus/server.log"))).unwrap();
            assert_eq!(terminal_host(&unit), Some(PathBuf::from("/run/octopus/host.sock")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn service(log: &str) -> Service {
        let run = ServerRun {
            name: "mac-mini".into(),
            socket: "/run/octopus/mac mini.sock".into(),
            store: "/var/octopus/store.db".into(),
            worktrees: "/var/octopus/worktrees".into(),
            terminal_host: "/run/octopus/host.sock".into(),
            udp: ([0, 0, 0, 0], 7300).into(),
            relay: remote::N0_EU_RELAY.parse().expect("n0's relay is a URL"),
            microphone: true,
        };
        Service { id: "octopus-mac-mini".into(), run, program: "/opt/octopus/bin/octopus".into(), log: log.into() }
    }

    const ARGV: [&str; 19] = [
        "/opt/octopus/bin/octopus",
        "server",
        "run",
        "--name",
        "mac-mini",
        "--socket",
        "/run/octopus/mac mini.sock",
        "--store",
        "/var/octopus/store.db",
        "--worktrees",
        "/var/octopus/worktrees",
        "--terminal-host",
        "/run/octopus/host.sock",
        "--udp",
        "0.0.0.0:7300",
        "--relay",
        "https://euc1-1.relay.n0.iroh.link./",
        "--microphone",
        "true",
    ];

    #[test]
    fn a_launchd_agent_runs_the_server_at_login_and_spares_its_terminal_host() {
        let plist = launchd_plist(&service("/var/octopus/server.log"));
        let array: String = ARGV.iter().map(|argument| format!("\t\t<string>{argument}</string>\n")).collect();
        assert!(plist.contains(&format!("<key>ProgramArguments</key>\n\t<array>\n{array}\t</array>")), "{plist}");
        for key in ["RunAtLoad", "KeepAlive", "AbandonProcessGroup"] {
            assert!(plist.contains(&format!("<key>{key}</key>\n\t<true/>")), "{key} in {plist}");
        }
        assert!(plist.contains("<key>Label</key>\n\t<string>octopus-mac-mini</string>"));
        assert!(plist.contains("<key>StandardErrorPath</key>\n\t<string>/var/octopus/server.log</string>"));
    }

    #[test]
    fn a_systemd_unit_quotes_each_word_and_stops_the_server_alone() {
        let exec: Vec<String> = ARGV.iter().map(|word| format!("\"{word}\"")).collect();
        let expected = format!(
            "[Unit]
Description=octopus server mac-mini

[Service]
ExecStart={}
Restart=always
KillMode=process
StandardOutput=append:/var/octopus/100%%.log
StandardError=append:/var/octopus/100%%.log

[Install]
WantedBy=default.target
",
            exec.join(" ")
        );
        assert_eq!(systemd_unit(&service("/var/octopus/100%.log")), expected);
    }

    /// A unit format and the unit installed now in it.
    struct Format {
        current: String,
        spelling: &'static Spelling,
    }

    impl Format {
        fn all() -> [Self; 2] {
            let service = service("/var/octopus/server.log");
            [
                Self { current: launchd_plist(&service), spelling: &PLIST },
                Self { current: systemd_unit(&service), spelling: &SYSTEMD },
            ]
        }

        fn spell(&self, words: &[&str]) -> String {
            words.iter().map(|text| (self.spelling.word)(text)).collect::<Vec<_>>().join(self.spelling.separator)
        }

        /// The current unit with its flags from `--udp` on replaced by `flags`.
        fn with(&self, flags: &[&str]) -> String {
            let now = self.spell(&["--udp", "0.0.0.0:7300", "--relay", remote::N0_EU_RELAY, "--microphone", "true"]);
            self.current.replace(&now, &self.spell(flags))
        }

        fn upgraded(&self, unit: &str) -> Option<String> {
            upgraded_unit(unit, self.spelling)
        }
    }

    #[test]
    fn a_flag_gives_back_the_argument_its_word_spells() {
        for argument in ["[::1%1]:7300", "[fe80::1%2]:7300", "https://relay.example/a?b=1&c=100%25$x"] {
            for format in Format::all() {
                let unit = format.spell(&["--udp", argument, "--relay", remote::N0_EU_RELAY]);
                let (_, read) = flag_value(&unit, "--udp", format.spelling).expect("the flag");
                assert_eq!(read, argument);
            }
        }
    }

    #[test]
    fn a_server_without_a_microphone_hands_off_unless_its_unit_cannot_gain_one() {
        let relay = remote::N0_EU_RELAY;
        for format in Format::all() {
            let before_microphones = format.with(&["--udp", "0.0.0.0:7300", "--relay", relay]);
            for upgradable in [before_microphones, format.current.clone()] {
                assert_eq!(reinstall_reason(Ok(upgradable), format.spelling), None);
            }
            let quic = format.with(&["--quic", "0.0.0.0:7300"]);
            let reason = reinstall_reason(Ok(quic), format.spelling).expect("a --quic unit is too old");
            assert!(reason.contains("too old") && reason.contains("`octopus server install`"), "{reason}");
        }
    }

    #[test]
    fn a_server_whose_unit_is_unreadable_names_the_reinstall_and_why() {
        let missing = std::env::temp_dir().join("octopus-no-such-unit.plist");
        let unit = std::fs::read_to_string(&missing).with_context(|| format!("read {}", missing.display()));
        let reason = reinstall_reason(unit, &PLIST).expect("an unreadable unit");
        assert!(reason.contains(&format!("read {}", missing.display())), "{reason}");
        assert!(reason.contains("`octopus server install`"), "{reason}");
        let reason = reinstall_reason(service_id("").map(|_| String::new()), &PLIST).expect("no --name");
        assert!(reason.contains("--name must not be empty"), "{reason}");
    }

    #[test]
    fn a_service_never_installs_on_loopback() {
        for udp in ["127.0.0.1:7300", "[::1]:7300"] {
            let mut run = service("/log").run;
            run.udp = udp.parse().expect("an address");
            let error = Service::new(run, "/opt/octopus/bin/octopus".into(), "/log".into()).err().expect("refused");
            assert!(error.to_string().contains("--udp 0.0.0.0:7300"), "{error}");
        }
    }

    #[test]
    fn a_unit_with_older_flags_is_rewritten_to_the_unit_installed_now() {
        let relay = remote::N0_EU_RELAY;
        for format in Format::all() {
            for old in [
                format.with(&["--udp", "0.0.0.0:7300", "--relay", relay]),
                format.with(&["--udp", "127.0.0.1:7300", "--relay", relay, "--microphone", "true"]),
                format.with(&["--udp", "127.1.2.3:7300", "--relay", relay, "--microphone", "true"]),
                format.with(&["--udp", "[::1]:7300", "--relay", relay, "--microphone", "true"]),
                format.with(&["--udp", "[::1%1]:7300", "--relay", relay, "--microphone", "true"]),
                format.with(&["--udp", "127.0.0.1:7300", "--relay", relay]),
            ] {
                assert_ne!(old, format.current);
                assert_eq!(format.upgraded(&old), Some(format.current.clone()));
            }
            assert_eq!(format.upgraded(&format.current), None);
        }
    }

    #[test]
    fn a_unit_keeps_every_listen_address_but_loopback() {
        let relay = remote::N0_EU_RELAY;
        for format in Format::all() {
            for address in ["0.0.0.0:7300", "192.168.1.20:7300", "100.101.102.103:7300", "[fe80::1%2]:7300"] {
                let current = format.with(&["--udp", address, "--relay", relay, "--microphone", "true"]);
                assert_eq!(format.upgraded(&current), None, "{address}");
                let old = format.with(&["--udp", address, "--relay", relay]);
                assert_eq!(format.upgraded(&old), Some(current), "{address}");
            }
        }
    }

    #[test]
    fn a_service_takes_one_safe_name_and_distinct_sockets() {
        assert_eq!(service_id("mac-mini.2").expect("id"), "octopus-mac-mini.2");
        assert_eq!(service_id("Ada’s Mac").expect("id"), "octopus-Ada-s-Mac");
        assert!(service_id("").is_err());
        let ServerRun { name, socket, store, worktrees, udp, relay, microphone, .. } = service("/log").run;
        let host = PathBuf::from(socket.display().to_string().to_uppercase());
        let shared = ServerRun { name, terminal_host: host, socket, store, worktrees, udp, relay, microphone };
        assert!(shared.check().is_err(), "one socket, spelled in another case");
    }
}
