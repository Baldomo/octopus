use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

macro_rules! id {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            pub fn random() -> Self {
                Self(format!("{}{:08x}", $prefix, fastrand::u32(..)))
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id!(WorkspaceId, "wsp_");
id!(TaskId, "tsk_");
id!(PtyId, "pty_");

pub const ENV_TASK: &str = "OCTOPUS_TASK";
pub const ENV_SERVER: &str = "OCTOPUS_SERVER";
pub const AGENT_PANEL: &str = "agent";

/// Parses a unit enum from its wire spelling, e.g. `"pinned"`.
pub fn from_wire<T: DeserializeOwned>(text: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(text.to_string())).map_err(|error| error.to_string())
}

/// The wire spelling of a unit enum, e.g. `"pinned"`.
pub fn to_wire<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(text)) => text,
        other => panic!("{other:?} is not a unit enum's wire spelling"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Pinned,
    Temporary,
    Completed,
    Cancelled,
}

impl TaskState {
    pub const ALL: [Self; 4] = [Self::Pinned, Self::Temporary, Self::Completed, Self::Cancelled];

    pub fn is_live(self) -> bool {
        matches!(self, Self::Pinned | Self::Temporary)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Checkout {
    Root,
    Worktree,
}

impl Checkout {
    pub const ALL: [Self; 2] = [Self::Root, Self::Worktree];
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub root: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Panel {
    pub id: PtyId,
    pub name: String,
    pub running: bool,
    pub lock_holder: Option<String>,
}

impl Panel {
    pub fn answers_to(&self, key: &str) -> bool {
        self.name == key || self.id.0 == key
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Working,
    Waiting,
    Idle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Agent {
    NotStarted,
    Running { status: AgentStatus },
    Exited,
    HarnessNotInstalled { install: String },
    HarnessUnknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: TaskId,
    pub workspace: WorkspaceId,
    pub parent: Option<TaskId>,
    pub title: String,
    pub state: TaskState,
    pub checkout: Checkout,
    pub harness: String,
    pub directory: String,
    pub branch: Option<String>,
    pub focused_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub rev: u64,
    pub agent: Agent,
    pub panels: Vec<Panel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stream {
    Pty(PtyId),
    File { task: TaskId, path: String },
}

impl Stream {
    pub fn name(&self) -> String {
        match self {
            Self::Pty(pty) => format!("pty:{pty}"),
            Self::File { task, path } => format!("file:{task}/{path}"),
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name.split_once(':')? {
            ("pty", pty) => Some(Self::Pty(PtyId(pty.to_string()))),
            ("file", rest) => {
                let (task, path) = rest.split_once('/')?;
                Some(Self::File { task: TaskId(task.to_string()), path: path.to_string() })
            }
            _ => None,
        }
    }
}
