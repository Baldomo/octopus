use std::borrow::Cow;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use octopus_proto::Error;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
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
        command.env("TERM", "xterm-256color");
        for (key, value) in env {
            command.env(key, value);
        }
        let child = pair.slave.spawn_command(command).map_err(Error::internal)?;
        drop(pair.slave);
        let reader = pair.master.try_clone_reader().map_err(Error::internal)?;
        let writer = pair.master.take_writer().map_err(Error::internal)?;
        let (live, _) = broadcast::channel(LIVE_CHUNKS);
        let pty = Arc::new(Self {
            writer: Mutex::new(writer),
            master: Mutex::new(Some(pair.master)),
            process: Mutex::new(Some(Process::of(child.as_ref()))),
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

    fn reap(&self, mut child: Box<dyn Child + Send + Sync>) {
        if let Err(error) = child.wait() {
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

impl Process {
    #[cfg(unix)]
    fn of(child: &(dyn Child + Send + Sync)) -> Self {
        let pid = child.process_id().and_then(|pid| i32::try_from(pid).ok()).expect("a spawned unix child has a pid");
        Self { pid: nix::unistd::Pid::from_raw(pid) }
    }

    #[cfg(windows)]
    fn of(child: &(dyn Child + Send + Sync)) -> Self {
        Self { killer: child.clone_killer() }
    }
}
