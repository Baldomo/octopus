use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nix::unistd::Pid;
use octopus_proto::Error;
use octopus_proto::types::TERM_SIZE_OSC;
use portable_pty::{CommandBuilder, MasterPty, PtyPair, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};

use crate::{lock, now, report};

const SCROLLBACK_BYTES: usize = 1 << 20;
const LIVE_CHUNKS: usize = 1024;
const READ_BYTES: usize = 8192;
const KILL_GRACE: Duration = Duration::from_secs(5);
/// Every terminal starts at this size, and a restarted server puts every terminal it adopts back to it.
pub const INITIAL_SIZE: (u16, u16) = (80, 24);

pub type Chunk = Arc<[u8]>;

/// Output the program wrote at one size.
struct Stretch {
    cols: u16,
    rows: u16,
    bytes: VecDeque<u8>,
}

fn size_record(cols: u16, rows: u16) -> Vec<u8> {
    format!("\x1b]{TERM_SIZE_OSC};{cols};{rows}\x07").into_bytes()
}

/// Where the output stands between the escape sequences and UTF-8 characters a terminal parses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grammar {
    Ground,
    Escape,
    Csi,
    /// An OSC's string, which BEL or ST ends.
    Osc,
    /// A DCS, SOS, PM or APC string, which ST alone ends.
    Str,
    StrEscape,
    Utf8 {
        left: u8,
    },
}

impl Grammar {
    fn after(self, byte: u8) -> Self {
        match (self, byte) {
            (_, 0x18 | 0x1a) | (Self::Osc, 0x07) | (Self::StrEscape, b'\\') => Self::Ground,
            (Self::Osc | Self::Str, 0x1b) => Self::StrEscape,
            (Self::Osc | Self::Str, _) => self,
            (Self::StrEscape, _) => Self::Escape.after(byte),
            (_, 0x1b) => Self::Escape,
            (Self::Escape, b'[') => Self::Csi,
            (Self::Escape, b']') => Self::Osc,
            (Self::Escape, b'P' | b'X' | b'^' | b'_') => Self::Str,
            (Self::Escape, 0x00..=0x2f) => Self::Escape,
            (Self::Escape, 0x80..) => Self::Ground.after(byte),
            (Self::Csi, 0x40..=0x7e) | (Self::Escape, _) => Self::Ground,
            (Self::Csi, _) => Self::Csi,
            (Self::Utf8 { left }, 0x80..=0xbf) => {
                if left == 1 {
                    Self::Ground
                } else {
                    Self::Utf8 { left: left - 1 }
                }
            }
            (Self::Ground | Self::Utf8 { .. }, 0xc0..=0xdf) => Self::Utf8 { left: 1 },
            (Self::Ground | Self::Utf8 { .. }, 0xe0..=0xef) => Self::Utf8 { left: 2 },
            (Self::Ground | Self::Utf8 { .. }, 0xf0..=0xf7) => Self::Utf8 { left: 3 },
            (Self::Ground | Self::Utf8 { .. }, _) => Self::Ground,
        }
    }
}

struct Output {
    /// The most recent output, oldest first, split where the terminal resized; never empty.
    scrollback: VecDeque<Stretch>,
    kept_bytes: usize,
    grammar: Grammar,
    /// A resize that came while the output stood inside a sequence or a character, which its size record would split.
    pending_size: Option<(u16, u16)>,
    live: Option<broadcast::Sender<Chunk>>,
}

impl Output {
    fn new(live: Option<broadcast::Sender<Chunk>>) -> Self {
        let (cols, rows) = INITIAL_SIZE;
        let scrollback = VecDeque::from([Stretch { cols, rows, bytes: VecDeque::new() }]);
        Self { scrollback, kept_bytes: 0, grammar: Grammar::Ground, pending_size: None, live }
    }

    /// A resize waiting on a sequence lands right after the byte that ends it.
    fn add(&mut self, chunk: &[u8]) {
        let mut rest = chunk;
        if let Some((cols, rows)) = self.pending_size {
            let Some(end) = rest.iter().position(|&byte| {
                self.grammar = self.grammar.after(byte);
                self.grammar == Grammar::Ground
            }) else {
                return self.keep(rest);
            };
            let (whole, after) = rest.split_at(end + 1);
            self.keep(whole);
            self.pending_size = None;
            self.record(cols, rows);
            rest = after;
        }
        self.grammar = rest.iter().fold(self.grammar, |grammar, &byte| grammar.after(byte));
        self.keep(rest);
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        if self.grammar == Grammar::Ground {
            self.record(cols, rows);
        } else {
            self.pending_size = Some((cols, rows));
        }
    }

    fn keep(&mut self, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }
        self.scrollback.back_mut().expect("the scrollback is never empty").bytes.extend(chunk);
        self.kept_bytes += chunk.len();
        while self.kept_bytes > SCROLLBACK_BYTES {
            let oldest = self.scrollback.front_mut().expect("the scrollback is never empty");
            let cut = (self.kept_bytes - SCROLLBACK_BYTES).min(oldest.bytes.len());
            oldest.bytes.drain(..cut);
            self.kept_bytes -= cut;
            if oldest.bytes.is_empty() && self.scrollback.len() > 1 {
                self.scrollback.pop_front();
            }
        }
        self.send(chunk);
    }

    /// A resize with no output since the last one replaces it.
    fn record(&mut self, cols: u16, rows: u16) {
        let last = self.scrollback.back_mut().expect("the scrollback is never empty");
        if (last.cols, last.rows) == (cols, rows) {
            return;
        }
        if last.bytes.is_empty() {
            (last.cols, last.rows) = (cols, rows);
        } else {
            self.scrollback.push_back(Stretch { cols, rows, bytes: VecDeque::new() });
        }
        self.send(&size_record(cols, rows));
    }

    fn send(&self, bytes: &[u8]) {
        if let Some(live) = &self.live {
            let _no_receivers = live.send(Chunk::from(bytes));
        }
    }

    fn replay(&self) -> Vec<u8> {
        let stretch =
            |stretch: &Stretch| size_record(stretch.cols, stretch.rows).into_iter().chain(stretch.bytes.clone());
        self.scrollback.iter().flat_map(stretch).collect()
    }
}

pub struct Attachment {
    pub scrollback: Vec<u8>,
    /// `None` once the terminal's output has ended.
    pub live: Option<broadcast::Receiver<Chunk>>,
}

/// A PTY and its process, owned by the terminal host.
pub struct Pty {
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    /// Leads its own session and group, so signalling the group reaches everything it started. Dropped right after
    /// the reap, narrowing the window for a signal to reach a process that reuses the id.
    process: Mutex<Option<Pid>>,
    output: Mutex<Output>,
    /// When the terminal last wrote output, or started.
    output_at: AtomicI64,
    exited: watch::Sender<bool>,
}

/// What tells whether a terminal's program is still at work.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Activity {
    pub output_at: i64,
    /// Whether a process descending from the program runs outside the program's process group, as a shell an agent
    /// starts for a command does; the processes of the agent itself, such as its MCP servers, stay in its group.
    pub detached_descendant: bool,
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }
}

impl Pty {
    pub fn spawn(argv: &[String], cwd: &Path, env: &[(String, String)]) -> Result<Arc<Self>, Error> {
        let pair = native_pty_system().openpty(size(INITIAL_SIZE.0, INITIAL_SIZE.1)).map_err(Error::internal)?;
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
            process: Mutex::new(Some(child)),
            output: Mutex::new(Output::new(Some(live))),
            output_at: AtomicI64::new(now()),
            exited: watch::Sender::new(false),
        });
        let (pump, reaper) = (Arc::clone(&pty), Arc::clone(&pty));
        std::thread::spawn(move || pump.pump(reader));
        std::thread::spawn(move || reaper.reap(child));
        Ok(pty)
    }

    fn pump(&self, mut reader: Box<dyn Read + Send>) {
        let mut buffer = [0u8; READ_BYTES];
        while let Ok(read) = reader.read(&mut buffer) {
            if read == 0 {
                break;
            }
            self.output_at.store(now(), Ordering::Relaxed);
            lock(&self.output).add(&buffer[..read]);
        }
        lock(&self.output).live = None;
    }

    fn reap(&self, child: Pid) {
        if let Err(error) = wait(child) {
            report!("waiting on a terminal's process: {error}");
        }
        lock(&self.process).take();
        lock(&self.master).take();
        self.exited.send_replace(true);
    }

    pub fn attach(&self) -> Attachment {
        let output = lock(&self.output);
        Attachment { scrollback: output.replay(), live: output.live.as_ref().map(broadcast::Sender::subscribe) }
    }

    pub fn running(&self) -> bool {
        !*self.exited.borrow()
    }

    /// Reads the process table, so it blocks.
    pub fn activity(&self) -> Result<Activity, Error> {
        let program = (*lock(&self.process)).ok_or_else(|| Error::conflict("the terminal has exited"))?;
        let table = std::process::Command::new("ps")
            .args(["-A", "-o", "pid=,ppid=,pgid="])
            .output()
            .map_err(|error| Error::internal(format!("ps: {error}")))?;
        if !table.status.success() {
            return Err(Error::internal(format!("ps: {}", table.status)));
        }
        let mut children: HashMap<i32, Vec<(i32, i32)>> = HashMap::new();
        for line in String::from_utf8_lossy(&table.stdout).lines() {
            if let [Ok(pid), Ok(parent), Ok(group)] =
                line.split_whitespace().map(str::parse::<i32>).collect::<Vec<_>>()[..]
            {
                children.entry(parent).or_default().push((pid, group));
            }
        }
        let mut pending = vec![program.as_raw()];
        let mut detached_descendant = false;
        while let Some(parent) = pending.pop()
            && !detached_descendant
        {
            for &(pid, group) in children.get(&parent).into_iter().flatten() {
                detached_descendant |= group != program.as_raw();
                pending.push(pid);
            }
        }
        Ok(Activity { output_at: self.output_at.load(Ordering::Relaxed), detached_descendant })
    }

    pub async fn exit(&self) {
        let mut exited = self.exited.subscribe();
        let _sender_lives_in_self = exited.wait_for(|exited| *exited).await.map(drop);
    }

    pub fn write(&self, bytes: &[u8]) -> Result<(), Error> {
        let mut writer = lock(&self.writer);
        writer.write_all(bytes).and_then(|()| writer.flush()).map_err(Error::internal)
    }

    /// Holds the output while it resizes, so whatever the program redraws at the new size follows the size's record.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), Error> {
        let mut output = lock(&self.output);
        match lock(&self.master).as_ref() {
            Some(master) => master.resize(size(cols, rows)).map_err(Error::internal)?,
            None => return Err(Error::conflict("the terminal has exited")),
        }
        output.resize(cols, rows);
        Ok(())
    }

    pub async fn terminate(self: Arc<Self>) {
        self.signal(nix::sys::signal::Signal::SIGTERM);
        if tokio::time::timeout(KILL_GRACE, self.exit()).await.is_err() {
            self.kill();
        }
    }

    fn signal(&self, signal: nix::sys::signal::Signal) {
        if let Some(pid) = *lock(&self.process)
            && let Err(error) = nix::sys::signal::killpg(pid, signal)
        {
            report!("send {signal} to {pid}: {error}");
        }
    }

    pub fn kill(&self) {
        self.signal(nix::sys::signal::Signal::SIGKILL);
    }
}

#[cfg(target_os = "linux")]
fn start(pair: &PtyPair, command: CommandBuilder) -> anyhow::Result<Pid> {
    let pid = pair.slave.spawn_command(command)?.process_id().expect("a spawned unix child has a pid");
    Ok(Pid::from_raw(i32::try_from(pid)?))
}

#[cfg(target_os = "macos")]
fn start(pair: &PtyPair, command: CommandBuilder) -> anyhow::Result<Pid> {
    let tty = pair.master.tty_name().expect("a macOS PTY has a device path");
    disclaimed::spawn(command, &tty)
}

fn wait(pid: Pid) -> nix::Result<()> {
    nix::sys::wait::waitpid(pid, None).map(drop)
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
        fn posix_spawn_file_actions_addchdir_np(
            actions: *mut libc::posix_spawn_file_actions_t,
            path: *const libc::c_char,
        ) -> c_int;
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
                check("enter the directory", posix_spawn_file_actions_addchdir_np(&mut actions, cwd.as_ptr()))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replay_draws_each_stretch_at_its_size_and_trimming_keeps_the_size_it_cuts_into() {
        let mut output = Output::new(None);
        output.resize(100, 30);
        output.add(b"wide");
        output.resize(40, 20);
        output.resize(46, 28);
        output.add(b"narrow");
        let narrow = [size_record(46, 28), b"narrow".to_vec()].concat();
        assert_eq!(output.replay(), [size_record(100, 30), b"wide".to_vec(), narrow.clone()].concat());

        output.add(&vec![b'.'; SCROLLBACK_BYTES - "wide".len() - "narrow".len() + 2]);
        assert!(output.replay().starts_with(&[size_record(100, 30), b"de".to_vec(), narrow].concat()));
        output.add(b"..");
        assert!(output.replay().starts_with(&size_record(46, 28)));
    }

    #[test]
    fn a_resize_inside_a_sequence_or_a_character_records_after_it_ends() {
        let mut output = Output::new(None);
        output.add(b"\x1b[38;5");
        output.resize(46, 28);
        output.add(b";245mred \xe2\x94");
        output.resize(50, 30);
        output.add(b"\x80 box");
        let expected =
            [size_record(80, 24), b"\x1b[38;5;245m".to_vec(), size_record(46, 28), b"red \xe2\x94\x80".to_vec()];
        assert_eq!(output.replay(), [expected.concat(), size_record(50, 30), b" box".to_vec()].concat());
    }
}
