use std::borrow::Cow;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use octopus_proto::Error;
#[cfg(windows)]
use portable_pty::Child;
use portable_pty::{CommandBuilder, MasterPty, PtyPair, PtySize, native_pty_system};
use tokio::sync::{broadcast, watch};

use crate::{lock, report};

const SCROLLBACK_BYTES: usize = 1 << 20;
const LIVE_CHUNKS: usize = 1024;
const READ_BYTES: usize = 8192;
const KILL_GRACE: Duration = Duration::from_secs(5);
const INITIAL_COLS: u16 = 80;
const INITIAL_ROWS: u16 = 24;

pub type Chunk = Arc<[u8]>;

struct Output {
    scrollback: VecDeque<u8>,
    live: Option<broadcast::Sender<Chunk>>,
}

pub struct Attachment {
    pub scrollback: Vec<u8>,
    /// `None` once the terminal's output has ended.
    pub live: Option<broadcast::Receiver<Chunk>>,
}

struct Process {
    /// The process leads its own session and group, so signalling the group reaches everything it started.
    #[cfg(unix)]
    pid: nix::unistd::Pid,
    #[cfg(windows)]
    killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
}

/// A PTY and its process, owned by the terminal host.
pub struct Pty {
    writer: Mutex<Box<dyn Write + Send>>,
    /// Dropped once the process exits: a Windows pseudoconsole ends its output only when closed.
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    /// Dropped right after the reap, narrowing the window for a signal to reach a process that reuses the id.
    process: Mutex<Option<Process>>,
    output: Mutex<Output>,
    exited: watch::Sender<bool>,
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }
}

impl Pty {
    pub fn spawn(argv: &[String], cwd: &Path, env: &[(String, String)]) -> Result<Arc<Self>, Error> {
        let pair = native_pty_system().openpty(size(INITIAL_COLS, INITIAL_ROWS)).map_err(Error::internal)?;
        let mut command = CommandBuilder::from_argv(argv.iter().map(Into::into).collect());
        command.cwd(cwd);
        for (key, value) in env {
            command.env(key, value);
        }
        let child = start(&pair, command).map_err(Error::internal)?;
        drop(pair.slave);
        let reader = pair.master.try_clone_reader().map_err(Error::internal)?;
        let writer = pair.master.take_writer().map_err(Error::internal)?;
        let (live, _) = broadcast::channel(LIVE_CHUNKS);
        let pty = Arc::new(Self {
            writer: Mutex::new(writer),
            master: Mutex::new(Some(pair.master)),
            process: Mutex::new(Some(Process::of(&child))),
            output: Mutex::new(Output { scrollback: VecDeque::new(), live: Some(live) }),
            exited: watch::Sender::new(false),
        });
        let (pump, reaper) = (Arc::clone(&pty), Arc::clone(&pty));
        std::thread::spawn(move || pump.pump(reader));
        std::thread::spawn(move || reaper.reap(child));
        Ok(pty)
    }

    fn pump(&self, mut reader: Box<dyn Read + Send>) {
        let mut buffer = [0u8; READ_BYTES];
        let mut cursor_query_answered = false;
        while let Ok(read) = reader.read(&mut buffer) {
            if read == 0 {
                break;
            }
            let chunk = self.answer_cursor_query(&buffer[..read], &mut cursor_query_answered);
            let mut output = lock(&self.output);
            output.scrollback.extend(chunk.iter());
            let excess = output.scrollback.len().saturating_sub(SCROLLBACK_BYTES);
            output.scrollback.drain(..excess);
            if let Some(live) = &output.live {
                let _no_receivers = live.send(Chunk::from(chunk));
            }
        }
        lock(&self.output).live = None;
    }

    /// A pseudoconsole asks its terminal for the cursor position and renders nothing until answered. The host is
    /// that terminal whether or not a client is attached, and a fresh screen starts at the top left. The query is
    /// kept out of the output, so no client attaching later answers it again.
    #[cfg(windows)]
    fn answer_cursor_query<'chunk>(&self, chunk: &'chunk [u8], answered: &mut bool) -> Cow<'chunk, [u8]> {
        const QUERY: &[u8] = b"\x1b[6n";
        const AT_TOP_LEFT: &[u8] = b"\x1b[1;1R";
        let found = (!*answered).then(|| chunk.windows(QUERY.len()).position(|window| window == QUERY)).flatten();
        let Some(at) = found else { return Cow::Borrowed(chunk) };
        *answered = true;
        if let Err(error) = self.write(AT_TOP_LEFT) {
            report!("answer a terminal's cursor query: {error}");
        }
        Cow::Owned([&chunk[..at], &chunk[at + QUERY.len()..]].concat())
    }

    /// A Unix PTY asks nothing of its terminal before it runs.
    #[cfg(unix)]
    fn answer_cursor_query<'chunk>(&self, chunk: &'chunk [u8], _answered: &mut bool) -> Cow<'chunk, [u8]> {
        Cow::Borrowed(chunk)
    }

    fn reap(&self, child: Started) {
        if let Err(error) = wait(child) {
            report!("waiting on a terminal's process: {error}");
        }
        lock(&self.process).take();
        lock(&self.master).take();
        self.exited.send_replace(true);
    }

    pub fn attach(&self) -> Attachment {
        let output = lock(&self.output);
        Attachment {
            scrollback: output.scrollback.iter().copied().collect(),
            live: output.live.as_ref().map(broadcast::Sender::subscribe),
        }
    }

    pub fn running(&self) -> bool {
        !*self.exited.borrow()
    }

    pub async fn exit(&self) {
        let mut exited = self.exited.subscribe();
        let _sender_lives_in_self = exited.wait_for(|exited| *exited).await.map(drop);
    }

    pub fn write(&self, bytes: &[u8]) -> Result<(), Error> {
        let mut writer = lock(&self.writer);
        writer.write_all(bytes).and_then(|()| writer.flush()).map_err(Error::internal)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), Error> {
        match lock(&self.master).as_ref() {
            Some(master) => master.resize(size(cols, rows)).map_err(Error::internal),
            None => Err(Error::conflict("the terminal has exited")),
        }
    }

    /// The grace starts as the ask leaves: on Windows the ask is a write, which a full input buffer blocks.
    pub async fn terminate(self: Arc<Self>) {
        let asking = Arc::clone(&self);
        drop(tokio::task::spawn_blocking(move || asking.ask_to_exit()));
        if tokio::time::timeout(KILL_GRACE, self.exit()).await.is_err() {
            self.kill();
        }
    }

    #[cfg(unix)]
    fn signal(&self, signal: nix::sys::signal::Signal) {
        if let Some(process) = lock(&self.process).as_ref()
            && let Err(error) = nix::sys::signal::killpg(process.pid, signal)
        {
            report!("send {signal} to {}: {error}", process.pid);
        }
    }

    #[cfg(unix)]
    fn ask_to_exit(&self) {
        self.signal(nix::sys::signal::Signal::SIGTERM);
    }

    #[cfg(unix)]
    pub fn kill(&self) {
        self.signal(nix::sys::signal::Signal::SIGKILL);
    }

    /// Ctrl-C typed into the pseudoconsole: Windows' console processes take it as their interrupt.
    #[cfg(windows)]
    fn ask_to_exit(&self) {
        const CTRL_C: u8 = 0x03;
        if self.running()
            && let Err(error) = self.write(&[CTRL_C])
        {
            report!("interrupt a terminal: {error}");
        }
    }

    #[cfg(windows)]
    pub fn kill(&self) {
        if let Some(process) = lock(&self.process).as_mut()
            && let Err(error) = process.killer.kill()
        {
            report!("kill a terminal: {error}");
        }
    }
}

#[cfg(unix)]
type Started = nix::unistd::Pid;
#[cfg(windows)]
type Started = Box<dyn Child + Send + Sync>;

#[cfg(all(unix, not(target_os = "macos")))]
fn start(pair: &PtyPair, command: CommandBuilder) -> anyhow::Result<Started> {
    let pid = pair.slave.spawn_command(command)?.process_id().expect("a spawned unix child has a pid");
    Ok(nix::unistd::Pid::from_raw(i32::try_from(pid)?))
}

#[cfg(target_os = "macos")]
fn start(pair: &PtyPair, command: CommandBuilder) -> anyhow::Result<Started> {
    let tty = pair.master.tty_name().expect("a macOS PTY has a device path");
    disclaimed::spawn(command, &tty)
}

#[cfg(windows)]
fn start(pair: &PtyPair, command: CommandBuilder) -> anyhow::Result<Started> {
    pair.slave.spawn_command(command)
}

#[cfg(unix)]
fn wait(pid: Started) -> nix::Result<()> {
    nix::sys::wait::waitpid(pid, None).map(drop)
}

#[cfg(windows)]
fn wait(mut child: Started) -> std::io::Result<()> {
    child.wait().map(drop)
}

impl Process {
    #[cfg(unix)]
    fn of(pid: &Started) -> Self {
        Self { pid: *pid }
    }

    #[cfg(windows)]
    fn of(child: &Started) -> Self {
        Self { killer: child.clone_killer() }
    }
}

/// macOS asks a process's responsible process for its privacy permissions, and a child inherits its parent's.
/// A terminal's program disclaims the host, so it answers for itself. A process takes a controlling terminal only
/// through its own ioctl, so a fork takes the PTY, then becomes the program through a disclaiming posix_spawn.
#[cfg(target_os = "macos")]
#[allow(unsafe_code, reason = "libSystem's fork, ioctl, posix_spawn and responsibility calls have no safe binding")]
mod disclaimed {
    use std::ffi::{CString, OsStr, c_int, c_short};
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr;

    use anyhow::Context;
    use nix::libc;
    use nix::sys::wait::WaitStatus;
    use nix::unistd::Pid;
    use portable_pty::CommandBuilder;

    const FLAGS: c_short = (libc::POSIX_SPAWN_SETEXEC
        | libc::POSIX_SPAWN_CLOEXEC_DEFAULT
        | libc::POSIX_SPAWN_SETSIGDEF
        | libc::POSIX_SPAWN_SETSIGMASK) as c_short;
    const STDIO: [c_int; 3] = [libc::STDIN_FILENO, libc::STDOUT_FILENO, libc::STDERR_FILENO];
    /// The fork's exit status names the step that failed.
    const TAKE_TERMINAL: c_int = 1;
    const START: c_int = 2;

    unsafe extern "C" {
        fn responsibility_spawnattrs_setdisclaim(attrs: *mut libc::posix_spawnattr_t, disclaim: c_int) -> c_int;
        fn posix_spawn_file_actions_addinherit_np(actions: *mut libc::posix_spawn_file_actions_t, fd: c_int) -> c_int;
    }

    fn c_string(text: impl AsRef<OsStr>) -> anyhow::Result<CString> {
        Ok(CString::new(text.as_ref().as_bytes())?)
    }

    fn check(what: &'static str, code: c_int) -> anyhow::Result<()> {
        match code {
            0 => Ok(()),
            errno => Err(std::io::Error::from_raw_os_error(errno)).context(what),
        }
    }

    /// Spawns `command` as the leader of a new session whose controlling terminal is the PTY at `tty`, so the
    /// program leads the terminal's foreground group.
    pub fn spawn(mut command: CommandBuilder, tty: &Path) -> anyhow::Result<Pid> {
        command.env("SHELL", command.get_shell());
        let argv = command.get_argv();
        let cwd = command.get_cwd().expect("a terminal has a working directory");
        let program = which::which_in(&argv[0], command.get_env("PATH"), cwd);
        let program = c_string(program.with_context(|| format!("find {:?}", argv[0]))?)?;
        let (tty, cwd) = (c_string(tty)?, c_string(cwd)?);
        let argv = argv.iter().map(c_string).collect::<anyhow::Result<Vec<_>>>()?;
        let env = command
            .iter_full_env_as_str()
            .map(|(key, value)| c_string(format!("{key}={value}")))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let argv: Vec<_> = argv.iter().map(|arg| arg.as_ptr().cast_mut()).chain([ptr::null_mut()]).collect();
        let env: Vec<_> = env.iter().map(|pair| pair.as_ptr().cast_mut()).chain([ptr::null_mut()]).collect();

        let mut attr: libc::posix_spawnattr_t = ptr::null_mut();
        let mut actions: libc::posix_spawn_file_actions_t = ptr::null_mut();
        // SAFETY: every pointer outlives the calls, `argv` and `env` end in null, the fork allocates nothing and
        // calls only syscall wrappers (libSystem's posix_spawn copies the prepared attributes into one syscall), and
        // destroying an attribute or action set that failed to initialise is a no-op returning EINVAL.
        unsafe {
            let (mut every, mut none) = (std::mem::zeroed(), std::mem::zeroed());
            libc::sigfillset(&mut every);
            libc::sigemptyset(&mut none);
            let spawned = (|| {
                check("init spawn attributes", libc::posix_spawnattr_init(&mut attr))?;
                check("set spawn flags", libc::posix_spawnattr_setflags(&mut attr, FLAGS))?;
                check("default signals", libc::posix_spawnattr_setsigdefault(&mut attr, &every))?;
                check("unblock signals", libc::posix_spawnattr_setsigmask(&mut attr, &none))?;
                check("disclaim responsibility", responsibility_spawnattrs_setdisclaim(&mut attr, 1))?;
                check("init file actions", libc::posix_spawn_file_actions_init(&mut actions))?;
                for stdio in STDIO {
                    check("keep the terminal", posix_spawn_file_actions_addinherit_np(&mut actions, stdio))?;
                }
                check("enter the directory", libc::posix_spawn_file_actions_addchdir_np(&mut actions, cwd.as_ptr()))?;
                let (mut reader, writer) = std::io::pipe().context("open a pipe for the start's failure")?;
                match libc::fork() {
                    -1 => Err(std::io::Error::last_os_error()).context("fork"),
                    0 => {
                        libc::setsid();
                        let terminal = libc::open(tty.as_ptr(), libc::O_RDWR);
                        let (errno, step) = if terminal != -1
                            && libc::ioctl(terminal, libc::TIOCSCTTY.into(), 0) != -1
                            && STDIO.into_iter().all(|stdio| libc::dup2(terminal, stdio) != -1)
                        {
                            let argv = argv.as_ptr();
                            (libc::posix_spawn(&mut 0, program.as_ptr(), &actions, &attr, argv, env.as_ptr()), START)
                        } else {
                            (*libc::__error(), TAKE_TERMINAL)
                        };
                        libc::write(writer.as_raw_fd(), ptr::from_ref(&errno).cast(), size_of_val(&errno));
                        libc::_exit(step)
                    }
                    pid => {
                        drop(writer);
                        let pid = Pid::from_raw(pid);
                        let mut errno = Vec::new();
                        reader.read_to_end(&mut errno).context("read the start's failure")?;
                        if errno.is_empty() {
                            return Ok(pid);
                        }
                        let errno = c_int::from_ne_bytes(errno.try_into().expect("the fork reports one errno"));
                        let error = std::io::Error::from_raw_os_error(errno);
                        match nix::sys::wait::waitpid(pid, None) {
                            Ok(WaitStatus::Exited(_, TAKE_TERMINAL)) => {
                                Err(anyhow::anyhow!("take the terminal {}: {error}", tty.to_string_lossy()))
                            }
                            _ => Err(anyhow::anyhow!("start {}: {error}", program.to_string_lossy())),
                        }
                    }
                }
            })();
            libc::posix_spawn_file_actions_destroy(&mut actions);
            libc::posix_spawnattr_destroy(&mut attr);
            spawned
        }
    }
}
