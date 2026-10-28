mod attach;
pub mod commands;
mod install;
mod manual;
pub mod skill;

use std::process::ExitCode;

use anyhow::{Context, anyhow};
use octopus_client::servers::Servers;
use octopus_client::{CallError, Connection, Identity};
use octopus_proto::{Error, Op};
use serde::Serialize;

/// A malformed call exits with clap's usage status; a call the server or transport fails exits 1.
const USAGE: u8 = 2;

async fn connect(server: &str) -> anyhow::Result<Connection> {
    let servers = Servers::load()?;
    let connection = Connection::connect(&servers.get(server)?.address, &Identity::load()?).await?;
    Servers::refresh(server, connection.server_addresses())?;
    Ok(connection)
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

/// The app owns the main thread, which the macOS event loop requires; every other command runs on tokio.
pub fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let matches = match commands::tree().try_get_matches_from(&argv) {
        Ok(matches) => matches,
        Err(error) => match commands::missing_flags(&error, &argv) {
            Some(missing) => {
                eprintln!("error: {missing}");
                return ExitCode::from(USAGE);
            }
            None => {
                install::hand_off_old_service(&argv);
                error.exit()
            }
        },
    };
    let outcome = match matches.subcommand() {
        Some(("app", _)) => octopus_app::run(
            install::list_this_machine().err().map(|error| format!("list this machine's server: {error:#}")),
        ),
        _ => tokio::runtime::Runtime::new()
            .context("start the runtime")
            .and_then(|runtime| runtime.block_on(commands::run(&matches))),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
