// Rendered from the Rust types by `cargo run -p octopus-app --example types`. Edit those, not this.

export type TaskId = string;

export type WorkspaceId = string;

export type PtyId = string;

export type BrowserId = string;

export type TaskState = "pinned" | "temporary" | "completed" | "cancelled";

export type Checkout = "root" | "worktree";

export type AgentStatus = "working" | "waiting" | "idle";

export type Agent = { "kind": "not_started" } | { "kind": "running", status: AgentStatus, } | { "kind": "exited" } | { "kind": "harness_not_installed", install: string, } | { "kind": "harness_unknown" };

export type Panel = { id: PtyId, name: string, running: boolean, lock_holder: string | null, };

export type BrowserPanel = { panel: BrowserId, url: string, };

export type Task = { id: TaskId, workspace: WorkspaceId, parent: TaskId | null, title: string, state: TaskState, checkout: Checkout, harness: string, directory: string, branch: string | null, focused_at: number | null, ended_at: number | null, rev: number, agent: Agent, panels: Array<Panel>, browsers: Array<BrowserPanel>, };

export type EntryKind = "file" | "directory";

export type Entry = { name: string, kind: EntryKind, };

export type Match = { path: string, line: number, text: string, };

export type TaskRef = { server: string, task: TaskId, };

export type Glyph = "working" | "waiting" | "idle" | "completed" | "cancelled";

export type Row = { key: string, at: TaskRef, title: string, depth: number, 
/**
 * The parent's title, for a subtask that sits apart from its parent.
 */
via: string | null, glyph: Glyph, subtasks: number, };

export type Column = { state: TaskState, 
/**
 * Whether the column's state is live, which puts it in the sidebar too.
 */
live: boolean, rows: Array<Row>, };

export type Section = { name: string, live: number, 
/**
 * One per state, in `TaskState::ALL` order, empty ones included.
 */
columns: Array<Column>, };

export type WorkspaceRef = { server: string, workspace: WorkspaceId, name: string, };

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

export type ServerView = { name: string, reachable: boolean, 
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

export type Boot = { 
/**
 * This client's name, as terminal lock holders see it.
 */
me: string, servers: Array<ServerView>, model: Model, };

export type NewTask = { server: string, workspace: WorkspaceId, title: string, parent: TaskId | null, state: TaskState, checkout: Checkout, harness: string, };

export type Bounds = { x: number, y: number, width: number, height: number, };

export type Go = "back" | "forward" | "reload";

export type Navigated = { 
/**
 * The task's key.
 */
task: string, panel: BrowserId, url: string, };

export const TASK_STATES = ["pinned","temporary","completed","cancelled"] as const;

export const CHECKOUTS = ["root","worktree"] as const;

export const AGENT_PANEL = "agent" as const;

export const NEW_BROWSER_PANEL = "new" as const;

export const EVENTS = {"browserNavigated":"browser_navigated","filesChanged":"files_changed","model":"model","servers":"servers"} as const;
