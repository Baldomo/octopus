use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::frames::Op;
use crate::types::{AgentStatus, Checkout, Task, TaskId, TaskState, Workspace, WorkspaceId};

macro_rules! op {
    ($name:ident, $wire:literal, $ok:ty, { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name { $(pub $field: $ty),* }

        impl Op for $name {
            const NAME: &'static str = $wire;
            const FIELDS: &'static [&'static str] = &[$(stringify!($field)),*];
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

op!(SessionHello, "session.hello", Welcome, { client: String });
op!(SessionSync, "session.sync", Snapshot, { since_rev: Option<u64> });
op!(WorkspaceAdd, "workspace.add", WorkspaceCreated, { name: String, root: String });
op!(WorkspaceList, "workspace.list", Workspaces, {});
op!(HarnessSet, "harness.set", Revised, { id: String, argv: Vec<String>, install: String, hooks: bool });
op!(AgentReport, "agent.report", Revised, { task: TaskId, status: AgentStatus });
op!(TaskCreate, "task.create", TaskCreated, {
    workspace: WorkspaceId,
    title: String,
    parent: Option<TaskId>,
    state: TaskState,
    checkout: Checkout,
    harness: String,
});
op!(TaskMove, "task.move", Revised, { task: TaskId, state: TaskState });
op!(TaskFocus, "task.focus", Revised, { task: TaskId });
op!(TaskGet, "task.get", Task, { task: TaskId });
op!(TermSend, "term.send", Revised, { task: TaskId, panel: String, text: String, enter: bool, take_lock: bool });
op!(TermTakeLock, "term.take_lock", Revised, { task: TaskId, panel: String });
op!(TermResize, "term.resize", Revised, { task: TaskId, panel: String, cols: u16, rows: u16 });

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

op!(FileList, "file.list", Entries, { task: TaskId, path: String });
op!(FileWrite, "file.write", Written, { task: TaskId, path: String, content: String });
op!(FileSearch, "file.search", Matches, { task: TaskId, pattern: String });
