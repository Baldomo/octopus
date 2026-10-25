import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { EVENTS } from "./generated";
import type {
  Boot,
  Bounds,
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
  serverAdd: (name: string, socket: string) => invoke<void>("server_add", { name, socket }),
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
  browserOpen: (panel: string, url: string) => invoke<void>("browser_open", { panel, url }),
  browserPlace: (panel: string, bounds: Bounds | null) => invoke<void>("browser_place", { panel, bounds }),
  browserGo: (panel: string, go: Go) => invoke<void>("browser_go", { panel, go }),
  browserClose: (panel: string) => invoke<void>("browser_close", { panel }),
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
