//! The platform's user service manager, launchd on macOS and systemd on Linux, which starts a server's unit at login.

use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};

use super::run;
use super::unit::Service;
#[cfg(target_os = "macos")]
use super::unit::launchd_plist;
#[cfg(target_os = "linux")]
use super::unit::systemd_unit;

#[cfg(feature = "desktop")]
pub use platform::exited;
pub use platform::{NAME, register, unit_file, unregister};

fn remove_unit(path: &Path) -> anyhow::Result<()> {
    std::fs::remove_file(path).with_context(|| format!("remove {}", path.display()))
}

/// The user service manager serves the whole account, so a unit read or registered under any other HOME would run
/// in the account's real session, beside and outliving the account's own server.
fn account_home() -> anyhow::Result<PathBuf> {
    let account = nix::unistd::User::from_uid(nix::unistd::getuid())
        .context("look up this account")?
        .ok_or_else(|| anyhow!("this user has no account entry"))?;
    let home = dirs::home_dir().ok_or_else(|| anyhow!("no home directory"))?;
    if home != account.dir {
        bail!(
            "HOME is {}, not this account's home {}, so octopus manages no service here",
            home.display(),
            account.dir.display()
        );
    }
    Ok(home)
}

pub fn unit_path(id: &str) -> anyhow::Result<PathBuf> {
    Ok(platform::unit_directory()?.join(format!("{id}{}", platform::EXTENSION)))
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    pub const NAME: &str = "launchd";
    pub const EXTENSION: &str = ".plist";

    pub fn unit_directory() -> anyhow::Result<PathBuf> {
        Ok(account_home()?.join("Library").join("LaunchAgents"))
    }

    pub fn unit_file(service: &Service) -> Vec<u8> {
        launchd_plist(service).into_bytes()
    }

    fn domain() -> String {
        format!("gui/{}", nix::unistd::getuid())
    }

    fn unload_if_loaded(id: &str) -> anyhow::Result<()> {
        let target = format!("{}/{id}", domain());
        if run("launchctl", &["print", &target]).is_ok() {
            run("launchctl", &["bootout", &target])?;
        }
        Ok(())
    }

    pub fn register(id: &str, unit: &Path) -> anyhow::Result<()> {
        unload_if_loaded(id)?;
        run("launchctl", &["bootstrap", &domain(), &unit.display().to_string()]).map(drop)
    }

    pub fn unregister(id: &str, unit: &Path) -> anyhow::Result<()> {
        unload_if_loaded(id)?;
        remove_unit(unit)
    }

    /// Registering loads the service afresh, so any exit launchd recorded came since then.
    #[cfg(feature = "desktop")]
    pub fn exited(id: &str) -> anyhow::Result<bool> {
        let printed = run("launchctl", &["print", &format!("{}/{id}", domain())])?;
        Ok(printed.lines().map(str::trim).any(|line| {
            line.starts_with("last terminating signal = ")
                || line.strip_prefix("last exit code = ").is_some_and(|code| code != "(never exited)")
        }))
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;

    pub const NAME: &str = "systemd user manager";
    pub const EXTENSION: &str = ".service";

    pub fn unit_directory() -> anyhow::Result<PathBuf> {
        Ok(account_home()?.join(".config").join("systemd").join("user"))
    }

    pub fn unit_file(service: &Service) -> Vec<u8> {
        systemd_unit(service).into_bytes()
    }

    fn systemctl(args: &[&str]) -> anyhow::Result<()> {
        run("systemctl", &[&["--user"], args].concat()).map(drop)
    }

    pub fn register(id: &str, _unit: &Path) -> anyhow::Result<()> {
        let file = format!("{id}.service");
        systemctl(&["daemon-reload"])?;
        systemctl(&["enable", &file])?;
        systemctl(&["reset-failed", &file])?;
        systemctl(&["restart", &file])
    }

    pub fn unregister(id: &str, unit: &Path) -> anyhow::Result<()> {
        systemctl(&["disable", "--now", &format!("{id}.service")])?;
        remove_unit(unit)?;
        systemctl(&["daemon-reload"])
    }

    /// Registering restarts the service by hand, which zeroes its count of restarts, so a restart since then, or a
    /// service that restarted too often and failed, is an exit.
    #[cfg(feature = "desktop")]
    pub fn exited(id: &str) -> anyhow::Result<bool> {
        let file = format!("{id}.service");
        let shown = run("systemctl", &["--user", "show", &file, "--property=NRestarts,ActiveState"])?;
        Ok(shown.lines().any(|line| {
            line == "ActiveState=failed" || line.strip_prefix("NRestarts=").is_some_and(|count| count != "0")
        }))
    }
}
