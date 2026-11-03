//! The Octopus Microphone, the HAL plug-in a macOS server's terminals record from while a client on another
//! machine holds them.

use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use anyhow::Context;
use serde::Serialize;

#[cfg(target_os = "macos")]
use super::run;

#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[cfg_attr(not(target_os = "macos"), allow(dead_code, reason = "only macOS installs a plug-in"))]
pub enum Outcome {
    /// This install gives terminals no virtual microphone, or the platform needs no plug-in.
    NotAsked,
    /// The same plug-in was already installed, so nothing was asked.
    Unchanged,
    Installed,
    Removed,
    /// Another server on this machine still uses it.
    Kept,
    /// The prompt was declined or the copy failed; the server installed or uninstalled all the same.
    Failed {
        reason: String,
    },
}

#[cfg(any(target_os = "macos", test))]
const BUNDLE: &str = "OctopusMicrophone.driver";

#[cfg(target_os = "macos")]
const PLUG_INS: &str = "/Library/Audio/Plug-Ins/HAL";

#[cfg(target_os = "macos")]
macro_rules! built {
    ($path:literal) => {
        ($path, include_bytes!(concat!(env!("OUT_DIR"), "/OctopusMicrophone.driver/", $path)) as &[u8])
    };
}

#[cfg(target_os = "macos")]
const FILES: [(&str, &[u8]); 3] = [
    built!("Contents/Info.plist"),
    built!("Contents/MacOS/OctopusMicrophone"),
    built!("Contents/_CodeSignature/CodeResources"),
];

#[cfg(not(target_os = "macos"))]
pub fn install() -> Outcome {
    Outcome::NotAsked
}

#[cfg(not(target_os = "macos"))]
pub fn terminal_host(_unit: &Path) -> Option<PathBuf> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn uninstall(_terminal_host: Option<&Path>, _unit: &Path) -> Outcome {
    Outcome::NotAsked
}

/// The --terminal-host the launchd agent at `unit` runs its server with.
#[cfg(target_os = "macos")]
pub fn terminal_host(unit: &Path) -> Option<PathBuf> {
    let unit = unit.display().to_string();
    let argv = run("plutil", &["-extract", "ProgramArguments", "json", "-o", "-", &unit]).ok()?;
    let argv: Vec<String> = serde_json::from_str(&argv).ok()?;
    argv.windows(2).find(|pair| pair[0] == "--terminal-host").map(|pair| PathBuf::from(&pair[1]))
}

/// Gives back the input the server's microphone stood in for, then removes the plug-in once no other octopus
/// launchd agent remains beside `unit`.
#[cfg(target_os = "macos")]
pub fn uninstall(terminal_host: Option<&Path>, unit: &Path) -> Outcome {
    if let Some(terminal_host) = terminal_host
        && let Err(error) = octopus_server::give_back_input(terminal_host)
    {
        eprintln!("octopus: give back the default input: {error:#}");
    }
    let agents = unit.parent().expect("a launchd agent sits in a directory");
    let others = std::fs::read_dir(agents).into_iter().flatten().flatten().any(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        name.starts_with("octopus-") && name.ends_with(".plist")
    });
    let bundle = Path::new(PLUG_INS).join(BUNDLE);
    if others {
        return Outcome::Kept;
    }
    if !bundle.exists() {
        return Outcome::NotAsked;
    }
    match run("osascript", &["-e", &applescript(&removal(&bundle))]) {
        Ok(_) => Outcome::Removed,
        Err(error) => Outcome::Failed { reason: format!("{error:#}") },
    }
}

/// Run as root: coreaudiod drops a plug-in only as it restarts.
#[cfg(any(target_os = "macos", test))]
fn removal(bundle: &Path) -> String {
    format!("rm -rf {}; killall coreaudiod || true", quote(bundle))
}

/// Asks for an administrator only when the installed plug-in differs from this build's.
#[cfg(target_os = "macos")]
pub fn install() -> Outcome {
    let plug_ins = Path::new(PLUG_INS);
    if up_to_date(&plug_ins.join(BUNDLE), &FILES) {
        return Outcome::Unchanged;
    }
    match copy_as_administrator(plug_ins) {
        Ok(()) => Outcome::Installed,
        Err(error) => Outcome::Failed { reason: format!("{error:#}") },
    }
}

#[cfg(target_os = "macos")]
fn copy_as_administrator(plug_ins: &Path) -> anyhow::Result<()> {
    let stage = tempfile::tempdir().context("create a staging directory")?;
    let staged = stage.path().join(BUNDLE);
    for (path, bytes) in FILES {
        let file = staged.join(path);
        std::fs::create_dir_all(file.parent().expect("a bundle file sits in a directory"))?;
        std::fs::write(&file, bytes).with_context(|| format!("stage {}", file.display()))?;
    }
    run("osascript", &["-e", &applescript(&script(&staged, plug_ins))]).map(drop)
}

#[cfg(any(target_os = "macos", test))]
fn up_to_date(bundle: &Path, files: &[(&str, &[u8])]) -> bool {
    files.iter().all(|(path, bytes)| std::fs::read(bundle.join(path)).is_ok_and(|found| found == *bytes))
}

#[cfg(any(target_os = "macos", test))]
fn quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', r"'\''"))
}

/// Run as root: swaps the staged bundle in whole, then restarts coreaudiod, which loads plug-ins only as it
/// starts. launchd starts it again at once.
#[cfg(any(target_os = "macos", test))]
fn script(staged: &Path, plug_ins: &Path) -> String {
    let (installed, fresh) = (plug_ins.join(BUNDLE), plug_ins.join(format!("{BUNDLE}.new")));
    let (staged, plug_ins, installed, fresh) = (quote(staged), quote(plug_ins), quote(&installed), quote(&fresh));
    format!(
        "set -e; mkdir -p {plug_ins}; rm -rf {fresh}; cp -R {staged} {fresh}; chown -R root:wheel {fresh}; \
         chmod -R go-w {fresh}; rm -rf {installed}; mv {fresh} {installed}; killall coreaudiod || true"
    )
}

/// One prompt: AppleScript runs the whole script with the administrator's rights.
#[cfg(any(target_os = "macos", test))]
fn applescript(script: &str) -> String {
    let escaped = script.replace('\\', "\\\\").replace('"', "\\\"");
    format!("do shell script \"{escaped}\" with administrator privileges")
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use octopus_proto::types::MIC_DEVICE;

    use super::*;

    #[test]
    fn only_a_plug_in_differing_from_this_build_is_installed_again() {
        let bundle = tempfile::tempdir().expect("tempdir");
        let files: [(&str, &[u8]); 2] = [("Contents/Info.plist", b"plist"), ("Contents/MacOS/Mic", b"binary")];
        assert!(!up_to_date(bundle.path(), &files), "nothing installed");
        for (path, bytes) in files {
            let file = bundle.path().join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
        assert!(up_to_date(bundle.path(), &files));
        std::fs::write(bundle.path().join("Contents/MacOS/Mic"), b"older").unwrap();
        assert!(!up_to_date(bundle.path(), &files));
    }

    #[test]
    fn the_install_script_survives_quotes_in_its_paths() {
        let staged = Path::new("/tmp/it's \"staged\"/OctopusMicrophone.driver");
        let script = script(staged, Path::new("/Library/Audio/Plug-Ins/HAL"));
        let quoted = quote(staged);
        let echoed = Command::new("sh").args(["-c", &format!("printf %s {quoted}")]).output().unwrap().stdout;
        assert_eq!(String::from_utf8(echoed).unwrap(), staged.display().to_string());
        assert!(script.contains(&format!("cp -R {quoted} '/Library/Audio/Plug-Ins/HAL/OctopusMicrophone.driver.new'")));
        assert!(script.ends_with("killall coreaudiod || true"));
        let applescript = applescript(&script);
        let inner = applescript
            .strip_prefix("do shell script \"")
            .and_then(|rest| rest.strip_suffix("\" with administrator privileges"))
            .unwrap();
        assert_eq!(inner.replace("\\\"", "\"").replace("\\\\", "\\"), script);
        assert!(!inner.replace("\\\"", "").contains('"'), "every quote in the script is escaped");
    }

    #[test]
    fn removing_the_plug_in_restarts_the_audio_service() {
        let bundle = Path::new("/Library/Audio/Plug-Ins/HAL/OctopusMicrophone.driver");
        assert_eq!(removal(bundle), format!("rm -rf {}; killall coreaudiod || true", quote(bundle)));
    }

    #[test]
    fn the_driver_names_the_device_the_server_and_clients_look_for() {
        let driver = include_str!("../../microphone/OctopusMicrophone.c");
        for (define, value) in [("kName", MIC_DEVICE.name), ("kDeviceUID", MIC_DEVICE.uid)] {
            assert!(driver.contains(&format!("#define {define} \"{value}\"\n")), "{define} is {value:?}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn uninstall_finds_the_terminal_host_its_agent_runs_with() {
        let dir = tempfile::tempdir().unwrap();
        let unit = dir.path().join("octopus-mac-mini.plist");
        std::fs::write(
            &unit,
            crate::install::unit::launchd_plist(&crate::install::unit::tests::service("/var/octopus/server.log")),
        )
        .unwrap();
        assert_eq!(terminal_host(&unit), Some(PathBuf::from("/run/octopus/host.sock")));
    }
}
