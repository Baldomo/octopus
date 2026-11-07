//! The terminal host: a process of its own that keeps every terminal, so terminals outlive the server process.
//!
//! Each connection to the host opens with one `HostRequest` line and gets one `Outcome` line back. An `attach`
//! connection then carries the terminal's pieces, its screen and then its live output; an `input` connection carries
//! bytes to it.

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
#[cfg(not(target_os = "macos"))]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
#[cfg(not(target_os = "macos"))]
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::Pid;
use octopus_proto::local::{self, BlockingStream, Listener};
use octopus_proto::term::Piece;
use octopus_proto::types::PtyId;
use octopus_proto::{Error, Outcome, line};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::broadcast;

use crate::pty::{Activity, Attachment, Pty};
use crate::{lock, report};

/// Bumped whenever an existing request or reply changes shape; a server refuses a host that speaks another. A host
/// older than a request added since refuses that request.
const PROTOCOL: u32 = 3;
const HOST_START: Duration = Duration::from_secs(5);
const HOST_START_POLL: Duration = Duration::from_millis(5);
const INPUT_BYTES: usize = 4096;

#[derive(Debug, Clone)]
pub struct TerminalHost {
    /// An `octopus` binary: the host runs as `<program> server terminal-host --address <address>`.
    pub program: PathBuf,
    pub address: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum HostRequest {
    Spawn { id: PtyId, argv: Vec<String>, cwd: PathBuf, env: Vec<(String, String)> },
    List,
    Attach { id: PtyId },
    Input { id: PtyId },
    Resize { id: PtyId, cols: u16, rows: u16 },
    Terminate { id: PtyId },
    Forget { id: PtyId },
    WaitExit { id: PtyId },
    Activity { id: PtyId },
    Program { id: PtyId },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Listing {
    protocol: u32,
    terminals: Vec<Hosted>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Hosted {
    pub id: PtyId,
    pub running: bool,
}

type Terminals = Arc<Mutex<HashMap<PtyId, Arc<Pty>>>>;

/// What a connection carries once its request is answered.
enum Then {
    Close,
    Output(Attachment),
    Input(Arc<Pty>),
}

/// Serves terminals at `address` until the address is gone, ending every terminal on the way out.
pub async fn run_terminal_host(address: &Path) -> std::io::Result<()> {
    let listener = Listener::bind(address)?;
    let unreachable = unreachable(address)?;
    let terminals = Terminals::default();
    let serving = Arc::clone(&terminals);
    tokio::spawn(local::accept_forever(listener, move |stream| {
        drop(tokio::spawn(serve(Arc::clone(&serving), stream)))
    }));
    unreachable.await;
    let terminals = lock(&terminals);
    report!("{} is gone; ending its {} terminals", address.display(), terminals.len());
    terminals.values().for_each(|pty| pty.kill());
    Ok(())
}

const UNREACHABLE_CHECK: Duration = Duration::from_secs(1);

fn identity(address: &Path) -> std::io::Result<(u64, u64)> {
    std::fs::metadata(address).map(|metadata| (metadata.dev(), metadata.ino()))
}

/// Resolves once the socket file the host bound is gone or replaced: nothing can reach the host again.
fn unreachable(address: &Path) -> std::io::Result<impl Future<Output = ()> + use<'_>> {
    let bound = identity(address)?;
    Ok(async move {
        loop {
            tokio::time::sleep(UNREACHABLE_CHECK).await;
            match identity(address) {
                Ok(current) if current == bound => {}
                Ok(_) => return,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                Err(error) => report!("check {}: {error}", address.display()),
            }
        }
    })
}

fn find(terminals: &Terminals, id: &PtyId) -> Result<Arc<Pty>, Error> {
    lock(terminals).get(id).cloned().ok_or_else(|| Error::not_found(id))
}

async fn answer<R: AsyncRead + Unpin>(
    terminals: &Terminals,
    request: HostRequest,
    read: &mut R,
) -> Result<(Value, Then), Error> {
    match request {
        HostRequest::Spawn { id, argv, cwd, env } => {
            let already = || Error::conflict(format!("{id} is already hosted"));
            if lock(terminals).contains_key(&id) {
                return Err(already());
            }
            let pty =
                tokio::task::spawn_blocking(move || Pty::spawn(&argv, &cwd, &env)).await.map_err(Error::internal)??;
            let mut terminals = lock(terminals);
            if terminals.contains_key(&id) {
                tokio::spawn(pty.terminate());
                return Err(already());
            }
            terminals.insert(id, pty);
            Ok((Value::Null, Then::Close))
        }
        HostRequest::List => {
            let terminals =
                lock(terminals).iter().map(|(id, pty)| Hosted { id: id.clone(), running: pty.running() }).collect();
            let listing = serde_json::to_value(Listing { protocol: PROTOCOL, terminals }).map_err(Error::internal)?;
            Ok((listing, Then::Close))
        }
        HostRequest::Attach { id } => Ok((Value::Null, Then::Output(find(terminals, &id)?.attach()))),
        HostRequest::Input { id } => Ok((Value::Null, Then::Input(find(terminals, &id)?))),
        HostRequest::Resize { id, cols, rows } => {
            find(terminals, &id)?.resize(cols, rows)?;
            Ok((Value::Null, Then::Close))
        }
        HostRequest::Terminate { id } => {
            tokio::spawn(find(terminals, &id)?.terminate());
            Ok((Value::Null, Then::Close))
        }
        HostRequest::Forget { id } => {
            let pty = lock(terminals).remove(&id).ok_or_else(|| Error::not_found(&id))?;
            tokio::spawn(pty.terminate());
            Ok((Value::Null, Then::Close))
        }
        HostRequest::WaitExit { id } => {
            let pty = find(terminals, &id)?;
            let mut probe = [0u8; 1];
            tokio::select! {
                () = pty.exit() => {}
                // The read returns when the waiter hangs up, which frees this task.
                _ = read.read(&mut probe) => {}
            }
            Ok((Value::Null, Then::Close))
        }
        HostRequest::Activity { id } => {
            let pty = find(terminals, &id)?;
            let activity = tokio::task::spawn_blocking(move || pty.activity()).await.map_err(Error::internal)??;
            Ok((serde_json::to_value(activity).map_err(Error::internal)?, Then::Close))
        }
        HostRequest::Program { id } => {
            let program = find(terminals, &id)?.program().map(|pid| pid.as_raw());
            Ok((serde_json::to_value(program).map_err(Error::internal)?, Then::Close))
        }
    }
}

async fn serve<S: AsyncRead + AsyncWrite + Send + 'static>(terminals: Terminals, stream: S) {
    let (read, mut write) = tokio::io::split(stream);
    let mut read = BufReader::new(read);
    let mut first = String::new();
    if read.read_line(&mut first).await.unwrap_or(0) == 0 {
        return;
    }
    let answered = match serde_json::from_str::<HostRequest>(&first) {
        Ok(request) => answer(&terminals, request, &mut read).await,
        Err(error) => Err(Error::invalid(error)),
    };
    let (reply, then) = answered.map_or_else(|error| (Err(error), Then::Close), |(value, then)| (Ok(value), then));
    if write.write_all(line(&Outcome::from(reply)).as_bytes()).await.is_err() {
        return;
    }
    match then {
        Then::Close => {}
        Then::Output(attachment) => {
            let _peer_gone = stream_output(attachment, &mut write).await;
        }
        Then::Input(pty) => forward_input(pty, read).await,
    }
}

/// Ends an attachment that falls behind, so its client never sees output with a gap.
async fn stream_output<W: AsyncWrite + Unpin>(attachment: Attachment, write: &mut W) -> std::io::Result<()> {
    Piece::Screen(attachment.screen).write_to(write).await?;
    if let Some(mut live) = attachment.live {
        loop {
            match live.recv().await {
                Ok(piece) => piece.write_to(write).await?,
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    report!("closing an attachment {missed} pieces behind");
                    break;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
    write.shutdown().await
}

/// Writes happen off the runtime: a full PTY input buffer blocks them.
async fn forward_input<R: AsyncRead + Unpin>(pty: Arc<Pty>, mut read: R) {
    let mut buffer = [0u8; INPUT_BYTES];
    while let Ok(read) = read.read(&mut buffer).await
        && read > 0
    {
        let (pty, bytes) = (Arc::clone(&pty), buffer[..read].to_vec());
        match tokio::task::spawn_blocking(move || pty.write(&bytes)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return report!("terminal input: {error}"),
            Err(error) => return report!("terminal input panicked: {error}"),
        }
    }
}

/// Starts the host out of the server's process group, so a Ctrl-C or group signal aimed at the server spares it, with
/// stderr inherited, so its diagnostics land in the server's log.
#[cfg(not(target_os = "macos"))]
fn start_host(program: &Path, address: &Path) -> std::io::Result<Pid> {
    let mut command = Command::new(program);
    command.args(["server", "terminal-host", "--address"]).arg(address);
    command.stdin(Stdio::null()).stdout(Stdio::null()).process_group(0);
    Ok(Pid::from_raw(command.spawn()?.id().cast_signed()))
}

/// Starts the host out of the server's process group, so a Ctrl-C or group signal aimed at the server spares it, with
/// stderr inherited, so its diagnostics land in the server's log. It answers for its own privacy permissions: macOS asks a process's responsible
/// process, which a child inherits and a process outliving its parent loses. Disclaiming here makes the host responsible
/// from its first instruction, so every terminal's program answers to it, whoever started the server.
#[cfg(target_os = "macos")]
#[allow(unsafe_code, reason = "libSystem's posix_spawn and responsibility calls have no safe binding")]
fn start_host(program: &Path, address: &Path) -> std::io::Result<Pid> {
    use std::ffi::{CString, OsStr, c_int, c_short};
    use std::io::Error as IoError;
    use std::os::unix::ffi::OsStrExt;
    use std::ptr;

    use nix::libc;

    unsafe extern "C" {
        fn responsibility_spawnattrs_setdisclaim(attrs: *mut libc::posix_spawnattr_t, disclaim: c_int) -> c_int;
        fn posix_spawn_file_actions_addinherit_np(actions: *mut libc::posix_spawn_file_actions_t, fd: c_int) -> c_int;
    }

    const FLAGS: c_short = (libc::POSIX_SPAWN_SETPGROUP
        | libc::POSIX_SPAWN_CLOEXEC_DEFAULT
        | libc::POSIX_SPAWN_SETSIGDEF
        | libc::POSIX_SPAWN_SETSIGMASK) as c_short;
    let check = |code: c_int| if code == 0 { Ok(()) } else { Err(IoError::from_raw_os_error(code)) };
    let text = |text: &OsStr| CString::new(text.as_bytes()).map_err(IoError::other);

    let argv =
        [program.as_os_str(), "server".as_ref(), "terminal-host".as_ref(), "--address".as_ref(), address.as_ref()];
    let argv = argv.map(text).into_iter().collect::<std::io::Result<Vec<_>>>()?;
    let env = std::env::vars_os().map(|(key, value)| text(&[key, value].join(OsStr::new("="))));
    let env = env.collect::<std::io::Result<Vec<_>>>()?;
    let pointers = |texts: &[CString]| -> Vec<*mut libc::c_char> {
        texts.iter().map(|text| text.as_ptr().cast_mut()).chain([ptr::null_mut()]).collect()
    };
    let (argv_pointers, env_pointers) = (pointers(&argv), pointers(&env));

    let mut attr: libc::posix_spawnattr_t = ptr::null_mut();
    let mut actions: libc::posix_spawn_file_actions_t = ptr::null_mut();
    let mut pid = 0;
    // SAFETY: every pointer outlives the calls, `argv` and `env` end in null, and destroying an attribute or action set
    // that failed to initialise is a no-op returning EINVAL.
    unsafe {
        let (mut every, mut none) = (std::mem::zeroed(), std::mem::zeroed());
        libc::sigfillset(&mut every);
        libc::sigemptyset(&mut none);
        let spawned = (|| {
            check(libc::posix_spawnattr_init(&mut attr))?;
            check(libc::posix_spawnattr_setflags(&mut attr, FLAGS))?;
            check(libc::posix_spawnattr_setpgroup(&mut attr, 0))?;
            check(libc::posix_spawnattr_setsigdefault(&mut attr, &every))?;
            check(libc::posix_spawnattr_setsigmask(&mut attr, &none))?;
            check(responsibility_spawnattrs_setdisclaim(&mut attr, 1))?;
            check(libc::posix_spawn_file_actions_init(&mut actions))?;
            let null = c"/dev/null".as_ptr();
            check(libc::posix_spawn_file_actions_addopen(&mut actions, libc::STDIN_FILENO, null, libc::O_RDONLY, 0))?;
            check(libc::posix_spawn_file_actions_addopen(&mut actions, libc::STDOUT_FILENO, null, libc::O_WRONLY, 0))?;
            check(posix_spawn_file_actions_addinherit_np(&mut actions, libc::STDERR_FILENO))?;
            check(libc::posix_spawn(
                &mut pid,
                argv[0].as_ptr(),
                &actions,
                &attr,
                argv_pointers.as_ptr(),
                env_pointers.as_ptr(),
            ))
        })();
        libc::posix_spawn_file_actions_destroy(&mut actions);
        libc::posix_spawnattr_destroy(&mut attr);
        spawned?;
    }
    Ok(Pid::from_raw(pid))
}

pub(crate) struct HostClient {
    address: PathBuf,
}

fn host_error(address: &Path, error: impl std::fmt::Display) -> Error {
    Error::internal(format!("terminal host {}: {error}", address.display()))
}

impl HostClient {
    /// A client of the host already answering at `address`.
    #[cfg(any(target_os = "macos", test))]
    pub(crate) fn at(address: &Path) -> Self {
        Self { address: address.to_path_buf() }
    }

    pub(crate) fn reach_or_start(host: &TerminalHost) -> Result<Self, Error> {
        let address = host.address.clone();
        if local::connect_blocking(&address).is_ok() {
            return Ok(Self { address });
        }
        let child =
            start_host(&host.program, &address).map_err(|error| host_error(&address, format!("start: {error}")))?;
        let deadline = Instant::now() + HOST_START;
        while local::connect_blocking(&address).is_err() {
            match waitpid(child, Some(WaitPidFlag::WNOHANG)).map_err(|error| host_error(&address, error))? {
                WaitStatus::StillAlive => {}
                status => return Err(host_error(&address, format!("exited on start with {status:?}"))),
            }
            if Instant::now() > deadline {
                return Err(host_error(&address, format!("not answering {HOST_START:?} after start")));
            }
            std::thread::sleep(HOST_START_POLL);
        }
        std::thread::spawn(move || waitpid(child, None));
        Ok(Self { address })
    }

    fn failed(&self, error: impl std::fmt::Display) -> Error {
        host_error(&self.address, error)
    }

    fn open(&self, request: &HostRequest) -> Result<(BlockingStream, Value), Error> {
        match local::open_blocking(&self.address, request).map_err(|error| self.failed(error))? {
            (stream, Outcome::Ok(value)) => Ok((stream, value)),
            (_, Outcome::Err(error)) => Err(error),
        }
    }

    fn call<T: DeserializeOwned>(&self, request: &HostRequest) -> Result<T, Error> {
        serde_json::from_value(self.open(request)?.1).map_err(|error| self.failed(error))
    }

    /// Reads the protocol before the rest, so a host of another version fails with its version, not its shape.
    pub fn list(&self) -> Result<Vec<Hosted>, Error> {
        let listing = self.open(&HostRequest::List)?.1;
        let protocol = listing.get("protocol").and_then(Value::as_u64);
        if protocol != Some(PROTOCOL.into()) {
            let found = protocol.map_or("no protocol".to_string(), |protocol| format!("protocol {protocol}"));
            return Err(self.failed(format!("speaks {found}, this server {PROTOCOL}; end the host to start over")));
        }
        let listing: Listing = serde_json::from_value(listing).map_err(|error| self.failed(error))?;
        Ok(listing.terminals)
    }

    /// The terminal's screen, then its live pieces until it exits.
    pub async fn attach(&self, id: &PtyId) -> Result<local::Stream, Error> {
        let request = HostRequest::Attach { id: id.clone() };
        match local::open(&self.address, &request).await.map_err(|error| self.failed(error))? {
            (stream, Outcome::Ok(_)) => Ok(stream),
            (_, Outcome::Err(error)) => Err(error),
        }
    }

    /// Whether anything still answers at the host's address.
    pub(crate) fn reachable(&self) -> bool {
        local::connect_blocking(&self.address).is_ok()
    }

    pub fn spawn(&self, id: &PtyId, argv: Vec<String>, cwd: &Path, env: Vec<(String, String)>) -> Result<(), Error> {
        self.call(&HostRequest::Spawn { id: id.clone(), argv, cwd: cwd.into(), env })
    }

    pub fn forget(&self, id: &PtyId) -> Result<(), Error> {
        self.call(&HostRequest::Forget { id: id.clone() })
    }

    pub(crate) fn wait_exit(&self, id: &PtyId) -> Result<(), Error> {
        self.call(&HostRequest::WaitExit { id: id.clone() })
    }

    pub(crate) fn input(&self, id: &PtyId) -> Result<BlockingStream, Error> {
        Ok(self.open(&HostRequest::Input { id: id.clone() })?.0)
    }

    pub(crate) fn resize(&self, id: &PtyId, cols: u16, rows: u16) -> Result<(), Error> {
        self.call(&HostRequest::Resize { id: id.clone(), cols, rows })
    }

    pub(crate) fn activity(&self, id: &PtyId) -> Result<Activity, Error> {
        self.call(&HostRequest::Activity { id: id.clone() })
    }

    pub(crate) fn terminate(&self, id: &PtyId) -> Result<(), Error> {
        self.call(&HostRequest::Terminate { id: id.clone() })
    }

    /// The pid of the program the terminal started, which leads its session, while it runs.
    #[cfg(any(target_os = "macos", test))]
    pub(crate) fn program(&self, id: &PtyId) -> Result<Option<i32>, Error> {
        self.call(&HostRequest::Program { id: id.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn each_terminal_names_the_program_that_leads_its_session() {
        let address = std::env::temp_dir().join(format!("octopus-host-test-{}.sock", std::process::id()));
        tokio::spawn({
            let address = address.clone();
            async move { run_terminal_host(&address).await }
        });
        let client = HostClient::at(&address);
        tokio::task::spawn_blocking(move || {
            let deadline = Instant::now() + HOST_START;
            while local::connect_blocking(&client.address).is_err() {
                assert!(Instant::now() < deadline, "the host never answered");
                std::thread::sleep(HOST_START_POLL);
            }
            let ids = ["pty_a", "pty_b", "pty_c"].map(|id| PtyId(id.into()));
            for id in &ids {
                let argv = vec!["sleep".to_string(), "30".to_string()];
                client.spawn(id, argv, &std::env::temp_dir(), Vec::new()).expect("spawn a terminal");
            }
            for id in &ids {
                let program = client.program(id).expect("ask the host").expect("the program runs");
                let leader = nix::unistd::getsid(Some(nix::unistd::Pid::from_raw(program))).expect("read its session");
                assert_eq!(leader.as_raw(), program);
            }
            for id in &ids {
                client.forget(id).expect("end the terminal");
            }
        })
        .await
        .expect("the host answers");
        std::fs::remove_file(&address).expect("remove the host's socket");
    }
}
