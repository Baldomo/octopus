//! Registers `octopus server run` with the platform's user service manager, started at login.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, anyhow, bail};
use serde::Serialize;

#[derive(clap::Args)]
pub struct ServerRun {
    /// The server's name, which agents in its terminals see as OCTOPUS_SERVER.
    #[arg(long)]
    pub name: String,
    /// A Unix socket path, or a named pipe `\\.\pipe\<name>` on Windows.
    #[arg(long)]
    pub socket: PathBuf,
    /// The SQLite file the server keeps its state in.
    #[arg(long)]
    pub store: PathBuf,
    /// Where task worktrees live: <worktrees>/<workspace>/<task>.
    #[arg(long)]
    pub worktrees: PathBuf,
    /// Socket of the terminal host, the process that keeps this server's terminals running across restarts;
    /// started if nothing answers there.
    #[arg(long)]
    pub terminal_host: PathBuf,
    /// The UDP address QUIC listens on for clients on other machines, e.g. 0.0.0.0:7300.
    #[arg(long)]
    pub quic: SocketAddr,
}

impl ServerRun {
    /// A server that dialled its own socket for its host would wait on itself forever. Compared without case,
    /// as macOS and Windows resolve names.
    pub fn check(&self) -> anyhow::Result<()> {
        let (socket, host) = (self.socket.display().to_string(), self.terminal_host.display().to_string());
        if socket.eq_ignore_ascii_case(&host) {
            bail!("--socket and --terminal-host must differ");
        }
        Ok(())
    }

    fn flags(&self) -> [(&'static str, String); 6] {
        [
            ("--name", self.name.clone()),
            ("--socket", self.socket.display().to_string()),
            ("--store", self.store.display().to_string()),
            ("--worktrees", self.worktrees.display().to_string()),
            ("--terminal-host", self.terminal_host.display().to_string()),
            ("--quic", self.quic.to_string()),
        ]
    }
}

pub struct Service {
    id: String,
    run: ServerRun,
    program: PathBuf,
    log: PathBuf,
}

#[derive(Serialize)]
pub struct Registration {
    pub service: String,
    pub unit: PathBuf,
}

fn service_id(name: &str) -> anyhow::Result<String> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    if name.is_empty() || !name.chars().all(allowed) {
        bail!("--name {name:?} must be letters, digits, '-', '_' or '.'");
    }
    Ok(format!("octopus-{name}"))
}

fn absolute(path: PathBuf) -> anyhow::Result<PathBuf> {
    std::path::absolute(&path).with_context(|| format!("resolve {}", path.display()))
}

impl Service {
    /// Service managers start in their own working directory, so every path is made absolute.
    pub fn new(run: ServerRun, program: PathBuf, log: PathBuf) -> anyhow::Result<Self> {
        let id = service_id(&run.name)?;
        let run = ServerRun {
            name: run.name,
            socket: absolute(run.socket)?,
            store: absolute(run.store)?,
            worktrees: absolute(run.worktrees)?,
            terminal_host: absolute(run.terminal_host)?,
            quic: run.quic,
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

#[cfg(any(target_os = "macos", windows, test))]
fn xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// `AbandonProcessGroup` keeps launchd from ending the terminal host along with the server.
#[cfg(any(target_os = "macos", test))]
pub fn launchd_plist(service: &Service) -> String {
    let arguments: String =
        service.argv().iter().map(|argument| format!("\t\t<string>{}</string>\n", xml(argument))).collect();
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

/// `KillMode=process` stops the server alone, so its terminal host and every agent outlive a restart.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
pub fn systemd_unit(service: &Service) -> String {
    let specifiers = |text: &str| text.replace('%', "%%");
    let word = |text: &String| {
        format!("\"{}\"", specifiers(text).replace('\\', "\\\\").replace('"', "\\\"").replace('$', "$$"))
    };
    let exec: Vec<String> = service.argv().iter().map(word).collect();
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

/// Quotes one argument for cmd: backslashes before the closing quote are doubled so they stay literal.
#[cfg(any(windows, test))]
fn windows_word(word: &str) -> anyhow::Result<String> {
    if word.contains(['"', '%']) {
        bail!("{word} holds a '\"' or '%', which a Windows command line cannot carry");
    }
    let trailing = word.len() - word.trim_end_matches('\\').len();
    Ok(format!("\"{word}{}\"", "\\".repeat(trailing)))
}

/// Runs headless under conhost so no console window opens at logon; Task Scheduler has no output redirection,
/// so cmd appends to the log.
#[cfg(any(windows, test))]
pub fn windows_task(service: &Service, user: &str) -> anyhow::Result<String> {
    let words: Vec<String> = service.argv().iter().map(|word| windows_word(word)).collect::<anyhow::Result<_>>()?;
    let log = windows_word(&service.log.display().to_string())?;
    let arguments = xml(&format!("--headless cmd.exe /d /s /c \"{} >> {log} 2>&1\"", words.join(" ")));
    let (name, user) = (xml(&service.run.name), xml(user));
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>octopus server {name}</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>999</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>conhost.exe</Command>
      <Arguments>{arguments}</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    ))
}

/// Returns the command's trimmed stdout.
fn run(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new(program).args(args).output().with_context(|| format!("run {program}"))?;
    if !output.status.success() {
        bail!("{program} {}: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn remove_unit(path: &Path) -> anyhow::Result<()> {
    std::fs::remove_file(path).with_context(|| format!("remove {}", path.display()))
}

pub fn install(service: &Service) -> anyhow::Result<Registration> {
    let unit = manager::unit_path(&service.id)?;
    for directory in [&unit, &service.log].map(|path| path.parent().expect("an absolute file path has a directory")) {
        std::fs::create_dir_all(directory).with_context(|| format!("create {}", directory.display()))?;
    }
    std::fs::write(&unit, manager::unit(service)?).with_context(|| format!("write {}", unit.display()))?;
    manager::register(&service.id, &unit)?;
    Ok(Registration { service: service.id.clone(), unit })
}

/// The terminal host and its agents keep running; they end with the machine.
pub fn uninstall(name: &str) -> anyhow::Result<Registration> {
    let id = service_id(name)?;
    let unit = manager::unit_path(&id)?;
    manager::unregister(&id, &unit)?;
    Ok(Registration { service: id, unit })
}

#[cfg(target_os = "macos")]
mod manager {
    use super::*;

    pub fn unit_path(id: &str) -> anyhow::Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("no home directory"))?;
        Ok(home.join("Library").join("LaunchAgents").join(format!("{id}.plist")))
    }

    pub fn unit(service: &Service) -> anyhow::Result<Vec<u8>> {
        Ok(launchd_plist(service).into_bytes())
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
}

#[cfg(all(unix, not(target_os = "macos")))]
mod manager {
    use super::*;

    pub fn unit_path(id: &str) -> anyhow::Result<PathBuf> {
        let config = dirs::config_dir().ok_or_else(|| anyhow!("no config directory"))?;
        Ok(config.join("systemd").join("user").join(format!("{id}.service")))
    }

    pub fn unit(service: &Service) -> anyhow::Result<Vec<u8>> {
        Ok(systemd_unit(service).into_bytes())
    }

    fn systemctl(args: &[&str]) -> anyhow::Result<()> {
        run("systemctl", &[&["--user"], args].concat()).map(drop)
    }

    pub fn register(id: &str, _unit: &Path) -> anyhow::Result<()> {
        let file = format!("{id}.service");
        systemctl(&["daemon-reload"])?;
        systemctl(&["enable", &file])?;
        systemctl(&["restart", &file])
    }

    pub fn unregister(id: &str, unit: &Path) -> anyhow::Result<()> {
        systemctl(&["disable", "--now", &format!("{id}.service")])?;
        remove_unit(unit)?;
        systemctl(&["daemon-reload"])
    }
}

#[cfg(windows)]
mod manager {
    use super::*;

    pub fn unit_path(id: &str) -> anyhow::Result<PathBuf> {
        let config = dirs::config_dir().ok_or_else(|| anyhow!("no config directory"))?;
        Ok(config.join("octopus").join(format!("{id}.xml")))
    }

    /// Task Scheduler reads task XML as UTF-16.
    pub fn unit(service: &Service) -> anyhow::Result<Vec<u8>> {
        const BOM: u16 = 0xfeff;
        let task = windows_task(service, &run("whoami", &[])?)?;
        Ok(std::iter::once(BOM).chain(task.encode_utf16()).flat_map(u16::to_le_bytes).collect())
    }

    pub fn register(id: &str, unit: &Path) -> anyhow::Result<()> {
        run("schtasks", &["/Create", "/TN", id, "/XML", &unit.display().to_string(), "/F"])?;
        run("schtasks", &["/Run", "/TN", id]).map(drop)
    }

    pub fn unregister(id: &str, unit: &Path) -> anyhow::Result<()> {
        if let Err(error) = run("schtasks", &["/End", "/TN", id]) {
            eprintln!("octopus: {id} was not running: {error}");
        }
        run("schtasks", &["/Delete", "/TN", id, "/F"])?;
        remove_unit(unit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(log: &str) -> Service {
        let run = ServerRun {
            name: "mac-mini".into(),
            socket: "/run/octopus/mac mini.sock".into(),
            store: "/var/octopus/store.db".into(),
            worktrees: "/var/octopus/worktrees".into(),
            terminal_host: "/run/octopus/host.sock".into(),
            quic: ([0, 0, 0, 0], 7300).into(),
        };
        Service { id: "octopus-mac-mini".into(), run, program: "/opt/octopus/bin/octopus".into(), log: log.into() }
    }

    const ARGV: [&str; 15] = [
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
        "--quic",
        "0.0.0.0:7300",
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
    fn a_windows_logon_task_runs_the_server_headless_for_its_user_without_a_time_limit() {
        let task = windows_task(&service(r"C:\Users\ada\octopus.log"), r"pc\ada").expect("task");
        let quoted: Vec<String> = ARGV.iter().map(|word| format!("\"{word}\"")).collect();
        let command = format!(
            r#"--headless cmd.exe /d /s /c "{} &gt;&gt; "C:\Users\ada\octopus.log" 2&gt;&amp;1""#,
            quoted.join(" ")
        );
        assert!(task.contains(&format!("<Arguments>{command}</Arguments>")), "{task}");
        assert!(task.contains("<LogonTrigger>\n      <Enabled>true</Enabled>\n      <UserId>pc\\ada</UserId>"));
        assert!(task.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert_eq!(windows_word(r"C:\octopus\").expect("word"), r#""C:\octopus\\""#);
        assert!(windows_task(&service("/var/octopus/100%.log"), r"pc\ada").is_err(), "'%' cannot be quoted");
    }

    #[test]
    fn a_service_takes_one_safe_name_and_distinct_sockets() {
        assert_eq!(service_id("mac-mini.2").expect("id"), "octopus-mac-mini.2");
        assert!(service_id("mac mini").is_err());
        let ServerRun { name, socket, store, worktrees, quic, .. } = service("/log").run;
        let host = PathBuf::from(socket.display().to_string().to_uppercase());
        let shared = ServerRun { name, terminal_host: host, socket, store, worktrees, quic };
        assert!(shared.check().is_err(), "one socket, spelled in another case");
    }
}
