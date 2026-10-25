//! The verbs that send no single op: they run a process, edit this client's server list, or stream.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, anyhow};
use clap::{ArgAction, Subcommand};
use octopus_client::Identity;
use octopus_client::servers::{self, Address, Server as KnownServer, Servers};
use octopus_proto::local::Listener;
use octopus_proto::ops::{TaskGet, TermResize, TermTakeLock};
use octopus_proto::pairing::Code;
use octopus_proto::quic;
use octopus_proto::types::{Stream, TaskId};
use octopus_server::TerminalHost;
use tokio::io::AsyncReadExt;

use crate::commands::SERVER_HELP;
use crate::install::{self, ServerRun, Service};
use crate::{attach, call, call_error, connect, print};

#[derive(Subcommand)]
pub enum ServerVerb {
    /// Run this machine's server in the foreground.
    Run(ServerRun),
    /// Run the server at every login as a user service (launchd, systemd --user, or Task Scheduler).
    Install {
        #[command(flatten)]
        run: ServerRun,
        /// The file the service's output is appended to.
        #[arg(long)]
        log: PathBuf,
    },
    /// Remove the login service that `install` registered for this --name.
    Uninstall {
        /// The --name the service was installed with.
        #[arg(long)]
        name: String,
    },
    /// Run the process that keeps a server's terminals alive across server restarts; `server run` starts it.
    #[command(hide = true)]
    TerminalHost {
        /// The socket to serve, the server's --terminal-host.
        #[arg(long)]
        address: PathBuf,
    },
    /// Remember a server on this machine's local socket under a name.
    Add {
        /// The name to call the server by.
        #[arg(long)]
        name: String,
        /// The server's --socket.
        #[arg(long)]
        socket: PathBuf,
    },
    /// Call a listed server by another name in this client's list; the server is not told.
    Rename {
        #[arg(long, help = SERVER_HELP)]
        server: String,
        /// The name to call the server by.
        #[arg(long)]
        name: String,
    },
    /// Drop a server and its pin from this client's list; the server keeps this client paired.
    Forget {
        #[arg(long, help = SERVER_HELP)]
        server: String,
    },
}

impl ServerVerb {
    pub async fn run(self) -> anyhow::Result<()> {
        match self {
            Self::Run(run) => {
                run.check()?;
                let ServerRun { name, socket, store, worktrees, terminal_host, quic } = run;
                let local = Listener::bind(&socket).with_context(|| format!("bind {}", socket.display()))?;
                let host = TerminalHost { program: std::env::current_exe()?, address: terminal_host };
                let server = octopus_server::Server::open(&name, &store, &worktrees, &host)
                    .map_err(|error| anyhow!("{error}"))?;
                let remote = quic::listen(quic, server.device()).with_context(|| format!("bind QUIC on {quic}"))?;
                let key = server.device().public();
                eprintln!("octopus server {name} listening on {} and QUIC {quic}, key {key}", socket.display());
                octopus_server::serve(server, local, remote).await;
                Ok(())
            }
            Self::Install { run, log } => print(&install::install(&Service::new(run, std::env::current_exe()?, log)?)?),
            Self::Uninstall { name } => print(&install::uninstall(&name)?),
            Self::TerminalHost { address } => {
                octopus_server::run_terminal_host(&address).await.with_context(|| format!("host {}", address.display()))
            }
            Self::Add { name, socket } => {
                print(&Servers::remember(KnownServer { name, address: Address::Local { socket } })?)
            }
            Self::Rename { server, name } => {
                let mut servers = Servers::load()?;
                servers.rename(&server, &name)?;
                servers.save()?;
                print(&servers)
            }
            Self::Forget { server } => {
                let mut servers = Servers::load()?;
                servers.forget(&server)?;
                servers.save()?;
                print(&servers)
            }
        }
    }
}

#[derive(Subcommand)]
pub enum TermVerb {
    /// Stream a terminal panel into this terminal; Ctrl-] detaches.
    Attach {
        #[arg(long, help = SERVER_HELP)]
        server: String,
        /// The task's id.
        #[arg(long)]
        task: String,
        /// The panel's name or id, e.g. agent.
        #[arg(long)]
        panel: String,
        /// Whether to take the panel's write lock and size it to this terminal.
        #[arg(long, action = ArgAction::Set)]
        take_lock: bool,
    },
}

impl TermVerb {
    pub async fn run(self) -> anyhow::Result<()> {
        let Self::Attach { server, task, panel, take_lock } = self;
        let connection = connect(&server).await?;
        let task = TaskId(task);
        let view = call(&connection, &TaskGet { task: task.clone() }).await?;
        let pty = view
            .panels
            .iter()
            .find(|candidate| candidate.answers_to(&panel))
            .ok_or_else(|| anyhow!("task {task} has no panel {panel}"))?
            .id
            .clone();
        if take_lock {
            call(&connection, &TermTakeLock { task: task.clone(), panel: panel.clone() }).await?;
            let (cols, rows) = crossterm::terminal::size()?;
            call(&connection, &TermResize { task, panel, cols, rows }).await?;
        }
        attach::run(&connection, &Stream::Pty(pty)).await
    }
}

#[derive(Subcommand)]
pub enum FileVerb {
    /// Write a file of a task's checkout to stdout, byte for byte.
    Read {
        #[arg(long, help = SERVER_HELP)]
        server: String,
        /// The task's id.
        #[arg(long)]
        task: String,
        /// The file, relative to the checkout.
        #[arg(long)]
        path: String,
    },
}

impl FileVerb {
    pub async fn run(self) -> anyhow::Result<()> {
        let Self::Read { server, task, path } = self;
        let connection = connect(&server).await?;
        let stream = Stream::File { task: TaskId(task), path };
        let mut opened = connection.open_data_stream(&stream).await.map_err(call_error)?;
        let mut bytes = Vec::new();
        opened.read_to_end(&mut bytes).await.with_context(|| format!("read {}", stream.name()))?;
        std::io::stdout().write_all(&bytes).context("write stdout")
    }
}

#[derive(Subcommand)]
pub enum PairingVerb {
    /// Pair with a server on another machine using the code its `pairing offer` showed, and list it under its name.
    Accept {
        /// The server's --quic address as this machine reaches it, host:port.
        #[arg(long)]
        address: String,
        /// The one-time code.
        #[arg(long)]
        code: Code,
    },
}

impl PairingVerb {
    pub async fn run(self) -> anyhow::Result<()> {
        let Self::Accept { address, code } = self;
        let server = servers::pair(&address, &code, &Identity::load()?).await?;
        Servers::remember(server.clone())?;
        print(&server)
    }
}
