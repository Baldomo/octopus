import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { EVENTS } from "./generated";
import type {
  Boot,
  Entry,
  FilesChanged,
  Match,
  Model,
  NewTask,
  ShownOffer,
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

/** Every command of the client app's core, which the app adds as its `octopus` plugin. */
const octopus = (command: string) => `plugin:octopus|${command}`;

const UTF8 = new TextDecoder("utf-8", { fatal: true });
const fileBytes = (at: TaskRef, path: string) => invoke<ArrayBuffer>(octopus("file_read"), { at, path });

export const backend = {
  boot: () => invoke<Boot>(octopus("boot")),
  ready: () => invoke<void>(octopus("ready")),
  uiAct: (act: UiAct) => invoke<void>(octopus("ui_act"), { act }),
  quit: () => invoke<void>(octopus("quit")),
  /** Resolves to the paired server's name. */
  serverPair: (invite: string) => invoke<string>(octopus("server_pair"), { invite }),
  serverOffer: (server: string) => invoke<ShownOffer>(octopus("server_offer"), { server }),
  serverClients: (server: string) => invoke<PairedClient[]>(octopus("server_clients"), { server }),
  serverRevoke: (server: string, client: string) => invoke<void>(octopus("server_revoke"), { server, client }),
  /** Renames the server in this client's list only. */
  serverRename: (server: string, name: string) => invoke<void>(octopus("server_rename"), { server, name }),
  /** Forgets the server in this client's list only. */
  serverForget: (server: string) => invoke<void>(octopus("server_forget"), { server }),
  /** A null swatch adds the workspace with the auto color. */
  workspaceAdd: (server: string, name: string, root: string, swatch: Swatch | null) =>
    invoke<void>(octopus("workspace_add"), { server, name, root, swatch }),
  autoColor: (name: string) => invoke<Swatch>(octopus("auto_color"), { name }),
  workspaceRemove: (server: string, workspace: WorkspaceId) =>
    invoke<void>(octopus("workspace_remove"), { server, workspace }),
  /** Renames the section's workspace on every server of the section. */
  sectionRename: (section: string, name: string) => invoke<void>(octopus("section_rename"), { section, name }),
  /** Colors the section's workspace on every server of the section; a null swatch sets auto. */
  sectionColor: (section: string, swatch: Swatch | null) => invoke<void>(octopus("section_color"), { section, swatch }),
  taskFocus: (at: TaskRef) => invoke<void>(octopus("task_focus"), { at }),
  taskMove: (at: TaskRef, state: TaskState) => invoke<void>(octopus("task_move"), { at, state }),
  taskRename: (at: TaskRef, title: string) => invoke<void>(octopus("task_rename"), { at, title }),
  taskArchive: (at: TaskRef) => invoke<void>(octopus("task_archive"), { at }),
  /** Resolves to the created task's key. */
  taskCreate: (task: NewTask) => invoke<string>(octopus("task_create"), { task }),
  termAttach: (at: TaskRef, pty: PtyId, output: Channel<ArrayBuffer>) =>
    invoke<number>(octopus("term_attach"), { at, pty, output }),
  termInput: (attachment: number, keys: string) => invoke<void>(octopus("term_input"), { attachment, keys }),
  termDetach: (attachment: number) => invoke<void>(octopus("term_detach"), { attachment }),
  /** Resolves to the id of the shell panel it opens. */
  termOpen: (at: TaskRef) => invoke<PtyId>(octopus("term_open"), { at }),
  termClose: (at: TaskRef, panel: PtyId) => invoke<void>(octopus("term_close"), { at, panel }),
  termClaim: (at: TaskRef, panel: PtyId, cols: number, rows: number) =>
    invoke<void>(octopus("term_claim"), { at, panel, cols, rows }),
  termTakeLock: (at: TaskRef, panel: PtyId, cols: number, rows: number) =>
    invoke<void>(octopus("term_take_lock"), { at, panel, cols, rows }),
  termResize: (at: TaskRef, panel: PtyId, cols: number, rows: number) =>
    invoke<void>(octopus("term_resize"), { at, panel, cols, rows }),
  fileList: (at: TaskRef, path: string) => invoke<Entry[]>(octopus("file_list"), { at, path }),
  fileBytes,
  fileText: (at: TaskRef, path: string) =>
    fileBytes(at, path).then((bytes) => {
      try {
        return UTF8.decode(bytes);
      } catch {
        throw new Error(`${path} is not UTF-8 text`);
      }
    }),
  fileWrite: (at: TaskRef, path: string, content: string) => invoke<void>(octopus("file_write"), { at, path, content }),
  fileSearch: (at: TaskRef, pattern: string) => invoke<Match[]>(octopus("file_search"), { at, pattern }),
};

export const events = {
  model: (handle: (model: Model) => void) => listen<Model>(EVENTS.model, (event) => handle(event.payload)),
  servers: (handle: (servers: ServerView[]) => void) =>
    listen<ServerView[]>(EVENTS.servers, (event) => handle(event.payload)),
  filesChanged: (handle: (changed: FilesChanged[]) => void) =>
    listen<FilesChanged[]>(EVENTS.filesChanged, (event) => handle(event.payload)),
  ui: (handle: (ui: Ui) => void) => listen<Ui>(EVENTS.ui, (event) => handle(event.payload)),
  windowShown: (handle: (shown: boolean) => void) =>
    listen<boolean>(EVENTS.windowShown, (event) => handle(event.payload)),
};

export { Channel };
