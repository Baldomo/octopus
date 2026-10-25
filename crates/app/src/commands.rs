use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use octopus_client::servers::Server;
use octopus_proto::ops::{
    Entry, FileList, FileSearch, FileWrite, Match, TaskCreate, TaskFocus, TaskMove, TermResize, TermTakeLock,
};
use octopus_proto::types::{Checkout, PtyId, Stream, TaskId, TaskState, WorkspaceId};
use serde::{Deserialize, Serialize};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, State};
use tokio::io::AsyncReadExt;
use ts_rs::TS;

use crate::browser::{Bounds, Browsers, Go};
use crate::hub::{Hub, Model, ServerView, failure};
use crate::task_lists::TaskRef;
use crate::terminal::Terminals;

type Hubbed<'a> = State<'a, Arc<Hub>>;

pub struct Launched(pub Instant);

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Boot {
    /// This client's name, as terminal lock holders see it.
    pub me: String,
    pub servers: Vec<ServerView>,
    pub model: Model,
}

#[tauri::command]
pub fn boot(hub: Hubbed) -> Boot {
    Boot { me: hub.me.clone(), servers: hub.servers(), model: hub.model() }
}

#[tauri::command]
pub fn ready(launched: State<Launched>) {
    eprintln!("octopus app: interactive {} ms after launch", launched.0.elapsed().as_millis());
}

#[tauri::command]
pub async fn server_add(hub: Hubbed<'_>, name: String, socket: PathBuf) -> Result<(), String> {
    hub.add(Server { name, socket }).map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub async fn task_focus(hub: Hubbed<'_>, at: TaskRef) -> Result<(), String> {
    hub.call(&at.server, &TaskFocus { task: at.task }).await.map(drop)
}

#[tauri::command]
pub async fn task_move(hub: Hubbed<'_>, at: TaskRef, state: TaskState) -> Result<(), String> {
    hub.call(&at.server, &TaskMove { task: at.task, state }).await.map(drop)
}

/// Every field of `task.create`, plus the server that creates it.
#[derive(Debug, Clone, PartialEq, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct NewTask {
    pub server: String,
    pub workspace: WorkspaceId,
    pub title: String,
    pub parent: Option<TaskId>,
    pub state: TaskState,
    pub checkout: Checkout,
    pub harness: String,
}

/// Resolves to the created task's key.
#[tauri::command]
pub async fn task_create(hub: Hubbed<'_>, task: NewTask) -> Result<String, String> {
    let NewTask { server, workspace, title, parent, state, checkout, harness } = task;
    let created = hub.call(&server, &TaskCreate { workspace, title, parent, state, checkout, harness }).await?;
    Ok(TaskRef { server, task: created.task }.key())
}

#[tauri::command]
pub async fn term_attach(
    hub: Hubbed<'_>,
    terminals: State<'_, Terminals>,
    at: TaskRef,
    pty: PtyId,
    output: Channel<InvokeResponseBody>,
) -> Result<u64, String> {
    let connection = hub.connection(&at.server)?;
    terminals.attach(&connection, pty, output).await.map_err(|error| failure(&at.server, error))
}

#[tauri::command]
pub fn term_input(terminals: State<Terminals>, attachment: u64, keys: String) -> Result<(), String> {
    terminals.input(attachment, keys.into_bytes())
}

#[tauri::command]
pub fn term_detach(terminals: State<Terminals>, attachment: u64) {
    terminals.detach(attachment);
}

#[tauri::command]
pub async fn term_take_lock(hub: Hubbed<'_>, at: TaskRef, panel: PtyId) -> Result<(), String> {
    hub.call(&at.server, &TermTakeLock { task: at.task, panel: panel.0 }).await.map(drop)
}

#[tauri::command]
pub async fn term_resize(hub: Hubbed<'_>, at: TaskRef, panel: PtyId, cols: u16, rows: u16) -> Result<(), String> {
    hub.call(&at.server, &TermResize { task: at.task, panel: panel.0, cols, rows }).await.map(drop)
}

#[tauri::command]
pub async fn file_list(hub: Hubbed<'_>, at: TaskRef, path: String) -> Result<Vec<Entry>, String> {
    Ok(hub.call(&at.server, &FileList { task: at.task, path }).await?.entries)
}

#[tauri::command]
pub async fn file_read(hub: Hubbed<'_>, at: TaskRef, path: String) -> Result<String, String> {
    let connection = hub.connection(&at.server)?;
    let stream = Stream::File { task: at.task, path: path.clone() };
    let mut opened = connection.open_data_stream(&stream).await.map_err(|error| failure(&at.server, error))?;
    let mut bytes = Vec::new();
    opened.read_to_end(&mut bytes).await.map_err(|error| format!("read {path}: {error}"))?;
    String::from_utf8(bytes).map_err(|_| format!("{path} is not UTF-8 text"))
}

#[tauri::command]
pub async fn file_write(hub: Hubbed<'_>, at: TaskRef, path: String, content: String) -> Result<(), String> {
    hub.call(&at.server, &FileWrite { task: at.task, path, content }).await.map(drop)
}

#[tauri::command]
pub async fn file_search(hub: Hubbed<'_>, at: TaskRef, pattern: String) -> Result<Vec<Match>, String> {
    Ok(hub.call(&at.server, &FileSearch { task: at.task, pattern }).await?.matches)
}

#[tauri::command]
pub async fn browser_open(
    app: AppHandle,
    browsers: State<'_, Browsers>,
    panel: String,
    url: String,
) -> Result<(), String> {
    browsers.open(&app, &panel, &url)
}

#[tauri::command]
pub async fn browser_place(browsers: State<'_, Browsers>, panel: String, bounds: Option<Bounds>) -> Result<(), String> {
    browsers.place(&panel, bounds)
}

#[tauri::command]
pub async fn browser_go(browsers: State<'_, Browsers>, panel: String, go: Go) -> Result<(), String> {
    browsers.go(&panel, go)
}

#[tauri::command]
pub async fn browser_close(browsers: State<'_, Browsers>, panel: String) -> Result<(), String> {
    browsers.close(&panel)
}
