mod account;
mod attach;
pub mod commands;
mod install;
mod manual;
mod repair;
pub mod skill;

use std::process::ExitCode;

use anyhow::{Context, anyhow};
use octopus_client::servers::{Config, Servers};
use octopus_client::{CallError, Connection, Identity};
use octopus_proto::{Error, Op};
use serde::Serialize;

/// A malformed call exits with clap's usage status; a call the server or transport fails exits 1.
const USAGE: u8 = 2;
const ERROR_PREFIX: &str = "error: ";

async fn connect(server: &str) -> anyhow::Result<Connection> {
    let config = Config::user()?;
    let me = Identity::load(&config)?;
    connect_as(&config, &me, server).await
}

/// Connects to a server this client reaches by name: one on this machine, or one its account lists.
async fn connect_as(config: &Config, me: &Identity, server: &str) -> anyhow::Result<Connection> {
    Connection::connect(&Servers::load(config)?.reach(me, server)?.address, me).await
}

/// A server error prints as its JSON wire shape so scripts can match its code.
fn server_error(error: &Error) -> anyhow::Error {
    anyhow!("{}", serde_json::to_string(error).expect("errors serialize"))
}

pub(crate) fn call_error(error: CallError) -> anyhow::Error {
    match error {
        CallError::Server(error) => server_error(&error),
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

/// The server's own processes leave this machine's server as they find it.
fn runs_the_server(noun: &str, verbs: &clap::ArgMatches) -> bool {
    noun == "server" && matches!(verbs.subcommand_name(), Some("run" | "start" | "terminal-host"))
}

/// The app owns the main thread, which the macOS event loop requires; every other command runs on tokio.
pub fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    // Finder, the Dock and desktop entries start the app's executable with no arguments.
    #[cfg(feature = "desktop")]
    let argv = if argv.len() == 1 { vec![argv[0].clone(), "app".to_string()] } else { argv };
    let matches = match commands::tree().try_get_matches_from(&argv) {
        Ok(matches) => matches,
        Err(error) => match commands::missing_flags(&error, &argv) {
            Some(missing) => {
                eprintln!("{ERROR_PREFIX}{missing}");
                return ExitCode::from(USAGE);
            }
            None => error.exit(),
        },
    };
    let outcome = match matches.subcommand() {
        #[cfg(feature = "desktop")]
        Some(("app", _)) => {
            let (problem, install) = match repair::app() {
                Ok(install) => (None, install),
                Err(error) => (Some(format!("repair this machine's server: {error:#}")), None),
            };
            octopus_desktop::run(octopus_desktop::Launch {
                problem,
                install,
                skill: octopus_desktop::Skill(skill::install),
                #[cfg(target_os = "macos")]
                transient: install::binary::transient_bundle(),
            })
        }
        Some(("install", _)) => repair::install().and_then(|registration| print(&registration)),
        Some(("uninstall", _)) => repair::uninstall().and_then(|registration| print(&registration)),
        Some((noun, verbs)) => {
            if !runs_the_server(noun, verbs) {
                repair::cli();
            }
            tokio::runtime::Runtime::new()
                .context("start the runtime")
                .and_then(|runtime| runtime.block_on(commands::run(&matches)))
        }
        None => unreachable!("the tree requires a noun"),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{ERROR_PREFIX}{error:#}");
            ExitCode::FAILURE
        }
    }
}
