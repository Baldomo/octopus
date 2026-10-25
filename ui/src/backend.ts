import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { EVENTS, NEW_BROWSER_PANEL } from "./generated";
import type {
  Boot,
  Bounds,
  BrowserId,
  Entry,
  FilesChanged,
  Go,
  Match,
  Model,
  Navigated,
  NewTask,
  PtyId,
  ServerView,
  TaskRef,
  TaskState,
} from "./generated";

export const backend = {
  boot: () => invoke<Boot>("boot"),
  ready: () => invoke<void>("ready"),
  /** Resolves to the paired server's name. */
  serverPair: (address: string, code: string) => invoke<string>("server_pair", { address, code }),
  taskFocus: (at: TaskRef) => invoke<void>("task_focus", { at }),
  taskMove: (at: TaskRef, state: TaskState) => invoke<void>("task_move", { at, state }),
  /** Resolves to the created task's key. */
  taskCreate: (task: NewTask) => invoke<string>("task_create", { task }),
  termAttach: (at: TaskRef, pty: PtyId, output: Channel<ArrayBuffer>) =>
    invoke<number>("term_attach", { at, pty, output }),
  termInput: (attachment: number, keys: string) => invoke<void>("term_input", { attachment, keys }),
  termDetach: (attachment: number) => invoke<void>("term_detach", { attachment }),
  termTakeLock: (at: TaskRef, panel: PtyId) => invoke<void>("term_take_lock", { at, panel }),
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
  browserShow: (at: TaskRef, panel: BrowserId, url: string) =>
    invoke<number | null>("browser_show", { at, panel, url }),
  browserPlace: (at: TaskRef, panel: BrowserId, bounds: Bounds | null) =>
    invoke<void>("browser_place", { at, panel, bounds }),
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
};

export { Channel };
