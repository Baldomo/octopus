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

fn remove_unit(path: &Path) -> anyhow::Result<()> {
    std::fs::remove_file(path).with_context(|| format!("remove {}", path.display()))
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code, reason = "ServiceManagement's SMAppService and libproc's proc_pidpath have no safe binding")]
mod platform {
    use std::os::unix::ffi::OsStringExt;
    use std::process::Command;

    use objc2_foundation::NSString;
    use objc2_service_management::{SMAppService, SMAppServiceStatus};

    use super::*;
    use crate::install::binary::{bundle, transient};
    use crate::install::unit::BUNDLED_PLIST;

    pub const NAME: &str = "launchd";
    pub const EXTENSION: &str = ".plist";
    /// What `launchctl bootout` exits with for a job launchd doesn't hold.
    const NOT_LOADED: i32 = 3;
    /// The manager `launchctl print` names for a job an app registered through SMAppService.
    const LOGIN_ITEMS: &str = "managed_by = com.apple.xpc.ServiceManagement";

    /// Where earlier installs wrote their agents.
    pub fn unit_directory(home: &Path) -> PathBuf {
        home.join("Library").join("LaunchAgents")
    }

    fn target(id: &str) -> String {
        format!("gui/{}/{id}", nix::unistd::getuid())
    }

    fn printed() -> Option<String> {
        run("launchctl", &["print", &target(ID)]).ok()
    }

    fn login_item() -> objc2::rc::Retained<SMAppService> {
        // SAFETY: a plist name is all agentServiceWithPlistName reads.
        unsafe { SMAppService::agentServiceWithPlistName(&NSString::from_str(BUNDLED_PLIST)) }
    }

    fn status() -> SMAppServiceStatus {
        // SAFETY: status reads the login item's state.
        unsafe { login_item().status() }
    }

    /// The executable process `pid` runs, as the kernel resolved it.
    fn executable(pid: libc::pid_t) -> Option<PathBuf> {
        let mut path = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        // SAFETY: proc_pidpath writes at most the buffer's size into it and returns how many bytes it wrote.
        let written = unsafe { libc::proc_pidpath(pid, path.as_mut_ptr().cast(), path.len() as u32) };
        let written = usize::try_from(written).ok().filter(|&written| written > 0)?;
        path.truncate(written);
        Some(PathBuf::from(std::ffi::OsString::from_vec(path)))
    }

    /// This machine's server as the Mac app's login item, read once per launch.
    pub struct Service {
        bundle: PathBuf,
        /// The pid of the server launchd runs for the login item; None while the login item runs none.
        pid: Option<libc::pid_t>,
        login_item: bool,
    }

    impl Service {
        /// Only the running app reaches its own login item, so a binary outside any app manages no server.
        pub fn read(_home: &Path) -> anyhow::Result<Option<Self>> {
            let Some(bundle) = bundle(&std::env::current_exe()?).map(Path::to_path_buf) else { return Ok(None) };
            let printed = printed().unwrap_or_default();
            let lines = || printed.lines().map(str::trim);
            Ok(Some(Self {
                bundle,
                pid: lines().find_map(|line| line.strip_prefix("pid = ")?.parse().ok()),
                login_item: lines().any(|line| line == LOGIN_ITEMS),
            }))
        }

        pub fn registered(&self) -> bool {
            self.login_item
        }

        /// The binary the login item's server runs, while that one is there to stay. Launchd finds the bundle
        /// wherever it moves, so the login item always runs the plist the bundle carries.
        pub fn binary(&self) -> Option<PathBuf> {
            executable(self.pid?).filter(|program| program.is_file() && !transient(program))
        }

        /// The login item runs this app, which must stay where it is.
        pub fn put(&self, source: &Path) -> anyhow::Result<PathBuf> {
            if bundle(source) != Some(self.bundle.as_path()) {
                bail!("{} is a newer octopus; open it to repair this machine's server", source.display());
            }
            if transient(source) {
                bail!(
                    "{} runs from a disk image, a translocated copy or the Trash; move octopus to Applications and open it there",
                    source.display()
                );
            }
            Ok(source.to_path_buf())
        }

        /// Registers this app's login item afresh, so launchd runs this bundle wherever it sits now. A login item
        /// the user turned off fails it, opening System Settings at Login Items.
        pub fn register(&self, _program: &Path) -> anyhow::Result<PathBuf> {
            self.unregister()?;
            // SAFETY: registering the app's own login item.
            let registered = unsafe { login_item().registerAndReturnError() };
            if status() == SMAppServiceStatus::RequiresApproval {
                // SAFETY: opens System Settings at Login Items.
                unsafe { SMAppService::openSystemSettingsLoginItems() };
                bail!("octopus is off in System Settings > General > Login Items; turn it on there");
            }
            registered.map_err(|error| anyhow!("register the login item: {error}"))?;
            Ok(self.plist())
        }

        /// A login item that never registered reads NotFound, and unregistering it is refused.
        pub fn unregister(&self) -> anyhow::Result<PathBuf> {
            if [SMAppServiceStatus::Enabled, SMAppServiceStatus::RequiresApproval].contains(&status()) {
                // SAFETY: unregistering the app's own login item.
                unsafe { login_item().unregisterAndReturnError() }
                    .map_err(|error| anyhow!("unregister the login item: {error}"))?;
            }
            Ok(self.plist())
        }

        fn plist(&self) -> PathBuf {
            self.bundle.join("Contents/Library/LaunchAgents").join(BUNDLED_PLIST)
        }
    }

    pub fn unregister_unit(id: &str, unit: &Path) -> anyhow::Result<()> {
        let output = Command::new("launchctl").args(["bootout", &target(id)]).output().context("run launchctl")?;
        if !output.status.success() && output.status.code() != Some(NOT_LOADED) {
            bail!("launchctl bootout {}: {}", target(id), String::from_utf8_lossy(&output.stderr).trim());
        }
        remove_unit(unit)
    }

    /// Registering loads the service afresh, so any exit launchd recorded came since then. A login item launchd has
    /// yet to load has not exited.
    pub fn exited() -> anyhow::Result<bool> {
        Ok(printed().is_some_and(|printed| {
            printed.lines().map(str::trim).any(|line| {
                line.starts_with("last terminating signal = ")
                    || line.strip_prefix("last exit code = ").is_some_and(|code| code != "(never exited)")
            })
        }))
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn a_process_resolves_to_the_executable_it_runs() {
            let pid = libc::pid_t::try_from(std::process::id()).expect("a pid fits");
            assert_eq!(super::executable(pid), Some(std::env::current_exe().expect("this test's binary")));
        }
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

    fn write_unit(unit: &Path, contents: String) -> anyhow::Result<()> {
        let directory = unit.parent().expect("a unit sits in a directory");
        std::fs::create_dir_all(directory).with_context(|| format!("create {}", directory.display()))?;
        std::fs::write(unit, contents).with_context(|| format!("write {}", unit.display()))
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
        pub fn read(home: &Path) -> anyhow::Result<Option<Self>> {
            let unit = unit_path(home, ID);
            Ok(Some(Self { home: home.to_path_buf(), written: std::fs::read(&unit).ok(), unit }))
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
