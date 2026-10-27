import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { EVENTS, NEW_BROWSER_PANEL } from "./generated";
import type {
  Boot,
  BrowserId,
  Entry,
  FilesChanged,
  Go,
  Match,
  Model,
  Navigated,
  NewTask,
  Offer,
  OnScreen,
  PairedClient,
  PtyId,
  ServerView,
  Swatch,
  TaskRef,
  TaskState,
  Ui,
  UiAct,
  WorkspaceId,
} from "./generated";

export const backend = {
  boot: () => invoke<Boot>("boot"),
  ready: () => invoke<void>("ready"),
  uiAct: (act: UiAct) => invoke<void>("ui_act", { act }),
  titleBarDoubleClick: () => invoke<void>("title_bar_double_click"),
  quit: () => invoke<void>("quit"),
  /** Resolves to the paired server's name. */
  serverPair: (address: string, code: string) => invoke<string>("server_pair", { address, code }),
  serverOffer: (server: string) => invoke<Offer>("server_offer", { server }),
  serverClients: (server: string) => invoke<PairedClient[]>("server_clients", { server }),
  serverRevoke: (server: string, client: string) => invoke<void>("server_revoke", { server, client }),
  /** Renames the server in this client's list only. */
  serverRename: (server: string, name: string) => invoke<void>("server_rename", { server, name }),
  /** Forgets the server in this client's list only. */
  serverForget: (server: string) => invoke<void>("server_forget", { server }),
  /** A null swatch adds the workspace with the auto color. */
  workspaceAdd: (server: string, name: string, root: string, swatch: Swatch | null) =>
    invoke<void>("workspace_add", { server, name, root, swatch }),
  autoColor: (name: string) => invoke<Swatch>("auto_color", { name }),
  workspaceRemove: (server: string, workspace: WorkspaceId) => invoke<void>("workspace_remove", { server, workspace }),
  /** Renames the section's workspace on every server of the section. */
  sectionRename: (section: string, name: string) => invoke<void>("section_rename", { section, name }),
  /** Colors the section's workspace on every server of the section; a null swatch sets auto. */
  sectionColor: (section: string, swatch: Swatch | null) => invoke<void>("section_color", { section, swatch }),
  taskFocus: (at: TaskRef) => invoke<void>("task_focus", { at }),
  taskMove: (at: TaskRef, state: TaskState) => invoke<void>("task_move", { at, state }),
  taskRename: (at: TaskRef, title: string) => invoke<void>("task_rename", { at, title }),
  taskArchive: (at: TaskRef) => invoke<void>("task_archive", { at }),
  /** Resolves to the created task's key. */
  taskCreate: (task: NewTask) => invoke<string>("task_create", { task }),
  termAttach: (at: TaskRef, pty: PtyId, output: Channel<ArrayBuffer>) =>
    invoke<number>("term_attach", { at, pty, output }),
  termInput: (attachment: number, keys: string) => invoke<void>("term_input", { attachment, keys }),
  termDetach: (attachment: number) => invoke<void>("term_detach", { attachment }),
  /** Resolves to the id of the shell panel it opens. */
  termOpen: (at: TaskRef) => invoke<PtyId>("term_open", { at }),
  termClose: (at: TaskRef, panel: PtyId) => invoke<void>("term_close", { at, panel }),
  termClaim: (at: TaskRef, panel: PtyId, cols: number, rows: number) =>
    invoke<void>("term_claim", { at, panel, cols, rows }),
  termTakeLock: (at: TaskRef, panel: PtyId, cols: number, rows: number) =>
    invoke<void>("term_take_lock", { at, panel, cols, rows }),
  termResize: (at: TaskRef, panel: PtyId, cols: number, rows: number) =>
    invoke<void>("term_resize", { at, panel, cols, rows }),
  fileList: (at: TaskRef, path: string) => invoke<Entry[]>("file_list", { at, path }),
  fileRead: (at: TaskRef, path: string) => invoke<string>("file_read", { at, path }),
  fileWrite: (at: TaskRef, path: string, content: string) => invoke<void>("file_write", { at, path, content }),
  fileSearch: (at: TaskRef, pattern: string) => invoke<Match[]>("file_search", { at, pattern }),
  /** Resolves to the id of the panel that shows `url` on the task's server. */
  browserOpen: (at: TaskRef, panel: BrowserId | typeof NEW_BROWSER_PANEL, url: string) =>
    invoke<BrowserId>("browser_open", { at, panel, url }),
  /** Resolves to the port the page reaches through a tunnel, if it does. */
  browserLoad: (at: TaskRef, panel: BrowserId, url: string) =>
    invoke<number | null>("browser_load", { at, panel, url }),
  browserShowPage: (onScreen: OnScreen | null) => invoke<void>("browser_show_page", { onScreen }),
  browserGo: (at: TaskRef, panel: BrowserId, go: Go) => invoke<void>("browser_go", { at, panel, go }),
  /** Closes the panel's webview on this client; the panel stays on its server. */
  browserClose: (at: TaskRef, panel: BrowserId) => invoke<void>("browser_close", { at, panel }),
};

export const events = {
  model: (handle: (model: Model) => void) => listen<Model>(EVENTS.model, (event) => handle(event.payload)),
  servers: (handle: (servers: ServerView[]) => void) =>
    listen<ServerView[]>(EVENTS.servers, (event) => handle(event.payload)),
  filesChanged: (handle: (changed: FilesChanged[]) => void) =>
    listen<FilesChanged[]>(EVENTS.filesChanged, (event) => handle(event.payload)),
  browserNavigated: (handle: (navigated: Navigated) => void) =>
    listen<Navigated>(EVENTS.browserNavigated, (event) => handle(event.payload)),
  ui: (handle: (ui: Ui) => void) => listen<Ui>(EVENTS.ui, (event) => handle(event.payload)),
  windowShown: (handle: (shown: boolean) => void) =>
    listen<boolean>(EVENTS.windowShown, (event) => handle(event.payload)),
};

export { Channel };
