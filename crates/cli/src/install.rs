//! Registers `octopus server run` with the platform's user service manager, started at login.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, anyhow, bail};
use octopus_client::servers::{Address, Server, Servers};
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

    fn flags(&self) -> [(&'static str, String); 7] {
        [
            ("--name", self.name.clone()),
            ("--socket", self.socket.display().to_string()),
            ("--store", self.store.display().to_string()),
            ("--worktrees", self.worktrees.display().to_string()),
            ("--terminal-host", self.terminal_host.display().to_string()),
            ("--udp", self.udp.to_string()),
            ("--relay", self.relay.to_string()),
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
    /// Service managers start in their own working directory, so every path is made absolute.
    pub fn new(run: ServerRun, program: PathBuf, log: PathBuf) -> anyhow::Result<Self> {
        let id = service_id(&run.name)?;
        let run = ServerRun {
            name: run.name,
            socket: absolute(run.socket)?,
            store: absolute(run.store)?,
            worktrees: absolute(run.worktrees)?,
            terminal_host: absolute(run.terminal_host)?,
            udp: run.udp,
            relay: run.relay,
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

/// Text no argument holds, which shows where a word puts its argument.
const MARK: &str = "\u{0}";

/// The unit `text`, spelled word by word with `word` and `separator`, re-spelled from `--quic <address>`, which this
/// binary no longer takes, to `--udp <address> --relay <n0's relay>`, keeping the address as the unit spells it;
/// none for a unit already current.
fn upgraded_unit(text: &str, word: impl Fn(&str) -> String, separator: &str) -> Option<String> {
    let flag = word("--quic") + separator;
    let marked = word(MARK);
    let (prefix, suffix) = marked.split_once(MARK).expect("a word wraps its text");
    let start = text.find(&flag)?;
    let address_start = start + flag.len() + prefix.len();
    text[start + flag.len()..].starts_with(prefix).then_some(())?;
    let address_end = address_start + text[address_start..].find(suffix)?;
    let address = format!("{prefix}{}{suffix}", &text[address_start..address_end]);
    let current = [word("--udp"), address, word("--relay"), word(remote::N0_EU_RELAY)].join(separator);
    Some(format!("{}{current}{}", &text[..start], &text[address_end + suffix.len()..]))
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

/// The name the user gave this computer, which names its server.
fn machine_name() -> anyhow::Result<String> {
    #[cfg(target_os = "macos")]
    return run("scutil", &["--get", "ComputerName"]);
    #[cfg(target_os = "linux")]
    Ok(octopus_client::servers::hostname())
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

fn unit_path(id: &str) -> anyhow::Result<PathBuf> {
    Ok(manager::unit_directory()?.join(format!("{id}{}", manager::EXTENSION)))
}

/// How long an upgrade may hold a unit before another takes it over.
const UPGRADE_PATIENCE: std::time::Duration = std::time::Duration::from_secs(60);

/// Registers again each octopus unit that runs this binary with `--quic`, which it no longer takes, so its server
/// comes back up on `--udp` and n0's relay. A unit it cannot read is another program's. Every unit has its turn, and
/// the error names each one that could not be registered.
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
        if let Some(upgraded) = manager::upgraded(&text) {
            let registered = std::fs::write(&unit, upgraded)
                .with_context(|| format!("write {}", unit.display()))
                .and_then(|()| manager::register(id, &unit));
            failed.extend(registered.err().map(|error| format!("{id}: {error:#}")));
        }
    }
    if !failed.is_empty() {
        bail!("register the services installed with --quic again: {}", failed.join("; "));
    }
    Ok(())
}

/// Upgrades the unit of the service `name`, whose server found it still passing `--quic`, and registers it again
/// whatever its file now says, since the service manager may still hold the old copy. A service manager restarting
/// the old server spawns many of these at once, and one does the work.
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
    let upgraded = manager::upgraded(&text)
        .map_or(Ok(()), |upgraded| std::fs::write(&unit, upgraded).with_context(|| format!("write {}", unit.display())))
        .and_then(|()| manager::register(&id, &unit));
    let _lock_gone = std::fs::remove_file(&lock);
    upgraded
}

/// A service an older binary installed starts `server run --quic`, which no longer parses. Its server hands the
/// upgrade to `server upgrade` in a process group of its own, which outlives the service manager stopping this one.
pub fn hand_off_old_service(argv: &[String]) {
    use std::os::unix::process::CommandExt;
    let old_server =
        argv.get(1..3) == Some(&["server".to_string(), "run".to_string()]) && argv.iter().any(|word| word == "--quic");
    let Some(name) = argv.windows(2).find(|pair| pair[0] == "--name").map(|pair| &pair[1]) else { return };
    if !old_server {
        return;
    }
    let upgrade = std::env::current_exe().and_then(|program| {
        Command::new(program).args(["server", "upgrade", "--name", name]).process_group(0).stdin(Stdio::null()).spawn()
    });
    match upgrade {
        Ok(_) => {
            eprintln!("octopus: service {name} was installed with --quic; registering it again with --udp and --relay")
        }
        Err(error) => eprintln!("octopus: start `octopus server upgrade --name {name}`: {error}"),
    }
}

pub fn install(service: &Service) -> anyhow::Result<Registration> {
    let unit = unit_path(&service.id)?;
    for directory in [&unit, &service.log].map(|path| path.parent().expect("an absolute file path has a directory")) {
        std::fs::create_dir_all(directory).with_context(|| format!("create {}", directory.display()))?;
    }
    std::fs::write(&unit, manager::unit(service)).with_context(|| format!("write {}", unit.display()))?;
    manager::register(&service.id, &unit)?;
    Ok(Registration { service: service.id.clone(), unit })
}

/// The terminal host and its agents keep running; they end with the machine.
pub fn uninstall(name: &str) -> anyhow::Result<Registration> {
    let id = service_id(name)?;
    let unit = unit_path(&id)?;
    manager::unregister(&id, &unit)?;
    Ok(Registration { service: id, unit })
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
    let mut servers = Servers::load()?;
    if servers.servers.iter().any(|server| matches!(server.address, Address::Local { .. })) {
        return Ok(());
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow!("no home directory"))?.join(".octopus");
    let (name, socket) = (machine_name()?, local_socket(&home, "server"));
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
        };
        install(&Service::new(run, std::env::current_exe()?, home.join("server.log"))?)?;
    }
    servers.save()
}

fn local_socket(home: &Path, name: &str) -> PathBuf {
    home.join(format!("{name}.sock"))
}

#[cfg(target_os = "macos")]
mod manager {
    use super::*;

    pub const EXTENSION: &str = ".plist";

    pub fn unit_directory() -> anyhow::Result<PathBuf> {
        Ok(dirs::home_dir().ok_or_else(|| anyhow!("no home directory"))?.join("Library").join("LaunchAgents"))
    }

    pub fn upgraded(unit: &str) -> Option<String> {
        upgraded_unit(unit, plist_word, "")
    }

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
        Ok(dirs::config_dir().ok_or_else(|| anyhow!("no config directory"))?.join("systemd").join("user"))
    }

    pub fn upgraded(unit: &str) -> Option<String> {
        upgraded_unit(unit, systemd_word, " ")
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn service(log: &str) -> Service {
        let run = ServerRun {
            name: "mac-mini".into(),
            socket: "/run/octopus/mac mini.sock".into(),
            store: "/var/octopus/store.db".into(),
            worktrees: "/var/octopus/worktrees".into(),
            terminal_host: "/run/octopus/host.sock".into(),
            udp: ([0, 0, 0, 0], 7300).into(),
            relay: remote::N0_EU_RELAY.parse().expect("n0's relay is a URL"),
        };
        Service { id: "octopus-mac-mini".into(), run, program: "/opt/octopus/bin/octopus".into(), log: log.into() }
    }

    const ARGV: [&str; 17] = [
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

    #[test]
    fn a_unit_installed_with_quic_is_rewritten_to_the_unit_installed_now() {
        let service = service("/var/octopus/server.log");
        type Word = fn(&str) -> String;
        let formats: [(String, Word, &str); 2] =
            [(launchd_plist(&service), plist_word, ""), (systemd_unit(&service), systemd_word, " ")];
        for (current, word, separator) in formats {
            let flags = |words: &[&str]| words.iter().map(|text| word(text)).collect::<Vec<_>>().join(separator);
            let now = flags(&["--udp", "0.0.0.0:7300", "--relay", remote::N0_EU_RELAY]);
            let old = current.replace(&now, &flags(&["--quic", "0.0.0.0:7300"]));
            assert_ne!(old, current);
            assert_eq!(upgraded_unit(&old, word, separator), Some(current.clone()));
            assert_eq!(upgraded_unit(&current, word, separator), None);
        }
    }

    #[test]
    fn a_systemd_unit_installed_with_quic_keeps_its_escaped_address() {
        let old = r#"[Service]
ExecStart="/opt/octopus/bin/octopus" "server" "run" "--name" "mac-mini" "--quic" "[fe80::1%%2]:7300"
Restart=always
"#;
        let upgraded = upgraded_unit(old, systemd_word, " ").expect("an old unit");
        let relay = remote::N0_EU_RELAY;
        assert!(upgraded.contains(&format!(
            r#""--udp" "[fe80::1%%2]:7300" "--relay" "{relay}"
Restart"#
        )));
        assert_eq!(upgraded_unit(&upgraded, systemd_word, " "), None);
    }

    #[test]
    fn a_service_takes_one_safe_name_and_distinct_sockets() {
        assert_eq!(service_id("mac-mini.2").expect("id"), "octopus-mac-mini.2");
        assert_eq!(service_id("Ada’s Mac").expect("id"), "octopus-Ada-s-Mac");
        assert!(service_id("").is_err());
        let ServerRun { name, socket, store, worktrees, udp, relay, .. } = service("/log").run;
        let host = PathBuf::from(socket.display().to_string().to_uppercase());
        let shared = ServerRun { name, terminal_host: host, socket, store, worktrees, udp, relay };
        assert!(shared.check().is_err(), "one socket, spelled in another case");
    }
}
