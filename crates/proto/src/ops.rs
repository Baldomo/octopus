use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::frames::Op;
use crate::pairing::{Code, Proof};
use crate::schema::{Field, record};
use crate::types::{
    AgentStatus, BrowserId, BrowserTarget, Checkout, SessionId, Task, TaskId, TaskState, Workspace, WorkspaceId,
};

/// Declares an op: its args as a `record!`, its wire name, and the type of its result.
macro_rules! op {
    ($(#[doc = $doc:literal])* $name:ident = $wire:literal -> $ok:ty { $($fields:tt)* }) => {
        record!($(#[doc = $doc])* $name { $($fields)* });

        impl Op for $name {
            const NAME: &'static str = $wire;
            const DOC: &'static str = concat!($($doc),*);
            type Ok = $ok;
        }
    };
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revised {
    pub rev: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Welcome {
    pub server: String,
    pub rev: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceCreated {
    pub workspace: WorkspaceId,
    pub rev: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCreated {
    pub task: TaskId,
    pub rev: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspaces {
    pub workspaces: Vec<Workspace>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub rev: u64,
    pub workspaces: Vec<Workspace>,
    pub tasks: Vec<Task>,
}

op!(
    /// Open a session; the first request on every control connection.
    SessionHello = "session.hello" -> Welcome {
        /// This client's name, as terminal lock holders see it.
        client: String,
    }
);
op!(
    /// Answer with the server's whole state.
    SessionSync = "session.sync" -> Snapshot {
        /// The rev of the client's cache of this server, or null for a client holding none.
        since_rev: Option<u64>,
    }
);
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Done {}

op!(
    /// Answer at once; the client times its connection's round trip with it.
    SessionPing = "session.ping" -> Done {}
);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Offer {
    #[ts(type = "string")]
    pub code: Code,
    /// When the code stops pairing, in milliseconds since 1970.
    pub expires_at: i64,
}

/// A client the server has pinned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PairedClient {
    /// The client's name, as it paired.
    pub name: String,
    pub connected: bool,
    /// When one of its connections last opened or closed, in milliseconds since 1970; null if it never connected.
    pub seen_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clients {
    pub clients: Vec<PairedClient>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Paired {
    /// The server's name, which the client lists it under.
    pub server: String,
    pub proof: Proof,
}

op!(
    /// Show a one-time code that pairs one client with this server.
    PairingOffer = "pairing.offer" -> Offer {}
);
op!(
    /// List the clients this server has paired with.
    PairingList = "pairing.list" -> Clients {}
);
op!(
    /// Forget a paired client, which must pair with a code again; its open connections stay until they close.
    PairingForget = "pairing.forget" -> Done {
        /// The client's name, as it paired.
        client: String,
    }
);
op!(
    /// Pair with a code: the one request an unpaired client may send, on its connection's first stream.
    PairingPair = "pairing.pair" -> Paired {
        /// The client's name, as terminal lock holders see it.
        client: String,
        /// The client's proof that it holds the code.
        proof: Proof,
    }
);

op!(
    /// Add a workspace: a directory this server runs tasks in.
    WorkspaceAdd = "workspace.add" -> WorkspaceCreated {
        /// The workspace's name, unique on its server.
        name: String,
        /// The directory on the server's machine, as an absolute path.
        root: String,
    }
);
op!(
    /// List the server's workspaces.
    WorkspaceList = "workspace.list" -> Workspaces {}
);
op!(
    /// Rename a workspace; it keeps its root and its tasks.
    WorkspaceRename = "workspace.rename" -> Revised {
        /// The workspace.
        workspace: WorkspaceId,
        /// Its new name, unique on its server.
        name: String,
    }
);
op!(
    /// Remove a workspace with no live task, with every task in it; their worktrees go and their branches stay.
    WorkspaceRemove = "workspace.remove" -> Revised {
        /// The workspace.
        workspace: WorkspaceId,
    }
);
op!(
    /// Declare how a harness starts its agent.
    HarnessSet = "harness.set" -> Revised {
        /// The harness's id, which tasks name.
        id: String,
        /// The agent's argv as a JSON array of strings, e.g. ["claude"].
        argv: Vec<String>,
        /// The args appended to argv on a session's first start, with {session} for the session id, e.g.
        /// ["--session-id","{session}"].
        start: Vec<String>,
        /// The args appended to argv on every later start, with {session} for the session id, e.g.
        /// ["--resume","{session}"].
        resume: Vec<String>,
        /// The command that installs the agent, shown when it is missing.
        install: String,
        /// Whether the agent reports its status with `agent report`.
        hooks: bool,
    }
);
op!(
    /// Report what a task's agent is doing; harness hooks call this.
    AgentReport = "agent.report" -> Revised {
        /// The task's id.
        task: TaskId,
        /// The agent's status.
        status: AgentStatus,
    }
);
op!(
    /// Create a task.
    TaskCreate = "task.create" -> TaskCreated {
        /// The workspace the task works in.
        workspace: WorkspaceId,
        /// The task's title.
        title: String,
        /// The parent task's id.
        parent: Option<TaskId>,
        /// The task's state.
        state: TaskState,
        /// Where the task's files live: the workspace root, or a worktree of its own.
        checkout: Checkout,
        /// The harness that runs the task's agent.
        harness: String,
        /// An existing session for the agent to resume.
        session: Option<SessionId>,
    }
);
op!(
    /// Point a task's agent at an existing session, ending the agent running now; the next focus resumes it.
    TaskSetSession = "task.set_session" -> Revised {
        /// The task's id.
        task: TaskId,
        /// The session to resume.
        session: SessionId,
    }
);
op!(
    /// Move a task to a state.
    TaskMove = "task.move" -> Revised {
        /// The task's id.
        task: TaskId,
        /// The state to move it to.
        state: TaskState,
    }
);
op!(
    /// Rename a task; a worktree task's branch keeps its name.
    TaskRename = "task.rename" -> Revised {
        /// The task's id.
        task: TaskId,
        /// The task's new title.
        title: String,
    }
);
op!(
    /// Archive an ended task, which takes it off the Tasks board until it moves again.
    TaskArchive = "task.archive" -> Revised {
        /// The task's id.
        task: TaskId,
    }
);
op!(
    /// Focus a task, starting its agent if it has none.
    TaskFocus = "task.focus" -> Revised {
        /// The task's id.
        task: TaskId,
    }
);
op!(
    /// Print a task.
    TaskGet = "task.get" -> Task {
        /// The task's id.
        task: TaskId,
    }
);
op!(
    /// Type text into a terminal panel.
    TermSend = "term.send" -> Revised {
        /// The task's id.
        task: TaskId,
        /// The panel's name or id, e.g. agent.
        panel: String,
        /// The text to type.
        text: String,
        /// Whether Enter follows the text.
        enter: bool,
        /// Whether to take the panel's write lock from its holder.
        take_lock: bool,
    }
);
op!(
    /// Take a terminal panel's write lock from its holder.
    TermTakeLock = "term.take_lock" -> Revised {
        /// The task's id.
        task: TaskId,
        /// The panel's name or id, e.g. agent.
        panel: String,
    }
);
op!(
    /// Resize a terminal panel; only its lock holder may.
    TermResize = "term.resize" -> Revised {
        /// The task's id.
        task: TaskId,
        /// The panel's name or id, e.g. agent.
        panel: String,
        /// Columns.
        cols: u16,
        /// Rows.
        rows: u16,
    }
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entries {
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Match {
    pub path: String,
    pub line: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Matches {
    pub matches: Vec<Match>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Written {
    pub bytes: u64,
}

op!(
    /// List a directory of a task's checkout.
    FileList = "file.list" -> Entries {
        /// The task's id.
        task: TaskId,
        /// The directory, relative to the checkout; . for its root.
        path: String,
    }
);
op!(
    /// Create a file in a task's checkout, or replace its content.
    FileWrite = "file.write" -> Written {
        /// The task's id.
        task: TaskId,
        /// The file, relative to the checkout.
        path: String,
        /// The file's new content, as text.
        content: String,
    }
);
op!(
    /// Find each line of a task's checkout that contains a text.
    FileSearch = "file.search" -> Matches {
        /// The task's id.
        task: TaskId,
        /// The literal text to find.
        pattern: String,
    }
);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shown {
    pub panel: BrowserId,
    pub rev: u64,
}

op!(
    /// Show a page in one of a task's browser panels, on every client that renders the task.
    BrowserOpen = "browser.open" -> Shown {
        /// The task's id.
        task: TaskId,
        /// The page, an http or https URL; one on localhost:<port> reaches that port on the server's machine.
        url: String,
        /// `new` for a new browser panel, or an existing browser panel's id.
        panel: BrowserTarget,
    }
);

/// An op as a CLI command: `<noun>.<verb>` runs as `octopus <noun> <verb>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub op: &'static str,
    pub doc: &'static str,
    pub fields: Vec<Field>,
}

fn command<O: Op>() -> Command {
    Command { op: O::NAME, doc: O::DOC.trim(), fields: O::fields() }
}

/// Every op a client runs on its own; the session ops and `pairing.pair` belong to the connection that carries them.
pub fn commands() -> Vec<Command> {
    vec![
        command::<WorkspaceAdd>(),
        command::<WorkspaceList>(),
        command::<WorkspaceRename>(),
        command::<WorkspaceRemove>(),
        command::<HarnessSet>(),
        command::<AgentReport>(),
        command::<TaskCreate>(),
        command::<TaskMove>(),
        command::<TaskRename>(),
        command::<TaskSetSession>(),
        command::<TaskArchive>(),
        command::<TaskFocus>(),
        command::<TaskGet>(),
        command::<TermSend>(),
        command::<TermTakeLock>(),
        command::<TermResize>(),
        command::<FileList>(),
        command::<FileWrite>(),
        command::<FileSearch>(),
        command::<BrowserOpen>(),
        command::<PairingOffer>(),
        command::<PairingList>(),
        command::<PairingForget>(),
    ]
}
