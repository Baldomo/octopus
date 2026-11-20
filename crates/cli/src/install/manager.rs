//! The platform's user service manager, launchd on macOS and systemd on Linux, which starts this machine's server at
//! login.

use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};

use super::run;
use super::unit::ID;

pub use platform::{NAME, Service, exited, unregister_unit};

/// The user service manager serves the whole account, so a unit read or registered under any other HOME would run
/// in the account's real session, beside and outliving the account's own server.
pub fn account_home() -> anyhow::Result<PathBuf> {
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

pub fn unit_path(home: &Path, id: &str) -> PathBuf {
    platform::unit_directory(home).join(format!("{id}{}", platform::EXTENSION))
}

fn write_unit(unit: &Path, contents: String) -> anyhow::Result<()> {
    let directory = unit.parent().expect("a unit sits in a directory");
    std::fs::create_dir_all(directory).with_context(|| format!("create {}", directory.display()))?;
    std::fs::write(unit, contents).with_context(|| format!("write {}", unit.display()))
}

fn remove_unit(path: &Path) -> anyhow::Result<()> {
    std::fs::remove_file(path).with_context(|| format!("remove {}", path.display()))
}

#[cfg(target_os = "macos")]
mod platform {
    use std::process::Command;

    use super::*;
    use crate::install::binary::transient;
    use crate::install::unit::launchd_plist;

    pub const NAME: &str = "launchd";
    pub const EXTENSION: &str = ".plist";
    /// What `launchctl bootout` exits with for a job launchd doesn't hold.
    const NOT_LOADED: i32 = 3;

    pub fn unit_directory(home: &Path) -> PathBuf {
        home.join("Library").join("LaunchAgents")
    }

    fn target(id: &str) -> String {
        format!("gui/{}/{id}", nix::unistd::getuid())
    }

    fn bootout(id: &str) -> anyhow::Result<()> {
        let output = Command::new("launchctl").args(["bootout", &target(id)]).output().context("run launchctl")?;
        if !output.status.success() && output.status.code() != Some(NOT_LOADED) {
            bail!("launchctl bootout {}: {}", target(id), String::from_utf8_lossy(&output.stderr).trim());
        }
        Ok(())
    }

    /// This machine's server as launchd holds it: the agent in `~/Library/LaunchAgents`, read once per launch.
    pub struct Service {
        unit: PathBuf,
        written: Option<Vec<u8>>,
    }

    impl Service {
        pub fn read(home: &Path) -> Self {
            let unit = unit_path(home, ID);
            Self { written: std::fs::read(&unit).ok(), unit }
        }

        pub fn registered(&self) -> bool {
            self.written.is_some()
        }

        /// The program the agent runs, while the agent is as this install writes it and the program is there to
        /// stay.
        pub fn binary(&self) -> Option<PathBuf> {
            let argv =
                run("plutil", &["-extract", "ProgramArguments.0", "raw", "-o", "-", &self.unit.display().to_string()]);
            Some(PathBuf::from(argv.ok()?)).filter(|program| {
                self.written.as_deref() == Some(launchd_plist(program).as_bytes())
                    && program.is_file()
                    && !transient(program)
            })
        }

        /// The agent runs the launching binary where it is, which must stay there.
        pub fn put(&self, source: &Path) -> anyhow::Result<PathBuf> {
            if transient(source) {
                bail!(
                    "{} runs from a disk image or a translocated copy; move octopus to Applications and open it there",
                    source.display()
                );
            }
            Ok(source.to_path_buf())
        }

        /// Writes the agent running the server from `program` and has launchd start it afresh.
        pub fn register(&self, program: &Path) -> anyhow::Result<PathBuf> {
            write_unit(&self.unit, launchd_plist(program))?;
            bootout(ID)?;
            run(
                "launchctl",
                &["bootstrap", &format!("gui/{}", nix::unistd::getuid()), &self.unit.display().to_string()],
            )?;
            Ok(self.unit.clone())
        }

        pub fn unregister(&self) -> anyhow::Result<PathBuf> {
            unregister_unit(ID, &self.unit)?;
            Ok(self.unit.clone())
        }
    }

    pub fn unregister_unit(id: &str, unit: &Path) -> anyhow::Result<()> {
        bootout(id)?;
        remove_unit(unit)
    }

    /// Registering loads the service afresh, so any exit launchd recorded came since then.
    pub fn exited() -> anyhow::Result<bool> {
        let printed = run("launchctl", &["print", &target(ID)])?;
        Ok(printed.lines().map(str::trim).any(|line| {
            line.starts_with("last terminating signal = ")
                || line.strip_prefix("last exit code = ").is_some_and(|code| code != "(never exited)")
        }))
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use crate::install::binary;
    use crate::install::unit::systemd_unit;

    pub const NAME: &str = "systemd user manager";
    pub const EXTENSION: &str = ".service";

    pub fn unit_directory(home: &Path) -> PathBuf {
        home.join(".config").join("systemd").join("user")
    }

    fn systemctl(args: &[&str]) -> anyhow::Result<()> {
        run("systemctl", &[&["--user"], args].concat()).map(drop)
    }

    /// This machine's server as systemd holds it: the user unit running the server's copy, read once per launch.
    pub struct Service {
        home: PathBuf,
        unit: PathBuf,
        written: Option<Vec<u8>>,
    }

    impl Service {
        pub fn read(home: &Path) -> Self {
            let unit = unit_path(home, ID);
            Self { home: home.to_path_buf(), written: std::fs::read(&unit).ok(), unit }
        }

        pub fn registered(&self) -> bool {
            self.written.is_some()
        }

        /// The server's copy, while the unit is as this install writes it for the copy.
        pub fn binary(&self) -> Option<PathBuf> {
            binary::installed(&self.home).filter(|copy| self.written.as_deref() == Some(systemd_unit(copy).as_bytes()))
        }

        pub fn put(&self, source: &Path) -> anyhow::Result<PathBuf> {
            binary::put(&self.home, source)
        }

        /// Writes the unit running the server from `program` and restarts it.
        pub fn register(&self, program: &Path) -> anyhow::Result<PathBuf> {
            write_unit(&self.unit, systemd_unit(program))?;
            let file = format!("{ID}.service");
            systemctl(&["daemon-reload"])?;
            systemctl(&["enable", &file])?;
            systemctl(&["reset-failed", &file])?;
            systemctl(&["restart", &file])?;
            Ok(self.unit.clone())
        }

        pub fn unregister(&self) -> anyhow::Result<PathBuf> {
            unregister_unit(ID, &self.unit)?;
            Ok(self.unit.clone())
        }
    }

    pub fn unregister_unit(id: &str, unit: &Path) -> anyhow::Result<()> {
        systemctl(&["disable", "--now", &format!("{id}.service")])?;
        remove_unit(unit)?;
        systemctl(&["daemon-reload"])
    }

    /// Registering restarts the service by hand, which zeroes its count of restarts, so a restart since then, or a
    /// service that restarted too often and failed, is an exit.
    pub fn exited() -> anyhow::Result<bool> {
        let shown =
            run("systemctl", &["--user", "show", &format!("{ID}.service"), "--property=NRestarts,ActiveState"])?;
        Ok(shown.lines().any(|line| {
            line == "ActiveState=failed" || line.strip_prefix("NRestarts=").is_some_and(|count| count != "0")
        }))
    }
}
