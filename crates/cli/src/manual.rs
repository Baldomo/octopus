//! The verbs that send no single op: they run a process, edit this client's server list, or stream.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, anyhow};
use clap::{ArgAction, Subcommand};
use octopus_client::Identity;
use octopus_client::servers::{self, Address, Config, Server as KnownServer, Servers};
use octopus_proto::local::Listener;
use octopus_proto::ops::{TaskGet, TermTakeLock};
use octopus_proto::pairing::Invite;
use octopus_proto::remote;
use octopus_proto::types::{ReadDir, Stream, TaskId};
use octopus_server::TerminalHost;
use tokio::io::AsyncReadExt;

use crate::commands::SERVER_HELP;
use crate::install::{self, ServerRun, Service};
use crate::{account, attach, call, call_error, connect, print, skill};

#[derive(Subcommand)]
pub enum ServerVerb {
    /// Run this machine's server in the foreground.
    Run(ServerRun),
    /// Run the server at every login as a user service (launchd or systemd --user).
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
    /// Call a server by another name on every device of this account; the server is not told.
    Rename {
        #[arg(long, help = SERVER_HELP)]
        server: String,
        /// The name to call the server by.
        #[arg(long)]
        name: String,
    },
    /// Drop a server from this account's list on every device; the server keeps trusting the account.
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
                let ServerRun { name, socket, store, worktrees, terminal_host, udp, relay, microphone } = run;
                let local = Listener::bind(&socket).with_context(|| format!("bind {}", socket.display()))?;
                let host = TerminalHost { program: std::env::current_exe()?, address: terminal_host };
                let plugin = store.with_file_name(skill::PLUGIN_DIR);
                skill::write_plugin(&plugin)?;
                let server =
                    octopus_server::Server::open(&name, &store, &worktrees, &host, relay.clone(), microphone, &plugin)
                        .map_err(|error| anyhow!("{error}"))?;
                let endpoint = remote::listen(udp, relay.clone(), server.device()).await?;
                let key = server.device().public();
                let bound: Vec<String> = endpoint.bound_sockets().iter().map(ToString::to_string).collect();
                eprintln!(
                    "octopus server {name} listening on {}, and as {key} on UDP {} and {relay}",
                    socket.display(),
                    bound.join(", ")
                );
                if udp.ip().is_loopback() {
                    eprintln!(
                        "octopus server {name}: UDP {udp} is loopback, so clients on other machines reach it only through {relay}"
                    );
                }
                octopus_server::serve(server, local, endpoint).await;
                Ok(())
            }
            Self::Install { run, log } => print(&install::install(&Service::new(run, std::env::current_exe()?, log)?)?),
            Self::Uninstall { name } => print(&install::uninstall(&name)?),
            Self::TerminalHost { address } => {
                octopus_server::run_terminal_host(&address).await.with_context(|| format!("host {}", address.display()))
            }
            Self::Add { name, socket } => {
                print(&Servers::remember(&Config::user()?, KnownServer { name, address: Address::Local { socket } })?)
            }
            Self::Rename { server, name } => synced(|config, me| servers::rename(config, me, &server, &name)).await,
            Self::Forget { server } => synced(|config, me| servers::forget(config, me, &server)).await,
        }
    }
}

async fn synced(change: impl FnOnce(&Config, &Identity) -> anyhow::Result<()>) -> anyhow::Result<()> {
    let config = Config::user()?;
    let me = Identity::load(&config)?;
    change(&config, &me)?;
    account::sync_everywhere(&config, &me).await
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
            let (cols, rows) = crossterm::terminal::size()?;
            call(&connection, &TermTakeLock { task, panel, cols, rows }).await?;
        }
        attach::run(&connection, &Stream::Pty(pty)).await
    }
}

#[derive(Subcommand)]
pub enum FileVerb {
    /// Write a file of a task's directory to stdout, byte for byte.
    Read {
        #[arg(long, help = SERVER_HELP)]
        server: String,
        /// The task's id.
        #[arg(long)]
        task: String,
        /// The file, relative to the task's directory.
        #[arg(long)]
        path: String,
    },
    /// Create a file of a task's directory from stdin, byte for byte; fails conflict when the path exists.
    Upload {
        #[arg(long, help = SERVER_HELP)]
        server: String,
        /// The task's id.
        #[arg(long)]
        task: String,
        /// The new file, relative to the task's directory.
        #[arg(long)]
        path: String,
    },
}

impl FileVerb {
    pub async fn run(self) -> anyhow::Result<()> {
        match self {
            Self::Read { server, task, path } => {
                let connection = connect(&server).await?;
                let dir = ReadDir::Task(TaskId(task));
                let read = connection.read(dir, path.clone(), 0, Stream::TO_END).await;
                let mut bytes = Vec::new();
                read.map_err(call_error)?.1.read_to_end(&mut bytes).await.with_context(|| format!("read {path}"))?;
                std::io::stdout().write_all(&bytes).context("write stdout")
            }
            Self::Upload { server, task, path } => {
                let connection = connect(&server).await?;
                let mut content = Vec::new();
                tokio::io::stdin().read_to_end(&mut content).await.context("read stdin")?;
                connection
                    .upload(TaskId(task), path, content.len() as u64, content.as_slice())
                    .await
                    .map_err(call_error)?;
                print(&octopus_proto::ops::Done {})
            }
        }
    }
}

#[derive(Subcommand)]
pub enum PairingVerb {
    /// Pair with a server on another machine using the invite its `pairing offer` showed, and list it under its name.
    Accept {
        /// The one-time invite `pairing offer` printed: octopus://<server key>?code=…&relay=….
        #[arg(long)]
        invite: Invite,
    },
}

impl PairingVerb {
    pub async fn run(self) -> anyhow::Result<()> {
        let Self::Accept { invite } = self;
        let config = Config::user()?;
        let me = Identity::load(&config)?;
        let server = servers::pair(&invite, &me).await?;
        account::sync_everywhere(&config, &me).await?;
        print(&server)
    }
}
