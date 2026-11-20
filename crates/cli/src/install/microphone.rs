//! The virtual microphone `sudo octopus microphone install` readies on this machine, for the terminals a client on
//! another machine holds: on macOS the Octopus Microphone, a HAL plug-in; on Linux a sound server that can carry each
//! terminal's source.

#[cfg(any(target_os = "macos", test))]
use std::path::Path;

use anyhow::bail;
use serde::Serialize;

#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Outcome {
    Installed,
    /// On macOS this build's plug-in was already there; on Linux the sound server was already enabled.
    Unchanged,
    #[cfg_attr(target_os = "linux", allow(dead_code, reason = "octopus leaves the sound server on Linux"))]
    Removed,
    Absent,
}

fn as_root(verb: &str) -> anyhow::Result<()> {
    if !nix::unistd::geteuid().is_root() {
        bail!("run it as root: sudo octopus microphone {verb}");
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
const BUNDLE: &str = "OctopusMicrophone.driver";

#[cfg(target_os = "macos")]
const PLUG_INS: &str = "/Library/Audio/Plug-Ins/HAL";

#[cfg(target_os = "macos")]
macro_rules! built {
    ($path:literal, $mode:literal) => {
        BundleFile {
            path: $path,
            bytes: include_bytes!(concat!(env!("OUT_DIR"), "/OctopusMicrophone.driver/", $path)),
            mode: $mode,
        }
    };
}

#[cfg(any(target_os = "macos", test))]
struct BundleFile {
    path: &'static str,
    bytes: &'static [u8],
    mode: u32,
}

#[cfg(target_os = "macos")]
const FILES: [BundleFile; 3] = [
    built!("Contents/Info.plist", 0o644),
    built!("Contents/MacOS/OctopusMicrophone", 0o755),
    built!("Contents/_CodeSignature/CodeResources", 0o644),
];

/// coreaudiod loads plug-ins only as it starts, so the audio service restarts even when this build's plug-in was
/// already there: one that never loaded loads now.
#[cfg(target_os = "macos")]
pub fn install() -> anyhow::Result<Outcome> {
    as_root("install")?;
    let plug_ins = Path::new(PLUG_INS);
    let outcome = if up_to_date(&plug_ins.join(BUNDLE), &FILES) {
        Outcome::Unchanged
    } else {
        swap_in(plug_ins, &FILES)?;
        Outcome::Installed
    };
    restart_audio()?;
    Ok(outcome)
}

/// The default input falls back to another device as the plug-in goes.
#[cfg(target_os = "macos")]
pub fn uninstall() -> anyhow::Result<Outcome> {
    use anyhow::Context;

    as_root("uninstall")?;
    let bundle = Path::new(PLUG_INS).join(BUNDLE);
    if !bundle.exists() {
        return Ok(Outcome::Absent);
    }
    std::fs::remove_dir_all(&bundle).with_context(|| format!("remove {}", bundle.display()))?;
    restart_audio()?;
    Ok(Outcome::Removed)
}

/// launchd starts coreaudiod again at once.
#[cfg(target_os = "macos")]
fn restart_audio() -> anyhow::Result<()> {
    use anyhow::Context;

    super::run("killall", &["coreaudiod"]).map(drop).context("restart the audio service")
}

#[cfg(any(target_os = "macos", test))]
fn up_to_date(bundle: &Path, files: &[BundleFile]) -> bool {
    files.iter().all(|file| std::fs::read(bundle.join(file.path)).is_ok_and(|found| found == file.bytes))
}

/// Writes `files` beside the installed bundle, then swaps the whole bundle in, so coreaudiod never loads half of one.
#[cfg(any(target_os = "macos", test))]
fn swap_in(plug_ins: &Path, files: &[BundleFile]) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    use anyhow::Context;

    let (installed, fresh) = (plug_ins.join(BUNDLE), plug_ins.join(format!("{BUNDLE}.new")));
    if fresh.exists() {
        std::fs::remove_dir_all(&fresh).with_context(|| format!("remove {}", fresh.display()))?;
    }
    for BundleFile { path, bytes, mode } in files {
        let file = fresh.join(path);
        let directory = file.parent().expect("a bundle file sits in a directory");
        std::fs::create_dir_all(directory).with_context(|| format!("create {}", directory.display()))?;
        std::fs::write(&file, bytes).with_context(|| format!("write {}", file.display()))?;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(*mode))
            .with_context(|| format!("set the mode of {}", file.display()))?;
    }
    if installed.exists() {
        std::fs::remove_dir_all(&installed).with_context(|| format!("remove {}", installed.display()))?;
    }
    std::fs::rename(&fresh, &installed).with_context(|| format!("move {} into place", installed.display()))
}

/// Where logind keeps its seats, there while it runs; a machine without logind has no sessions running.
#[cfg(target_os = "linux")]
const LOGIND_SEATS: &str = "/run/systemd/seats";

/// Enables the sound server for every account's sessions and starts it in the sessions running now.
#[cfg(target_os = "linux")]
pub fn install() -> anyhow::Result<Outcome> {
    as_root("install")?;
    let found = |program: &str| {
        std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
    };
    let units = sound_server(found)?;
    let enabled = super::run("systemctl", &[&["--global", "is-enabled"][..], units].concat())
        .is_ok_and(|states| states.lines().all(|state| state == "enabled"));
    if !enabled {
        super::run("systemctl", &[&["--global", "enable"][..], units].concat())?;
    }
    let users = if std::path::Path::new(LOGIND_SEATS).exists() {
        super::run("loginctl", &["list-users", "--no-legend"])?
    } else {
        String::new()
    };
    let failed: Vec<String> = users
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter_map(|user| {
            let machine = format!("--machine={user}@");
            let started = super::run("systemctl", &[&["--user", machine.as_str(), "start"][..], units].concat());
            started.err().map(|error| format!("{user}: {error:#}"))
        })
        .collect();
    if !failed.is_empty() {
        bail!("start the sound server for {}", failed.join("; "));
    }
    Ok(if enabled { Outcome::Unchanged } else { Outcome::Installed })
}

/// octopus leaves the sound server to the machine.
#[cfg(target_os = "linux")]
pub fn uninstall() -> anyhow::Result<Outcome> {
    as_root("uninstall")?;
    Ok(Outcome::Absent)
}

/// The user units that start a sound server `pactl` reaches, by the programs `found` on the PATH.
#[cfg(any(target_os = "linux", test))]
fn sound_server(found: impl Fn(&str) -> bool) -> anyhow::Result<&'static [&'static str]> {
    let units: Option<&[&str]> = if found("pipewire-pulse") {
        Some(&["pipewire.socket", "pipewire-pulse.socket"])
    } else if found("pulseaudio") {
        Some(&["pulseaudio.socket"])
    } else {
        None
    };
    match (units, found("pactl")) {
        (Some(units), true) => Ok(units),
        (units, pactl) => {
            let missing: Vec<&str> =
                [(!pactl, "pactl"), (units.is_none(), "a sound server, pipewire-pulse or pulseaudio")]
                    .into_iter()
                    .filter_map(|(lacks, what)| lacks.then_some(what))
                    .collect();
            bail!(
                "this machine lacks {}: install PipeWire's pulse server and pactl, the pipewire-pulse and \
                 pulseaudio-utils packages on most distributions, then run this again",
                missing.join(" and ")
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use octopus_proto::types::MIC_DEVICE;

    use super::*;

    #[test]
    fn an_install_swaps_in_this_build_s_plug_in_whole() {
        let plug_ins = tempfile::tempdir().expect("tempdir");
        let files = [
            BundleFile { path: "Contents/Info.plist", bytes: b"plist", mode: 0o644 },
            BundleFile { path: "Contents/MacOS/Mic", bytes: b"binary", mode: 0o755 },
        ];
        let bundle = plug_ins.path().join(BUNDLE);
        assert!(!up_to_date(&bundle, &files), "nothing installed");
        std::fs::create_dir_all(bundle.join("Contents/Stale")).unwrap();
        std::fs::create_dir_all(plug_ins.path().join(format!("{BUNDLE}.new/Contents"))).unwrap();
        swap_in(plug_ins.path(), &files).unwrap();
        assert!(up_to_date(&bundle, &files));
        assert!(!bundle.join("Contents/Stale").exists(), "the older bundle goes whole");
        assert!(!plug_ins.path().join(format!("{BUNDLE}.new")).exists());
        let mode = |path: &str| std::fs::metadata(bundle.join(path)).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode("Contents/MacOS/Mic"), mode("Contents/Info.plist")), (0o755, 0o644));
        std::fs::write(bundle.join("Contents/MacOS/Mic"), b"older").unwrap();
        assert!(!up_to_date(&bundle, &files));
    }

    #[test]
    fn a_linux_install_names_what_the_machine_lacks() {
        let with = |programs: &'static [&'static str]| move |program: &str| programs.contains(&program);
        assert_eq!(
            sound_server(with(&["pipewire-pulse", "pulseaudio", "pactl"])).unwrap(),
            ["pipewire.socket", "pipewire-pulse.socket"]
        );
        assert_eq!(sound_server(with(&["pulseaudio", "pactl"])).unwrap(), ["pulseaudio.socket"]);
        let lacking = |programs| sound_server(with(programs)).unwrap_err().to_string();
        assert!(lacking(&["pactl"]).starts_with("this machine lacks a sound server, pipewire-pulse or pulseaudio:"));
        assert!(lacking(&["pulseaudio"]).starts_with("this machine lacks pactl:"));
        assert!(lacking(&[]).starts_with("this machine lacks pactl and a sound server, pipewire-pulse or pulseaudio:"));
    }

    #[test]
    fn the_driver_names_the_device_the_server_and_clients_look_for() {
        let driver = include_str!("../../microphone/OctopusMicrophone.c");
        for (define, value) in [("kName", MIC_DEVICE.name), ("kDeviceUID", MIC_DEVICE.uid)] {
            assert!(driver.contains(&format!("#define {define} \"{value}\"\n")), "{define} is {value:?}");
        }
    }
}
