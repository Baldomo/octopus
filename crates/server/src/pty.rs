use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nix::unistd::Pid;
use octopus_proto::Error;
use octopus_proto::term::{Piece, Screen};
use portable_pty::{CommandBuilder, MasterPty, PtyPair, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};

use crate::emulator::Emulator;
use crate::{lock, now, report};

const LIVE_PIECES: usize = 1024;
const READ_BYTES: usize = 8192;
const KILL_GRACE: Duration = Duration::from_secs(5);
/// Every terminal starts at this size, and a restarted server puts every terminal it adopts back to it.
pub(crate) const INITIAL_SIZE: (u16, u16) = (80, 24);

/// The terminal's screen and the pieces it sends each attached client, kept in step under one lock.
struct Output {
    emulator: Emulator,
    /// `None` once the program's output has ended.
    live: Option<broadcast::Sender<Piece>>,
}

impl Output {
    fn new(live: Option<broadcast::Sender<Piece>>) -> Self {
        Self { emulator: Emulator::new(INITIAL_SIZE.0, INITIAL_SIZE.1), live }
    }

    fn add(&mut self, bytes: &[u8]) {
        self.emulator.advance(bytes);
        self.send(Piece::Output(bytes.into()));
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.emulator.resize(cols, rows);
        let screen = self.emulator.screen();
        self.send(Piece::Screen(screen));
    }

    fn send(&self, piece: Piece) {
        if let Some(live) = &self.live {
            let _no_receivers = live.send(piece);
        }
    }
}

pub(crate) struct Attachment {
    pub screen: Screen,
    /// `None` once the terminal's output has ended.
    pub live: Option<broadcast::Receiver<Piece>>,
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
pub(crate) struct Activity {
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
        let (live, _) = broadcast::channel(LIVE_PIECES);
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
        let mut output = lock(&self.output);
        Attachment { screen: output.emulator.screen(), live: output.live.as_ref().map(broadcast::Sender::subscribe) }
    }

    pub fn running(&self) -> bool {
        !*self.exited.borrow()
    }

    /// Reads the process table, so it blocks.
    pub(crate) fn activity(&self) -> Result<Activity, Error> {
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

    /// Holds the output while it resizes, so whatever the program redraws at the new size follows the new screen.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), Error> {
        let mut output = lock(&self.output);
        match lock(&self.master).as_ref() {
            Some(master) => master.resize(size(cols, rows)).map_err(Error::internal)?,
            None => return Err(Error::conflict("the terminal has exited")),
        }
        output.resize(cols, rows);
        Ok(())
    }

    pub(crate) async fn terminate(self: Arc<Self>) {
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
    use alacritty_terminal::Term;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line};
    use alacritty_terminal::term::cell::Flags;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::term::{Config, TermMode};
    use alacritty_terminal::vte::ansi::Processor;

    use super::*;

    /// A client: a terminal that reads the pieces as a client does, so it shows what that client draws.
    struct Client {
        term: Term<VoidListener>,
        parser: Processor,
    }

    impl Client {
        fn new() -> Self {
            Self { term: Term::new(Config::default(), &TermSize::new(1, 1), VoidListener), parser: Processor::new() }
        }

        fn read(&mut self, piece: &Piece) {
            match piece {
                Piece::Screen(Screen { cols, rows, bytes }) => {
                    *self = Self::new();
                    self.term.resize(TermSize::new((*cols).into(), (*rows).into()));
                    self.parser.advance(&mut self.term, bytes);
                }
                Piece::Output(bytes) => self.parser.advance(&mut self.term, bytes),
            }
        }

        /// Every line from the oldest, trailing blanks trimmed.
        fn lines(&self) -> Vec<String> {
            let grid = self.term.grid();
            (grid.topmost_line().0..=grid.bottommost_line().0)
                .map(|line| {
                    let row = &grid[Line(line)];
                    (0..grid.columns()).map(|column| row[Column(column)].c).collect::<String>().trim_end().to_string()
                })
                .collect()
        }

        fn cursor(&self) -> (i32, usize) {
            let point = self.term.grid().cursor.point;
            (point.line.0, point.column.0)
        }
    }

    fn output() -> Output {
        Output::new(None)
    }

    fn attach(output: &mut Output) -> Client {
        let mut client = Client::new();
        client.read(&Piece::Screen(output.emulator.screen()));
        client
    }

    #[test]
    fn output_written_wide_reflows_into_the_screen_a_narrower_attach_draws() {
        let mut output = output();
        output.resize(110, 30);
        let long = "0123456789".repeat(10);
        output.add(format!("\x1b[1;31m{long}\x1b[0m\r\nnext\r\n\x1b[?1h$ ").as_bytes());
        output.resize(47, 12);

        let client = attach(&mut output);
        let Screen { cols, rows, .. } = output.emulator.screen();
        assert_eq!((cols, rows), (47, 12));
        let lines = client.lines();
        let drawn = lines.iter().position(|line| line.starts_with("0123")).expect("the long line");
        assert_eq!(lines[drawn..drawn + 5], [&long[..47], &long[47..94], &long[94..], "next", "$"]);
        let grid = client.term.grid();
        let wrapped = &grid[Line(i32::try_from(drawn + 1).unwrap() - i32::try_from(grid.history_size()).unwrap())];
        assert!(wrapped[Column(0)].flags.contains(Flags::BOLD), "attributes survive the reflow");
        assert_eq!(client.cursor().1, 2);
        assert!(client.term.mode().contains(TermMode::APP_CURSOR));
    }

    #[test]
    fn a_program_printing_a_size_record_of_its_own_changes_nothing() {
        let mut output = output();
        output.add(b"\x1b]6286;10;5\x07plain");
        let Screen { cols, rows, bytes } = output.emulator.screen();
        assert_eq!((cols, rows), INITIAL_SIZE);
        assert!(!String::from_utf8_lossy(&bytes).contains("6286"));
        assert_eq!(attach(&mut output).lines()[0], "plain");
    }

    #[test]
    fn a_character_split_across_an_attach_arrives_whole() {
        let mut output = output();
        output.add("a\x1b[38;5".as_bytes());
        output.add(b";245mb\xe2\x94");
        let mut client = attach(&mut output);
        let rest = b"\x80c";
        output.add(rest);
        client.read(&Piece::Output(rest[..].into()));
        assert_eq!(client.lines()[0], "ab─c");
        assert_eq!(attach(&mut output).lines()[0], "ab─c");
    }

    #[test]
    fn a_cursor_saved_before_an_attach_restores_after_it() {
        let mut output = output();
        output.add(b"\x1b[3;5H\x1b7\x1b[H");
        let mut client = attach(&mut output);
        assert_eq!(client.cursor(), (0, 0));
        client.read(&Piece::Output(b"\x1b8"[..].into()));
        assert_eq!(client.cursor(), (2, 4));
    }

    #[test]
    fn a_full_screen_program_redraws_over_the_shell_it_returns_to() {
        let mut output = output();
        output.add(b"$ vim\r\n\x1b[?1049h\x1b[Hfile\x1b[24;1H-- INSERT --\x1b[1;5H");
        let mut client = attach(&mut output);
        assert!(client.term.mode().contains(TermMode::ALT_SCREEN));
        assert_eq!(client.lines()[0], "file");
        assert_eq!(client.lines()[23], "-- INSERT --");
        assert_eq!(client.cursor(), (0, 4));
        let leave = b"\x1b[?1049l";
        output.add(leave);
        client.read(&Piece::Output(leave[..].into()));
        assert_eq!(client.lines()[0], "$ vim");
        assert_eq!(client.cursor(), (1, 0));
        assert_eq!(attach(&mut output).lines()[..2], ["$ vim", ""]);
    }
}
