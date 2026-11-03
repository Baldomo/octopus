//! The first run on a machine: it lists the server already answering here, or installs one named after the computer.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use octopus_client::servers::{Address, Config, Server, Servers};
use octopus_proto::{local, remote};

use crate::install::{self, ServerRun, Service};

/// The server already answering on this machine's local socket, or else a fresh install named after the computer.
pub fn list_or_install_this_machine() -> anyhow::Result<()> {
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
        install::install(&Service::new(run, std::env::current_exe()?, home.join("server.log"))?)?;
    }
    servers.save(&config)
}

fn local_socket(home: &Path, name: &str) -> PathBuf {
    home.join(format!("{name}.sock"))
}
