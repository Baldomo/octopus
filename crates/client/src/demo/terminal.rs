//! A demo terminal: its output since it opened, which redraws its screen, and the program reading its input, a
//! recorded agent or a scripted shell that runs nothing.

use std::collections::HashMap;
use std::sync::Arc;

use octopus_proto::device::PublicKey;
use octopus_proto::term::{Piece, Screen};
use octopus_proto::types::TaskId;
use tokio::sync::{broadcast, oneshot};

use super::project::Project;

const PIECE_BACKLOG: usize = 1024;
pub(super) const CLEAR: &str = "\x1b[H\x1b[2J\x1b[3J";
const SHELL_PROMPT: &str = "\x1b[34mtidewatch\x1b[0m $ ";

pub(super) enum Program {
    Agent(Turn),
    Shell { branch: String },
}

/// A playing agent ignores typing; an asking one hands the next line to its recording.
pub(super) enum Turn {
    Ready,
    Playing,
    Asking(oneshot::Sender<String>),
}

pub(super) enum Typed {
    UnaskedRequest,
    Exit,
}

pub(super) struct Term {
    pub task: TaskId,
    pub program: Program,
    output: Vec<u8>,
    pieces: broadcast::Sender<Piece>,
    line: String,
    in_escape: bool,
    streams_per_viewer: HashMap<PublicKey, usize>,
    /// Held through each recording played on the terminal, so no two interleave.
    pub stage: Arc<tokio::sync::Mutex<()>>,
}

/// Recordings break lines with LF alone.
fn crlf(text: &str) -> Vec<u8> {
    text.replace('\n', "\r\n").into_bytes()
}

impl Term {
    pub(super) fn new(task: TaskId, program: Program, output: &str) -> Self {
        let (pieces, _) = broadcast::channel(PIECE_BACKLOG);
        let mut term = Self {
            task,
            program,
            output: Vec::new(),
            pieces,
            line: String::new(),
            in_escape: false,
            streams_per_viewer: HashMap::new(),
            stage: Arc::default(),
        };
        term.write(output);
        term
    }

    pub(super) fn shell(task: TaskId, branch: String) -> Self {
        Self::new(task, Program::Shell { branch }, SHELL_PROMPT)
    }

    pub(super) fn write(&mut self, text: &str) {
        let bytes = crlf(text);
        self.output.extend_from_slice(&bytes);
        let _nobody_attached = self.pieces.send(Piece::Output(bytes.into()));
    }

    pub(super) fn clear(&mut self) {
        self.output.clear();
        self.write(CLEAR);
    }

    pub(super) fn screen(&self, cols: u16, rows: u16) -> Piece {
        Piece::Screen(Screen { cols, rows, bytes: Arc::from(self.output.as_slice()) })
    }

    pub(super) fn resized(&self, cols: u16, rows: u16) {
        let _nobody_attached = self.pieces.send(self.screen(cols, rows));
    }

    pub(super) fn attach(&mut self, viewer: PublicKey, cols: u16, rows: u16) -> (Piece, broadcast::Receiver<Piece>) {
        *self.streams_per_viewer.entry(viewer).or_default() += 1;
        (self.screen(cols, rows), self.pieces.subscribe())
    }

    /// Resolves to whether `viewer` still shows the terminal on another stream.
    pub(super) fn detach(&mut self, viewer: PublicKey) -> bool {
        let Some(count) = self.streams_per_viewer.get_mut(&viewer) else { return false };
        *count -= 1;
        let still = *count > 0;
        if !still {
            self.streams_per_viewer.remove(&viewer);
        }
        still
    }

    /// Echoes what is typed and runs each line it finishes.
    pub(super) fn type_text(&mut self, text: &str, project: &Project) -> Vec<Typed> {
        let mut typed = Vec::new();
        for key in text.chars() {
            match key {
                _ if matches!(self.program, Program::Agent(Turn::Playing)) => {}
                _ if self.in_escape => self.in_escape = !key.is_ascii_alphabetic() && key != '~',
                '\x1b' => self.in_escape = true,
                '\r' | '\n' => {
                    let line = std::mem::take(&mut self.line);
                    self.write("\n");
                    typed.extend(self.run(line, project));
                }
                '\x03' => {
                    self.line.clear();
                    self.write("^C\n");
                    self.prompt();
                }
                '\x7f' | '\x08' => {
                    if self.line.pop().is_some() {
                        self.write("\x08 \x08");
                    }
                }
                key if !key.is_control() => {
                    self.line.push(key);
                    self.write(key.encode_utf8(&mut [0; 4]));
                }
                _ => {}
            }
        }
        typed
    }

    fn prompt(&mut self) {
        match self.program {
            Program::Agent(_) => self.write(super::sessions::PROMPT),
            Program::Shell { .. } => self.write(SHELL_PROMPT),
        }
    }

    fn run(&mut self, line: String, project: &Project) -> Option<Typed> {
        match &mut self.program {
            Program::Agent(Turn::Ready) if line.trim().is_empty() => {
                self.prompt();
                None
            }
            Program::Agent(turn) => match std::mem::replace(turn, Turn::Playing) {
                Turn::Asking(answer) => {
                    let _player_gone = answer.send(line);
                    None
                }
                Turn::Ready => Some(Typed::UnaskedRequest),
                Turn::Playing => None,
            },
            Program::Shell { branch } => {
                let branch = branch.clone();
                match shell(line.trim(), &branch, project) {
                    Ran::Printed(text) => self.write(&text),
                    Ran::Cleared => self.clear(),
                    Ran::Exited => return Some(Typed::Exit),
                }
                self.write(SHELL_PROMPT);
                None
            }
        }
    }
}

enum Ran {
    Printed(String),
    Cleared,
    Exited,
}

const HELP: &str = "\
The demo computer's shell runs nothing. It knows:
  help              this list
  echo <text>       prints the text
  ls [folder]       lists a folder of the project
  cat <file>        prints a file of the project
  pwd, whoami, hostname
  git status, git log
  make test
  clear, exit
";

const GIT_LOG: &str = "\
\x1b[33ma41c9e2\x1b[0m Draw the week's tides as a chart
\x1b[33m7be0d13\x1b[0m Read harbor.csv with its offset
\x1b[33m3f5a8c1\x1b[0m List today's highs and lows
\x1b[33m0c2d9b4\x1b[0m Start tidewatch
";

const MAKE_TEST: &str = "\
· tests/forecast.test.js
\x1b[32m✓\x1b[0m groups tides by day
\x1b[32m✓\x1b[0m keeps each tide's offset
\x1b[32m✓\x1b[0m reads a bare time as +00:00
\x1b[32m✓\x1b[0m skips blank rows
\x1b[2m4 passed\x1b[0m
";

fn shell(line: &str, branch: &str, project: &Project) -> Ran {
    let (command, rest) = line.split_once(' ').map_or((line, ""), |(command, rest)| (command, rest.trim()));
    match (command, rest) {
        ("", _) => Ran::Printed(String::new()),
        ("help", _) => Ran::Printed(HELP.into()),
        ("echo", text) => Ran::Printed(format!("{text}\n")),
        ("pwd", _) => Ran::Printed(format!("{}\n", super::project::ROOT)),
        ("whoami", _) => Ran::Printed("demo\n".into()),
        ("hostname", _) => Ran::Printed(format!("{}\n", super::NAME)),
        ("ls", folder) => Ran::Printed(match project.list(if folder.is_empty() { "." } else { folder }) {
            Ok(entries) => entries.iter().map(|entry| format!("{}\n", entry.name)).collect(),
            Err(error) => format!("ls: {error}\n"),
        }),
        ("cat", file) => Ran::Printed(match project.file(file) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(bytes).into_owned();
                if text.ends_with('\n') { text } else { format!("{text}\n") }
            }
            Err(error) => format!("cat: {error}\n"),
        }),
        ("git", "status") => Ran::Printed(format!("On branch {branch}\nnothing to commit, working tree clean\n")),
        ("git", "log" | "log --oneline") => Ran::Printed(GIT_LOG.into()),
        ("make", "test" | "") => Ran::Printed(MAKE_TEST.into()),
        ("clear", _) => Ran::Cleared,
        ("exit", _) => Ran::Exited,
        (command, _) => Ran::Printed(format!("{command}: not on the demo computer; try help\n")),
    }
}
