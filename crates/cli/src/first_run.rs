//! The first run on a machine: it lists the server already answering here, or lists one named after the computer
//! and hands the app its install.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use octopus_client::servers::{Address, Config, Server, Servers};
use octopus_desktop::Install;
use octopus_proto::{local, remote};

use crate::install::{self, ServerRun, Service};

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
    if local::connect_blocking(&socket).is_ok() {
        return Ok(None);
    }
    let log = home.join("server.log");
    let run = ServerRun {
        name: name.clone(),
        socket,
        store: home.join("store.db"),
        worktrees: home.join("worktrees"),
        terminal_host: local_socket(&home, "host"),
        udp: ([0, 0, 0, 0], 7300).into(),
        relay: remote::N0_EU_RELAY.parse().expect("n0's relay is a URL"),
        microphone: true,
    };
    let service = Service::new(run, std::env::current_exe()?, log.clone())?;
    Ok(Some(Install { server: name, log, run: Box::new(move || install::install(&service).map(drop)) }))
}

fn local_socket(home: &Path, name: &str) -> PathBuf {
    home.join(format!("{name}.sock"))
}
