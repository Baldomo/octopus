//! The unit a service manager runs a server from: the `server run` flags, the service they make, and the launchd
//! plist and systemd unit that carry it.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, bail};
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
    /// The UDP address clients on other machines reach this server on directly, e.g. 0.0.0.0:7300, which takes IPv6
    /// on the same port too.
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

    fn flags(&self) -> [(&'static str, String); 8] {
        [
            ("--name", self.name.clone()),
            ("--socket", self.socket.display().to_string()),
            ("--store", self.store.display().to_string()),
            ("--worktrees", self.worktrees.display().to_string()),
            ("--terminal-host", self.terminal_host.display().to_string()),
            ("--udp", self.udp.to_string()),
            ("--relay", self.relay.to_string()),
            ("--microphone", self.microphone.to_string()),
        ]
    }
}

pub struct Service {
    pub(super) id: String,
    pub(super) run: ServerRun,
    program: PathBuf,
    pub(super) log: PathBuf,
}

/// Every service manager accepts ASCII letters, digits, '-', '_' and '.', so each other character becomes '-'.
pub(super) fn service_id(name: &str) -> anyhow::Result<String> {
    if name.is_empty() {
        bail!("--name must not be empty");
    }
    let safe = |c: char| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' };
    Ok(format!("octopus-{}", name.chars().map(safe).collect::<String>()))
}

fn absolute(path: PathBuf) -> anyhow::Result<PathBuf> {
    std::path::absolute(&path).with_context(|| format!("resolve {}", path.display()))
}

impl Service {
    /// Service managers start in their own working directory, so every path is made absolute. A service listens
    /// beyond loopback, since a loopback address leaves clients on other machines the relay alone.
    pub fn new(run: ServerRun, program: PathBuf, log: PathBuf) -> anyhow::Result<Self> {
        if run.udp.ip().is_loopback() {
            bail!("--udp {} is loopback; install with --udp 0.0.0.0:{}", run.udp, run.udp.port());
        }
        let id = service_id(&run.name)?;
        let run = ServerRun {
            socket: absolute(run.socket)?,
            store: absolute(run.store)?,
            worktrees: absolute(run.worktrees)?,
            terminal_host: absolute(run.terminal_host)?,
            ..run
        };
        run.check()?;
        let service = Self { id, run, program: absolute(program)?, log: absolute(log)? };
        let mut words = service.argv().into_iter().chain([service.log.display().to_string()]);
        if let Some(word) = words.find(|word| word.chars().any(char::is_control)) {
            bail!("{word:?} holds a control character, which no unit file can carry");
        }
        Ok(service)
    }

    fn argv(&self) -> Vec<String> {
        let mut argv = vec![self.program.display().to_string(), "server".into(), "run".into()];
        for (flag, value) in self.run.flags() {
            argv.extend([flag.to_string(), value]);
        }
        argv
    }
}

#[cfg(any(target_os = "macos", test))]
fn xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(any(target_os = "macos", test))]
fn plist_word(word: &str) -> String {
    format!("\t\t<string>{}</string>\n", xml(word))
}

/// `AbandonProcessGroup` keeps launchd from ending the terminal host along with the server.
#[cfg(any(target_os = "macos", test))]
pub fn launchd_plist(service: &Service) -> String {
    let arguments: String = service.argv().iter().map(|argument| plist_word(argument)).collect();
    let (label, log) = (&service.id, xml(&service.log.display().to_string()));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
{arguments}	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<true/>
	<key>AbandonProcessGroup</key>
	<true/>
	<key>StandardOutPath</key>
	<string>{log}</string>
	<key>StandardErrorPath</key>
	<string>{log}</string>
</dict>
</plist>
"#
    )
}

#[cfg(any(target_os = "linux", test))]
fn specifiers(text: &str) -> String {
    text.replace('%', "%%")
}

#[cfg(any(target_os = "linux", test))]
fn systemd_word(word: &str) -> String {
    format!("\"{}\"", specifiers(word).replace('\\', "\\\\").replace('"', "\\\"").replace('$', "$$"))
}

/// `KillMode=process` stops the server alone, so its terminal host and every agent outlive a restart.
#[cfg(any(target_os = "linux", test))]
pub fn systemd_unit(service: &Service) -> String {
    let exec: Vec<String> = service.argv().iter().map(|argument| systemd_word(argument)).collect();
    let (name, log) = (&service.run.name, specifiers(&service.log.display().to_string()));
    format!(
        "[Unit]
Description=octopus server {name}

[Service]
ExecStart={exec}
Restart=always
KillMode=process
StandardOutput=append:{log}
StandardError=append:{log}

[Install]
WantedBy=default.target
",
        exec = exec.join(" ")
    )
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub(in crate::install) fn service(log: &str) -> Service {
        let run = ServerRun {
            name: "mac-mini".into(),
            socket: "/run/octopus/mac mini.sock".into(),
            store: "/var/octopus/store.db".into(),
            worktrees: "/var/octopus/worktrees".into(),
            terminal_host: "/run/octopus/host.sock".into(),
            udp: ([0, 0, 0, 0], 7300).into(),
            relay: remote::N0_EU_RELAY.parse().expect("n0's relay is a URL"),
            microphone: true,
        };
        Service { id: "octopus-mac-mini".into(), run, program: "/opt/octopus/bin/octopus".into(), log: log.into() }
    }

    const ARGV: [&str; 19] = [
        "/opt/octopus/bin/octopus",
        "server",
        "run",
        "--name",
        "mac-mini",
        "--socket",
        "/run/octopus/mac mini.sock",
        "--store",
        "/var/octopus/store.db",
        "--worktrees",
        "/var/octopus/worktrees",
        "--terminal-host",
        "/run/octopus/host.sock",
        "--udp",
        "0.0.0.0:7300",
        "--relay",
        "https://euc1-1.relay.n0.iroh.link./",
        "--microphone",
        "true",
    ];

    #[test]
    fn a_launchd_agent_runs_the_server_at_login_and_spares_its_terminal_host() {
        let plist = launchd_plist(&service("/var/octopus/server.log"));
        let array: String = ARGV.iter().map(|argument| format!("\t\t<string>{argument}</string>\n")).collect();
        assert!(plist.contains(&format!("<key>ProgramArguments</key>\n\t<array>\n{array}\t</array>")), "{plist}");
        for key in ["RunAtLoad", "KeepAlive", "AbandonProcessGroup"] {
            assert!(plist.contains(&format!("<key>{key}</key>\n\t<true/>")), "{key} in {plist}");
        }
        assert!(plist.contains("<key>Label</key>\n\t<string>octopus-mac-mini</string>"));
        assert!(plist.contains("<key>StandardErrorPath</key>\n\t<string>/var/octopus/server.log</string>"));
    }

    #[test]
    fn a_systemd_unit_quotes_each_word_and_stops_the_server_alone() {
        let exec: Vec<String> = ARGV.iter().map(|word| format!("\"{word}\"")).collect();
        let expected = format!(
            "[Unit]
Description=octopus server mac-mini

[Service]
ExecStart={}
Restart=always
KillMode=process
StandardOutput=append:/var/octopus/100%%.log
StandardError=append:/var/octopus/100%%.log

[Install]
WantedBy=default.target
",
            exec.join(" ")
        );
        assert_eq!(systemd_unit(&service("/var/octopus/100%.log")), expected);
    }

    #[test]
    fn a_service_never_installs_on_loopback() {
        for udp in ["127.0.0.1:7300", "[::1]:7300"] {
            let mut run = service("/log").run;
            run.udp = udp.parse().expect("an address");
            let error = Service::new(run, "/opt/octopus/bin/octopus".into(), "/log".into()).err().expect("refused");
            assert!(error.to_string().contains("--udp 0.0.0.0:7300"), "{error}");
        }
    }

    #[test]
    fn a_service_takes_one_safe_name_and_distinct_sockets() {
        assert_eq!(service_id("mac-mini.2").expect("id"), "octopus-mac-mini.2");
        assert_eq!(service_id("Ada’s Mac").expect("id"), "octopus-Ada-s-Mac");
        assert!(service_id("").is_err());
        let mut shared = service("/log").run;
        shared.terminal_host = PathBuf::from(shared.socket.display().to_string().to_uppercase());
        assert!(shared.check().is_err(), "one socket, spelled in another case");
    }
}
