//! The unit a service manager runs this machine's server from, `<binary> server start`, as a launchd plist or a
//! systemd unit, and the `server run` flags a server takes.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::bail;
use octopus_proto::remote::{self, RelayUrl};

#[derive(clap::Args)]
pub struct ServerRun {
    /// The server's name, which agents in its terminals see as OCTOPUS_SERVER.
    #[arg(long)]
    pub name: String,
    /// The Unix socket clients on this machine connect to.
    #[arg(long)]
    pub socket: PathBuf,
    /// The SQLite file the server keeps its state in.
    #[arg(long)]
    pub store: PathBuf,
    /// Where task worktrees live: <worktrees>/<repo>/<task>.
    #[arg(long)]
    pub worktrees: PathBuf,
    /// Socket of the terminal host, the process that keeps this server's terminals running across restarts;
    /// started if nothing answers there.
    #[arg(long)]
    pub terminal_host: PathBuf,
    /// The UDP address clients on other machines reach this server on directly: 0.0.0.0:0 for a free port the OS
    /// picks at each start, or a fixed one like 0.0.0.0:7300 to forward on a router. 0.0.0.0 takes IPv6 too, on the
    /// same port when it is fixed.
    #[arg(long)]
    pub udp: SocketAddr,
    /// The iroh relay this server is homed on, which clients on other machines dial it through: one of n0's,
    /// e.g. https://euc1-1.relay.n0.iroh.link./, or one of your own.
    #[arg(long, value_parser = remote::relay)]
    pub relay: RelayUrl,
    /// Give each terminal a virtual microphone that its lock holder feeds.
    #[arg(long, action = clap::ArgAction::Set)]
    pub microphone: bool,
}

impl ServerRun {
    /// A server that dialled its own socket for its host would wait on itself forever. Compared without case,
    /// as macOS resolves names.
    pub fn check(&self) -> anyhow::Result<()> {
        let (socket, host) = (self.socket.display().to_string(), self.terminal_host.display().to_string());
        if socket.eq_ignore_ascii_case(&host) {
            bail!("--socket and --terminal-host must differ");
        }
        Ok(())
    }
}

/// The service manager's name for this machine's server. A server named by an earlier install went by
/// `octopus-<name>`, which the dash keeps apart from this one.
pub const ID: &str = "octopus";

/// The id an earlier install registered the server `name` under: each character a service manager refuses, any
/// but ASCII letters, digits, '-', '_' and '.', became '-'.
pub fn legacy_id(name: &str) -> String {
    let safe = |c: char| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' };
    format!("octopus-{}", name.chars().map(safe).collect::<String>())
}

fn argv(program: &Path) -> [String; 3] {
    [program.display().to_string(), "server".into(), "start".into()]
}

/// The command the unit runs, each word quoted for a POSIX shell.
pub fn command(program: &Path) -> String {
    let quoted = |word: &String| {
        let plain = word.chars().all(|c| c.is_ascii_alphanumeric() || "/._-:=,+@".contains(c));
        if plain { word.clone() } else { format!("'{}'", word.replace('\'', "'\\''")) }
    };
    argv(program).iter().map(quoted).collect::<Vec<_>>().join(" ")
}

/// The login item's plist, `crates/desktop/server.plist`, which the app bundle carries in
/// `Contents/Library/LaunchAgents` and launchd finds wherever the app moves. `AbandonProcessGroup` keeps launchd from
/// ending the terminal host along with the server.
#[cfg(any(target_os = "macos", test))]
pub const BUNDLED_PLIST: &str = "octopus.plist";

#[cfg(any(target_os = "linux", test))]
fn systemd_word(word: &str) -> String {
    format!("\"{}\"", word.replace('%', "%%").replace('\\', "\\\\").replace('"', "\\\"").replace('$', "$$"))
}

/// `KillMode=process` stops the server alone, so its terminal host and every agent outlive a restart.
#[cfg(any(target_os = "linux", test))]
pub fn systemd_unit(program: &Path) -> String {
    let exec: Vec<String> = argv(program).iter().map(|word| systemd_word(word)).collect();
    format!(
        "[Unit]
Description=octopus server

[Service]
ExecStart={exec}
Restart=always
KillMode=process

[Install]
WantedBy=default.target
",
        exec = exec.join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_bundles_a_login_item_that_starts_the_server_and_spares_its_terminal_host() {
        let plist = include_str!("../../../desktop/server.plist");
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../../../desktop/tauri.conf.json")).expect("tauri.conf.json is JSON");
        let identifier = conf["identifier"].as_str().expect("an identifier");
        let arguments: String =
            ["octopus", "server", "start"].map(|word| format!("\t\t<string>{word}</string>\n")).concat();
        for entry in [
            format!("<key>Label</key>\n\t<string>{ID}</string>"),
            "<key>BundleProgram</key>\n\t<string>Contents/MacOS/octopus</string>".into(),
            format!("<key>ProgramArguments</key>\n\t<array>\n{arguments}\t</array>"),
            format!("<key>AssociatedBundleIdentifiers</key>\n\t<array>\n\t\t<string>{identifier}</string>"),
            "<key>RunAtLoad</key>\n\t<true/>".into(),
            "<key>KeepAlive</key>\n\t<true/>".into(),
            "<key>AbandonProcessGroup</key>\n\t<true/>".into(),
        ] {
            assert!(plist.contains(&entry), "{entry} in {plist}");
        }
        let files = &conf["bundle"]["macOS"]["files"];
        assert_eq!(files[format!("Library/LaunchAgents/{BUNDLED_PLIST}")], "server.plist", "the bundle carries it");
    }

    #[test]
    fn a_systemd_unit_quotes_each_word_and_stops_the_server_alone() {
        let unit = systemd_unit(Path::new("/home/ada/100% \"real\"/octopus"));
        assert!(unit.contains("\nExecStart=\"/home/ada/100%% \\\"real\\\"/octopus\" \"server\" \"start\"\n"), "{unit}");
        assert!(unit.contains("\nRestart=always\nKillMode=process\n"), "{unit}");
    }

    #[test]
    fn an_earlier_install_named_its_unit_after_the_server() {
        assert_eq!(legacy_id("mac-mini.2"), "octopus-mac-mini.2");
        assert_eq!(legacy_id("Ada’s Mac"), "octopus-Ada-s-Mac");
        assert_ne!(legacy_id(""), ID);
    }

    #[test]
    fn a_server_never_dials_its_own_socket_for_its_host() {
        let run = ServerRun {
            name: "mac-mini".into(),
            socket: "/run/octopus/server.sock".into(),
            store: "/var/octopus/store.db".into(),
            worktrees: "/var/octopus/worktrees".into(),
            terminal_host: "/RUN/OCTOPUS/SERVER.SOCK".into(),
            udp: ([0, 0, 0, 0], 0).into(),
            relay: remote::N0_EU_RELAY.parse().expect("n0's relay is a URL"),
            microphone: true,
        };
        assert!(run.check().is_err(), "one socket, spelled in another case");
    }
}
