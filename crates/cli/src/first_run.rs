//! The first run on a machine: it lists the server already answering here, or lists one named after the computer
//! and hands the app its install.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail};
use octopus_client::servers::{Address, Config, Server, Servers};
use octopus_desktop::Install;
use octopus_proto::{local, remote};

use crate::ERROR_PREFIX;
use crate::install::{self, ServerRun, Service};

/// How long a server on its socket has to answer one ask.
const PATIENCE: Duration = Duration::from_secs(1);
/// The rest between asks while the installed server starts.
const ANSWER_POLL: Duration = Duration::from_millis(100);

/// Lists this machine's server, and returns its install when no server answers on its socket yet.
pub fn list_this_machine() -> anyhow::Result<Option<Install>> {
    let config = Config::user()?;
    let mut servers = Servers::load(&config)?;
    if servers.servers.iter().any(|server| matches!(server.address, Address::Local { .. })) {
        return Ok(None);
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow!("no home directory"))?.join(".octopus");
    let (name, socket) = (octopus_client::servers::machine_name()?, local_socket(&home, "server"));
    servers.add(Server { name: name.clone(), address: Address::Local { socket: socket.clone() } })?;
    servers.save(&config)?;
    if local::answers(&socket, PATIENCE) {
        return Ok(None);
    }
    let log = home.join("server.log");
    let run = ServerRun {
        name: name.clone(),
        socket: socket.clone(),
        store: home.join("store.db"),
        worktrees: home.join("worktrees"),
        terminal_host: local_socket(&home, "host"),
        udp: ([0, 0, 0, 0], 0).into(),
        relay: remote::N0_EU_RELAY.parse().expect("n0's relay is a URL"),
        microphone: true,
    };
    let service = Service::new(run, std::env::current_exe()?, log.clone())?;
    let answering = log.clone();
    Ok(Some(Install { server: name, log, run: Box::new(move || install_answered(&service, &socket, &answering)) }))
}

/// Installs the service and waits until its server answers on `socket`. A server the service manager sees exit first
/// fails the install with the last error line it wrote to `log`.
fn install_answered(service: &Service, socket: &Path, log: &Path) -> anyhow::Result<()> {
    let logged = std::fs::metadata(log).map_or(0, |written| written.len());
    install::install(service)?;
    while !local::answers(socket, PATIENCE) {
        if install::exited(service)? {
            bail!(last_error(log, logged).unwrap_or_else(|| "the server exited before it answered".into()));
        }
        std::thread::sleep(ANSWER_POLL);
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

fn local_socket(home: &Path, name: &str) -> PathBuf {
    home.join(format!("{name}.sock"))
}
