//! Registers `octopus server run` with the platform's user service manager, started at login.

mod manager;
pub mod microphone;
mod unit;

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, bail};
use serde::Serialize;

pub use unit::{ServerRun, Service};

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

/// The server installs whatever becomes of the Octopus Microphone.
pub fn install(service: &Service) -> anyhow::Result<Registration> {
    let unit = manager::unit_path(&service.id)?;
    for directory in [&unit, &service.log].map(|path| path.parent().expect("an absolute file path has a directory")) {
        std::fs::create_dir_all(directory).with_context(|| format!("create {}", directory.display()))?;
    }
    std::fs::write(&unit, manager::unit_file(service)).with_context(|| format!("write {}", unit.display()))?;
    let microphone = if service.run.microphone { microphone::install() } else { microphone::Outcome::NotAsked };
    if let microphone::Outcome::Failed { reason } = &microphone {
        eprintln!("octopus: installed without the Octopus Microphone: {reason}");
    }
    manager::register(&service.id, &unit)?;
    Ok(Registration { service: service.id.clone(), unit, microphone })
}

/// The terminal host and its agents keep running; they end with the machine. The user's input comes back, and the
/// Octopus Microphone goes with the machine's last server.
pub fn uninstall(name: &str) -> anyhow::Result<Registration> {
    let id = unit::service_id(name)?;
    let unit = manager::unit_path(&id)?;
    let terminal_host = microphone::terminal_host(&unit);
    manager::unregister(&id, &unit)?;
    let microphone = microphone::uninstall(terminal_host.as_deref(), &unit);
    if let microphone::Outcome::Failed { reason } = &microphone {
        eprintln!("octopus: uninstalled, leaving the Octopus Microphone: {reason}");
    }
    Ok(Registration { service: id, unit, microphone })
}
