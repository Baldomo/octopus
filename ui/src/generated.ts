// Rendered from the Rust types by `cargo run -p octopus-app --example types`. Edit those, not this.

export type TaskId = string;

export type WorkspaceId = string;

export type PtyId = string;

export type BrowserId = string;

export type SessionId = string;

export type TaskState = "pinned" | "temporary" | "completed" | "cancelled";

export type Checkout = "root" | "worktree";

export type AgentStatus = "working" | "waiting" | "idle";

export type Hue = "red" | "orange" | "amber" | "lime" | "green" | "mint" | "teal" | "sky" | "blue" | "indigo" | "violet" | "pink";

export type Tone = "mist" | "soft" | "vivid" | "deep";

export type Swatch = { hue: Hue, tone: Tone, };

export type Agent = { "kind": "not_started" } | { "kind": "running", status: AgentStatus, } | { "kind": "exited" } | { "kind": "harness_not_installed", install: string, } | { "kind": "harness_unknown" };

export type Panel = { id: PtyId, name: string, running: boolean, lock_holder: string | null, cols: number, rows: number, };

export type BrowserPanel = { panel: BrowserId, url: string, };

export type Task = { id: TaskId, workspace: WorkspaceId, parent: TaskId | null, title: string, state: TaskState, checkout: Checkout, harness: string, session: SessionId | null, directory: string, branch: string | null, focused_at: number | null, ended_at: number | null, archived_at: number | null, rev: number, agent: Agent, panels: Array<Panel>, browsers: Array<BrowserPanel>, };

export type EntryKind = "file" | "directory";

export type Entry = { name: string, kind: EntryKind, };

export type Match = { path: string, line: number, text: string, };

export type Offer = { invite: string, 
/**
 * When the invite stops pairing, in milliseconds since 1970.
 */
expires_at: number, };

export type ShownOffer = { offer: Offer, qr: string, };

export type PairedClient = { 
/**
 * The client's name, as it paired.
 */
name: string, connected: boolean, 
/**
 * When one of its connections last opened or closed, in milliseconds since 1970; null if it never connected.
 */
seen_at: number | null, };

export type TaskRef = { server: string, task: TaskId, };

export type Glyph = "working" | "waiting" | "idle" | "completed" | "cancelled";

export type Row = { key: string, at: TaskRef, title: string, depth: number, 
/**
 * The task's own state, which a subtask's group or column may differ from.
 */
state: TaskState, glyph: Glyph, };

export type Column = { state: TaskState, rows: Array<Row>, };

export type Section = { name: string, 
/**
 * The section's color, with auto resolved.
 */
swatch: Swatch, 
/**
 * The color auto resolves to for the section's name.
 */
auto: Swatch, live: number, 
/**
 * Whether the agent of one of its live tasks is waiting on the user.
 */
waiting: boolean, 
/**
 * The sidebar's groups, one per live state, of the live tasks.
 */
groups: Array<Column>, 
/**
 * The board's columns, one per state in `TaskState::ALL` order, of the unarchived tasks.
 */
columns: Array<Column>, };

export type WorkspaceRef = { server: string, workspace: WorkspaceId, name: string, 
/**
 * The directory on the server's machine.
 */
root: string, };

export type Lists = { sections: Array<Section>, workspaces: Array<WorkspaceRef>, };

export type TaskView = { key: string, at: TaskRef, section: string, 
/**
 * The workspace, then each ancestor root first, then the task.
 */
crumbs: Array<string>, 
/**
 * Where a click on the state pill moves the task.
 */
next: TaskState, live: boolean, glyph: Glyph, task: Task, };

export type ServerView = { name: string, 
/**
 * What the window calls the server: "local" for one on the local socket, else its name.
 */
label: string, 
/**
 * The relay a remote server is dialled through; none for a server on the local socket.
 */
relay: string | null, reachable: boolean, 
/**
 * When `reachable` last changed, in milliseconds since 1970.
 */
since: number, 
/**
 * The round trip of the connection's newest heartbeat.
 */
rtt_ms: number | null, problem: string | null, };

export type Model = { 
/**
 * Raised by every model built, so the UI drops one that arrives after a newer one.
 */
version: number, lists: Lists, tasks: { [key in string]: TaskView }, };

export type FilesChanged = { key: string, paths: Array<string>, };

export type View = "task" | "tasks";

export type Surface = "files" | "browser";

export type Modal = "palette" | "new_task" | "pair_server" | "shortcuts";

export type Appearance = "system" | "light" | "dark";

export type Zoom = number;

export type ZoomStep = "in" | "out" | "reset";

export type Prefs = { pinned: boolean, collapsed: Array<string>, surface: Surface, view: View, 
/**
 * The open task's key.
 */
open: string | null, 
/**
 * The section the Tasks board shows.
 */
board: string | null, appearance: Appearance, zoom: Zoom, };

export type Reveal = { 
/**
 * The task's key.
 */
task: string, path: string, 
/**
 * Counts reveals, so revealing the same file again opens it again.
 */
nth: number, };

export type Ui = { 
/**
 * Raised by every change, so a layer drops a state that arrives after a newer one.
 */
version: number, prefs: Prefs, 
/**
 * The opened tasks' keys, newest first.
 */
recent: Array<string>, sidebar: boolean, client_menu: boolean, modal: Modal | null, reveal: Reveal | null, problem: string | null, 
/**
 * The window's size in the UI's CSS pixels, which the zoom scales.
 */
viewport: Viewport, 
/**
 * Where the float layer takes the pointer over the layers beneath it.
 */
float_input: Array<Bounds>, };

export type UiAct = { "do": "open_task", key: string, } | { "do": "show_view", view: View, } | { "do": "show_board", section: string, } | { "do": "show_surface", surface: Surface, } | { "do": "toggle_pin" } | { "do": "toggle_collapsed", section: string, } | { "do": "show_sidebar", shown: boolean, } | { "do": "toggle_sidebar" } | { "do": "show_client_menu", shown: boolean, } | { "do": "toggle_client_menu" } | { "do": "set_appearance", appearance: Appearance, } | { "do": "zoom", step: ZoomStep, } | { "do": "show_modal", modal: Modal | null, } | { "do": "float", surface: string, at: Floated, modal: boolean, } | { "do": "sink", surface: string, } | { "do": "reveal", task: string, path: string, } | { "do": "report", problem: string, };

export type Viewport = { width: number, height: number, };

export type Boot = { 
/**
 * This client's name, as terminal lock holders see it.
 */
me: string, servers: Array<ServerView>, model: Model, ui: Ui, };

export type NewTask = { server: string, workspace: WorkspaceId, title: string, parent: TaskId | null, state: TaskState, checkout: Checkout, harness: string, };

export type Bounds = { x: number, y: number, width: number, height: number, };

export type Floated = { input: Bounds, drawn: Bounds, };

export type OnScreen = { at: TaskRef, panel: BrowserId, bounds: Bounds, };

export type Go = "back" | "forward" | "reload";

export type Navigated = { 
/**
 * The task's key.
 */
task: string, panel: BrowserId, url: string, };

export const TASK_STATES = ["pinned","temporary","completed","cancelled"] as const;

export const LIVE_STATES = ["pinned","temporary"] as const;

export const CHECKOUTS = ["root","worktree"] as const;

export const HUES = ["red","orange","amber","lime","green","mint","teal","sky","blue","indigo","violet","pink"] as const;

export const TONES = ["mist","soft","vivid","deep"] as const;

export const AGENT_PANEL = "agent" as const;

export const NEW_BROWSER_PANEL = "new" as const;

export const VERSION = "0.0.0" as const;

export const FLOAT_LAYER = "float" as const;

export const FLOAT_TOP = 38.0 as const;

export const EVENTS = {"browserNavigated":"browser_navigated","filesChanged":"files_changed","model":"model","servers":"servers","ui":"ui","windowShown":"window-shown"} as const;
