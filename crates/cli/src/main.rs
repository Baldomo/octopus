mod attach;
mod install;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, anyhow};
use clap::{ArgAction, Parser, Subcommand};
use install::{ServerRun, Service};
use octopus_client::servers::{Server as KnownServer, Servers};
use octopus_client::{CallError, Connection};
use octopus_proto::Op;
use octopus_proto::local::Listener;
use octopus_proto::ops::{
    AgentReport, HarnessSet, TaskCreate, TaskFocus, TaskGet, TaskMove, TermResize, TermSend, TermTakeLock,
    WorkspaceAdd, WorkspaceList,
};
use octopus_proto::types::{AgentStatus, Checkout, Stream, TaskId, TaskState, WorkspaceId, from_wire};
use octopus_server::TerminalHost;
use serde::Serialize;

#[derive(Parser)]
#[command(name = "octopus", about = "Drive octopus servers. Every flag is required.")]
struct Cli {
    #[command(subcommand)]
    verb: Verb,
}

#[derive(Subcommand)]
enum Verb {
    /// Open the window onto every server in this client's list.
    App,
    #[command(flatten)]
    Command(Command),
}

#[derive(Subcommand)]
enum Command {
    #[command(subcommand)]
    Server(ServerCommand),
    #[command(subcommand)]
    Workspace(WorkspaceCommand),
    #[command(subcommand)]
    Harness(HarnessCommand),
    #[command(subcommand)]
    Agent(AgentCommand),
    #[command(subcommand)]
    Task(TaskCommand),
    #[command(subcommand)]
    Term(TermCommand),
}

#[derive(Subcommand)]
enum ServerCommand {
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
        #[arg(long)]
        name: String,
    },
    /// Run the process that keeps a server's terminals alive across server restarts; `server run` starts it.
    TerminalHost {
        /// The socket to serve, the server's --terminal-host.
        #[arg(long)]
        address: PathBuf,
    },
    /// Remember a server on this machine's local socket under a name.
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        socket: PathBuf,
    },
}

#[derive(Subcommand)]
enum WorkspaceCommand {
    Add {
        #[arg(long)]
        server: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        root: String,
    },
    List {
        #[arg(long)]
        server: String,
    },
}

#[derive(Subcommand)]
enum HarnessCommand {
    Set {
        #[arg(long)]
        server: String,
        #[arg(long)]
        id: String,
        /// The agent's argv as a JSON array, e.g. '["claude"]'.
        #[arg(long)]
        argv: String,
        #[arg(long)]
        install: String,
        /// Whether the agent reports its status with `octopus agent report`.
        #[arg(long, action = ArgAction::Set)]
        hooks: bool,
    },
}

#[derive(Subcommand)]
enum AgentCommand {
    /// Report the agent's status; harness hooks call this.
    Report {
        #[arg(long)]
        server: String,
        #[arg(long)]
        task: String,
        #[arg(long, value_parser = from_wire::<AgentStatus>)]
        status: AgentStatus,
    },
}

#[derive(Subcommand)]
enum TaskCommand {
    Create {
        #[arg(long)]
        server: String,
        #[arg(long)]
        workspace: String,
        #[arg(long)]
        title: String,
        /// A task id, or none for a root task.
        #[arg(long)]
        parent: String,
        #[arg(long, value_parser = from_wire::<TaskState>)]
        state: TaskState,
        #[arg(long, value_parser = from_wire::<Checkout>)]
        checkout: Checkout,
        #[arg(long)]
        harness: String,
    },
    Move {
        #[arg(long)]
        server: String,
        #[arg(long)]
        task: String,
        #[arg(long, value_parser = from_wire::<TaskState>)]
        state: TaskState,
    },
    Focus {
        #[arg(long)]
        server: String,
        #[arg(long)]
        task: String,
    },
    Get {
        #[arg(long)]
        server: String,
        #[arg(long)]
        task: String,
    },
}

#[derive(Subcommand)]
enum TermCommand {
    Send {
        #[arg(long)]
        server: String,
        #[arg(long)]
        task: String,
        #[arg(long)]
        panel: String,
        #[arg(long)]
        text: String,
        #[arg(long, action = ArgAction::Set)]
        enter: bool,
        #[arg(long, action = ArgAction::Set)]
        take_lock: bool,
    },
    /// Stream a terminal into this one; Ctrl-] detaches.
    Attach {
        #[arg(long)]
        server: String,
        #[arg(long)]
        task: String,
        #[arg(long)]
        panel: String,
        #[arg(long, action = ArgAction::Set)]
        take_lock: bool,
    },
}

async fn connect(server: &str) -> anyhow::Result<Connection> {
    let servers = Servers::load()?;
    let known = servers.get(server)?;
    let client = gethostname::gethostname().to_string_lossy().to_string();
    Connection::connect(&known.socket, &client).await
}

/// A server error prints as its JSON wire shape so scripts can match its code.
pub(crate) fn call_error(error: CallError) -> anyhow::Error {
    match error {
        CallError::Server(error) => anyhow!("{}", serde_json::to_string(&error).expect("errors serialize")),
        CallError::Transport(error) => error,
    }
}

async fn call<O: Op>(connection: &Connection, op: &O) -> anyhow::Result<O::Ok> {
    connection.call(op).await.map_err(call_error)
}

fn print(value: &impl Serialize) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

async fn workspace_id(connection: &Connection, name: &str) -> anyhow::Result<WorkspaceId> {
    let workspaces = call(connection, &WorkspaceList {}).await?.workspaces;
    workspaces
        .into_iter()
        .find(|workspace| workspace.name == name)
        .map(|workspace| workspace.id)
        .ok_or_else(|| anyhow!("no workspace named {name}"))
}

async fn run(command: Command) -> anyhow::Result<()> {
    match command {
        Command::Server(ServerCommand::Run(run)) => {
            run.check()?;
            let ServerRun { name, socket, store, worktrees, terminal_host } = run;
            let listener = Listener::bind(&socket).with_context(|| format!("bind {}", socket.display()))?;
            let host = TerminalHost { program: std::env::current_exe()?, address: terminal_host };
            let server =
                octopus_server::Server::open(&name, &store, &worktrees, &host).map_err(|error| anyhow!("{error}"))?;
            eprintln!("octopus server {name} listening on {}", socket.display());
            octopus_server::serve(server, listener).await;
            Ok(())
        }
        Command::Server(ServerCommand::Install { run, log }) => {
            print(&install::install(&Service::new(run, std::env::current_exe()?, log)?)?)
        }
        Command::Server(ServerCommand::Uninstall { name }) => print(&install::uninstall(&name)?),
        Command::Server(ServerCommand::TerminalHost { address }) => {
            octopus_server::run_terminal_host(&address).await.with_context(|| format!("host {}", address.display()))
        }
        Command::Server(ServerCommand::Add { name, socket }) => {
            let mut servers = Servers::load()?;
            servers.add(KnownServer { name, socket })?;
            servers.save()?;
            print(&servers)
        }
        Command::Workspace(WorkspaceCommand::Add { server, name, root }) => {
            let connection = connect(&server).await?;
            print(&call(&connection, &WorkspaceAdd { name, root }).await?)
        }
        Command::Workspace(WorkspaceCommand::List { server }) => {
            let connection = connect(&server).await?;
            print(&call(&connection, &WorkspaceList {}).await?)
        }
        Command::Harness(HarnessCommand::Set { server, id, argv, install, hooks }) => {
            let argv: Vec<String> = serde_json::from_str(&argv).context("--argv must be a JSON array of strings")?;
            let connection = connect(&server).await?;
            print(&call(&connection, &HarnessSet { id, argv, install, hooks }).await?)
        }
        Command::Agent(AgentCommand::Report { server, task, status }) => {
            let connection = connect(&server).await?;
            print(&call(&connection, &AgentReport { task: TaskId(task), status }).await?)
        }
        Command::Task(TaskCommand::Create { server, workspace, title, parent, state, checkout, harness }) => {
            let connection = connect(&server).await?;
            let workspace = workspace_id(&connection, &workspace).await?;
            let parent = (parent != "none").then_some(TaskId(parent));
            print(&call(&connection, &TaskCreate { workspace, title, parent, state, checkout, harness }).await?)
        }
        Command::Task(TaskCommand::Move { server, task, state }) => {
            let connection = connect(&server).await?;
            print(&call(&connection, &TaskMove { task: TaskId(task), state }).await?)
        }
        Command::Task(TaskCommand::Focus { server, task }) => {
            let connection = connect(&server).await?;
            print(&call(&connection, &TaskFocus { task: TaskId(task) }).await?)
        }
        Command::Task(TaskCommand::Get { server, task }) => {
            let connection = connect(&server).await?;
            print(&call(&connection, &TaskGet { task: TaskId(task) }).await?)
        }
        Command::Term(TermCommand::Send { server, task, panel, text, enter, take_lock }) => {
            let connection = connect(&server).await?;
            print(&call(&connection, &TermSend { task: TaskId(task), panel, text, enter, take_lock }).await?)
        }
        Command::Term(TermCommand::Attach { server, task, panel, take_lock }) => {
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
}

/// The app owns the main thread, which the macOS event loop requires; every other command runs on tokio.
fn main() -> ExitCode {
    let outcome = match Cli::parse().verb {
        Verb::App => octopus_app::run(),
        Verb::Command(command) => tokio::runtime::Runtime::new()
            .context("start the runtime")
            .and_then(|runtime| runtime.block_on(run(command))),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
