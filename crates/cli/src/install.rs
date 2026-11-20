//! Registers this machine's server with the platform's user service manager, started at login.

pub mod binary;
mod manager;
pub mod microphone;
mod unit;

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};
use serde::Serialize;

pub use manager::{Service, account_home, exited};
pub use unit::ServerRun;

#[derive(Serialize)]
pub struct Registration {
    pub service: String,
    pub unit: PathBuf,
    pub microphone: microphone::Outcome,
}

/// Returns the command's trimmed stdout.
fn run(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new(program).args(args).output().with_context(|| format!("run {program}"))?;
    if !output.status.success() {
        bail!("{program} {}: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Where this machine's server keeps everything, `~/.octopus` within the account's home.
pub fn octopus_dir(home: &Path) -> PathBuf {
    home.join(".octopus")
}

/// Registers this machine's server from `program` and has the service manager start it afresh. The server installs
/// whatever becomes of the Octopus Microphone.
pub fn install(service: &Service, program: &Path) -> anyhow::Result<Registration> {
    let microphone = microphone::install();
    if let microphone::Outcome::Failed { reason } = &microphone {
        eprintln!("octopus: installed without the Octopus Microphone: {reason}");
    }
    let unit = service.register(program).with_context(|| {
        format!(
            "the {} did not start the server: start one in this session, or run the server by hand with `{}`",
            manager::NAME,
            unit::command(program)
        )
    })?;
    Ok(Registration { service: unit::ID.into(), unit, microphone })
}

/// The unit an earlier install registered the server `name` under, when it is still there.
pub fn legacy_unit(home: &Path, name: &str) -> Option<PathBuf> {
    Some(manager::unit_path(home, &unit::legacy_id(name))).filter(|unit| unit.exists())
}

/// Unregisters the unit an earlier install registered the server `name` under. Its terminal host and agents keep
/// running for the server that replaces it.
pub fn retire(name: &str, unit: &Path) -> anyhow::Result<()> {
    manager::unregister_unit(&unit::legacy_id(name), unit)
}

/// The terminal host and its agents keep running; they end with the machine. The user's input comes back through
/// `terminal_host`, and the Octopus Microphone goes with the server.
pub fn uninstall(service: &Service, terminal_host: &Path) -> anyhow::Result<Registration> {
    let unit = service.unregister()?;
    let microphone = microphone::uninstall(terminal_host);
    if let microphone::Outcome::Failed { reason } = &microphone {
        eprintln!("octopus: uninstalled, leaving the Octopus Microphone: {reason}");
    }
    Ok(Registration { service: unit::ID.into(), unit, microphone })
}
