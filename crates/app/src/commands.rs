use std::sync::Arc;
use std::time::Instant;

use octopus_client::servers;
use octopus_proto::ops::{
    BrowserOpen, Entry, FileList, FileSearch, FileWrite, Match, TaskCreate, TaskFocus, TaskMove, TermResize,
    TermTakeLock,
};
use octopus_proto::pairing::Code;
use octopus_proto::types::{BrowserId, BrowserTarget, Checkout, PtyId, Stream, TaskId, TaskState, WorkspaceId};
use serde::{Deserialize, Serialize};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, State};
use tokio::io::AsyncReadExt;
use ts_rs::TS;

use crate::browser::{Browsers, Go, OnScreen};
use crate::hub::{Hub, Model, ServerView, failure};
use crate::task_lists::TaskRef;
use crate::terminal::Terminals;
use crate::ui::{Ui, UiAct, UiStore};

type Hubbed<'a> = State<'a, Arc<Hub>>;
type Uis<'a> = State<'a, Arc<UiStore>>;
type Browsing<'a> = State<'a, Browsers>;

pub struct Launched(pub Instant);

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Boot {
    /// This client's name, as terminal lock holders see it.
    pub me: String,
    pub servers: Vec<ServerView>,
    pub model: Model,
    pub ui: Ui,
}

#[tauri::command]
pub fn boot(hub: Hubbed, ui: Uis) -> Boot {
    Boot { me: hub.me.name.clone(), servers: hub.servers(), model: hub.model(), ui: ui.ui() }
}

#[tauri::command]
pub fn ready(webview: tauri::Webview, launched: State<Launched>) {
    eprintln!("octopus app: {} interactive {} ms after launch", webview.label(), launched.0.elapsed().as_millis());
}

#[tauri::command]
pub fn ui_act(ui: Uis, act: UiAct) -> Result<(), String> {
    ui.act(act)
}

/// Pairs with the server at `address` using the code it offered, lists it, and resolves to its name.
#[tauri::command]
pub async fn server_pair(hub: Hubbed<'_>, address: String, code: Code) -> Result<String, String> {
    hub.refuse_listed(&address).map_err(|error| format!("{error:#}"))?;
    let server = servers::pair(&address, &code, &hub.me).await.map_err(|error| format!("{error:#}"))?;
    let name = server.name.clone();
    hub.add(server).map_err(|error| format!("{error:#}"))?;
    Ok(name)
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

/// Shows `url` in a browser panel of the task on its server, which pushes the panel to every client; resolves to
/// the panel's id.
#[tauri::command]
pub async fn browser_open(
    hub: Hubbed<'_>,
    at: TaskRef,
    panel: BrowserTarget,
    url: String,
) -> Result<BrowserId, String> {
    Ok(hub.call(&at.server, &BrowserOpen { task: at.task, url, panel }).await?.panel)
}

/// Renders the panel's server `url` on this client; resolves to the port it reaches through a tunnel, if any.
#[tauri::command]
pub async fn browser_load(
    app: AppHandle,
    hub: Hubbed<'_>,
    browsers: Browsing<'_>,
    at: TaskRef,
    panel: BrowserId,
    url: String,
) -> Result<Option<u16>, String> {
    browsers.load(&app, &hub, &at, &panel, &url).await
}

#[tauri::command]
pub async fn browser_show_page(browsers: Browsing<'_>, on_screen: Option<OnScreen>) -> Result<(), String> {
    browsers.show_page(on_screen)
}

#[tauri::command]
pub async fn browser_go(browsers: Browsing<'_>, at: TaskRef, panel: BrowserId, go: Go) -> Result<(), String> {
    browsers.go(&at, &panel, go)
}

/// Closes the panel's webview on this client; the panel stays on its server.
#[tauri::command]
pub async fn browser_close(browsers: Browsing<'_>, at: TaskRef, panel: BrowserId) -> Result<(), String> {
    browsers.close(&at, &panel).await
}

/// What double-clicking the title bar does, as the user set it: on macOS in System Settings ▸ Desktop & Dock,
/// elsewhere always a zoom.
fn title_bar_action() -> &'static str {
    if !cfg!(target_os = "macos") {
        return "Maximize";
    }
    let read = std::process::Command::new("defaults").args(["read", "-g", "AppleActionOnDoubleClick"]).output();
    match read.map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string()).as_deref() {
        Ok("Minimize") => "Minimize",
        Ok("None") => "None",
        _ => "Maximize",
    }
}

#[tauri::command]
pub fn title_bar_double_click(window: tauri::Window) -> Result<(), String> {
    let done = match title_bar_action() {
        "Minimize" => window.minimize(),
        "None" => Ok(()),
        _ if window.is_maximized().map_err(|error| error.to_string())? => window.unmaximize(),
        _ => window.maximize(),
    };
    done.map_err(|error| error.to_string())
}
