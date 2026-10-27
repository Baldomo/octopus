use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::schema::{Arg, Shape};

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
id!(BrowserId, "brw_");

impl Arg for WorkspaceId {
    fn shape() -> Shape {
        Shape::Workspace
    }
}

impl Arg for TaskId {
    fn shape() -> Shape {
        Shape::Text
    }
}

/// The browser panel `browser.open` shows a page in: `new`, or an existing panel's id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserTarget {
    New,
    Panel(BrowserId),
}

/// How `browser.open` names a browser panel it is to add.
pub const NEW_BROWSER_PANEL: &str = "new";

impl Serialize for BrowserTarget {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::New => serializer.serialize_str(NEW_BROWSER_PANEL),
            Self::Panel(id) => id.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for BrowserTarget {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(if text == NEW_BROWSER_PANEL { Self::New } else { Self::Panel(BrowserId(text)) })
    }
}

impl Arg for BrowserTarget {
    fn shape() -> Shape {
        Shape::Text
    }
}

impl Arg for Option<TaskId> {
    fn shape() -> Shape {
        Shape::Nullable { value: Box::new(Shape::Text), none: "a root task" }
    }
}

/// The conversation a task's agent resumes, as its harness names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(transparent)]
pub struct SessionId(pub String);

impl Arg for SessionId {
    fn shape() -> Shape {
        Shape::Text
    }
}

impl Arg for Option<SessionId> {
    fn shape() -> Shape {
        Shape::Nullable { value: Box::new(Shape::Text), none: "a fresh session" }
    }
}

impl Arg for Option<u64> {
    fn shape() -> Shape {
        Shape::Nullable { value: Box::new(Shape::Integer), none: "none" }
    }
}

fn choice<T: Serialize>(all: &[T]) -> Shape {
    Shape::Choice(all.iter().map(to_wire).collect())
}

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
    pub const LIVE: [Self; 2] = [Self::Pinned, Self::Temporary];

    pub fn is_live(self) -> bool {
        Self::LIVE.contains(&self)
    }
}

impl Arg for TaskState {
    fn shape() -> Shape {
        choice(&Self::ALL)
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

impl Arg for Checkout {
    fn shape() -> Shape {
        choice(&Self::ALL)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Hue {
    Red,
    Orange,
    Amber,
    Lime,
    Green,
    Mint,
    Teal,
    Sky,
    Blue,
    Indigo,
    Violet,
    Pink,
}

impl Hue {
    pub const ALL: [Self; 12] = [
        Self::Red,
        Self::Orange,
        Self::Amber,
        Self::Lime,
        Self::Green,
        Self::Mint,
        Self::Teal,
        Self::Sky,
        Self::Blue,
        Self::Indigo,
        Self::Violet,
        Self::Pink,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Mist,
    Soft,
    Vivid,
    Deep,
}

impl Tone {
    pub const ALL: [Self; 4] = [Self::Mist, Self::Soft, Self::Vivid, Self::Deep];
}

/// One cell of the color matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Swatch {
    pub hue: Hue,
    pub tone: Tone,
}

/// A workspace's color: `auto`, which each client resolves from the name, or one swatch, spelled `violet-vivid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Auto,
    Picked(Swatch),
}

impl Color {
    pub fn all() -> impl Iterator<Item = Self> {
        let swatches = Tone::ALL.into_iter().flat_map(|tone| Hue::ALL.into_iter().map(move |hue| Swatch { hue, tone }));
        std::iter::once(Self::Auto).chain(swatches.map(Self::Picked))
    }

    pub fn name(self) -> String {
        match self {
            Self::Auto => "auto".into(),
            Self::Picked(Swatch { hue, tone }) => format!("{}-{}", to_wire(&hue), to_wire(&tone)),
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        Self::all().find(|color| color.name() == text).ok_or_else(|| format!("{text:?} is not auto or a <hue>-<tone>"))
    }
}

impl Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.name())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl Arg for Color {
    fn shape() -> Shape {
        Shape::Choice(Self::all().map(Self::name).collect())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub root: String,
    pub color: Color,
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

/// A page a browser panel may show: every client loads it in a webview, so only http and https.
pub fn web_page(url: &str) -> Result<url::Url, String> {
    let parsed = url::Url::parse(url).map_err(|error| format!("{url}: {error}"))?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed),
        scheme => Err(format!("a browser panel shows http and https pages, not {scheme}:")),
    }
}

/// A browser panel of a task, which every client of the task renders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserPanel {
    pub panel: BrowserId,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Working,
    Waiting,
    Idle,
}

impl AgentStatus {
    pub const ALL: [Self; 3] = [Self::Working, Self::Waiting, Self::Idle];
}

impl Arg for AgentStatus {
    fn shape() -> Shape {
        choice(&Self::ALL)
    }
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
    pub session: Option<SessionId>,
    pub directory: String,
    pub branch: Option<String>,
    pub focused_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub archived_at: Option<i64>,
    pub rev: u64,
    pub agent: Agent,
    pub panels: Vec<Panel>,
    pub browsers: Vec<BrowserPanel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stream {
    Pty(PtyId),
    File {
        task: TaskId,
        path: String,
    },
    /// A TCP connection to a port on the server's machine.
    Tunnel(u16),
}

impl Stream {
    pub fn name(&self) -> String {
        match self {
            Self::Pty(pty) => format!("pty:{pty}"),
            Self::File { task, path } => format!("file:{task}/{path}"),
            Self::Tunnel(port) => format!("tunnel:{port}"),
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name.split_once(':')? {
            ("pty", pty) => Some(Self::Pty(PtyId(pty.to_string()))),
            ("file", rest) => {
                let (task, path) = rest.split_once('/')?;
                Some(Self::File { task: TaskId(task.to_string()), path: path.to_string() })
            }
            ("tunnel", port) => Some(Self::Tunnel(port.parse().ok()?)),
            _ => None,
        }
    }
}
