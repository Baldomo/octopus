import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { COMMANDS, EVENTS } from "./generated";
import type {
  AccountView,
  Boot,
  Entry,
  EntryKind,
  FileDrag,
  FilesChanged,
  Match,
  Model,
  NewTask,
  ShownLink,
  ShownOffer,
  PairedAccount,
  PtyId,
  ServerView,
  Setting,
  Swatch,
  TaskRef,
  TaskState,
  TermScreen,
  Ui,
  UiAct,
  WorkspaceId,
} from "./generated";

const octopus = (command: (typeof COMMANDS)[number]) => `plugin:octopus|${command}`;

export const backend = {
  boot: () => invoke<Boot>(octopus("boot")),
  ready: () => invoke<void>(octopus("ready")),
  uiAct: (act: UiAct) => invoke<void>(octopus("ui_act"), { act }),
  quit: () => invoke<void>(octopus("quit")),
  /** Resolves to the paired server's name. */
  serverPair: (invite: string) => invoke<string>(octopus("server_pair"), { invite }),
  serverOffer: (server: string) => invoke<ShownOffer>(octopus("server_offer"), { server }),
  serverAccounts: (server: string) => invoke<PairedAccount[]>(octopus("server_accounts"), { server }),
  /** Stops the server trusting the account, which is its id in full. */
  serverRevoke: (server: string, account: string) => invoke<void>(octopus("server_revoke"), { server, account }),
  /** A fresh link to this account, held by a reachable server for 10 minutes. */
  accountLink: () => invoke<ShownLink>(octopus("account_link")),
  /** Joins the account the link names, bringing this client's servers along. */
  accountJoin: (link: string) => invoke<void>(octopus("account_join"), { link }),
  /** Removes the device whose key, in hex, is `device`. */
  accountRemove: (device: string) => invoke<void>(octopus("account_remove"), { device }),
  accountLeave: () => invoke<void>(octopus("account_leave")),
  /** Sets the setting on every device of the account, resolving to the account as it then reads. */
  settingsSet: (setting: Setting) => invoke<AccountView>(octopus("settings_set"), { setting }),
  accountStartOver: () => invoke<void>(octopus("account_start_over")),
  /** Resyncs every connected server, rejecting with each one's problem. */
  serversRefresh: () => invoke<void>(octopus("servers_refresh")),
  /** Renames the server on every device of the account. */
  serverRename: (server: string, name: string) => invoke<void>(octopus("server_rename"), { server, name }),
  /** Forgets the server on every device of the account. */
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
  taskResume: (at: TaskRef) => invoke<void>(octopus("task_resume"), { at }),
  taskMove: (at: TaskRef, state: TaskState) => invoke<void>(octopus("task_move"), { at, state }),
  taskSetParent: (at: TaskRef, parent: TaskRef | null) => invoke<void>(octopus("task_set_parent"), { at, parent }),
  taskRename: (at: TaskRef, title: string) => invoke<void>(octopus("task_rename"), { at, title }),
  taskArchive: (at: TaskRef) => invoke<void>(octopus("task_archive"), { at }),
  /** Resolves to the created task's key. */
  taskCreate: (task: NewTask) => invoke<string>(octopus("task_create"), { task }),
  termAttach: (at: TaskRef, pty: PtyId, output: Channel<TermScreen | ArrayBuffer>) =>
    invoke<number>(octopus("term_attach"), { at, pty, output }),
  termInput: (attachment: number, keys: string) => invoke<void>(octopus("term_input"), { attachment, keys }),
  termDetach: (attachment: number) => invoke<void>(octopus("term_detach"), { attachment }),
  /** Resolves to the id of the shell panel it opens. */
  termOpen: (at: TaskRef, directory: string) => invoke<PtyId>(octopus("term_open"), { at, directory }),
  termClose: (at: TaskRef, panel: PtyId) => invoke<void>(octopus("term_close"), { at, panel }),
  termClaim: (at: TaskRef, panel: PtyId, cols: number, rows: number) =>
    invoke<void>(octopus("term_claim"), { at, panel, cols, rows }),
  termTakeLock: (at: TaskRef, panel: PtyId, cols: number, rows: number) =>
    invoke<void>(octopus("term_take_lock"), { at, panel, cols, rows }),
  termResize: (at: TaskRef, panel: PtyId, cols: number, rows: number) =>
    invoke<void>(octopus("term_resize"), { at, panel, cols, rows }),
  fileList: (at: TaskRef, path: string) => invoke<Entry[]>(octopus("file_list"), { at, path }),
  /** The URL each file of the checkout loads under, at its path with each part percent-encoded. */
  fileRootUrl: (at: TaskRef) => invoke<string>(octopus("file_root_url"), { at }),
  fileWrite: (at: TaskRef, path: string, content: string) => invoke<void>(octopus("file_write"), { at, path, content }),
  fileCreate: (at: TaskRef, path: string, kind: EntryKind) => invoke<void>(octopus("file_create"), { at, path, kind }),
  fileMove: (at: TaskRef, from: string, to: string) => invoke<void>(octopus("file_move"), { at, from, to }),
  fileCopy: (at: TaskRef, from: string, to: string) => invoke<void>(octopus("file_copy"), { at, from, to }),
  /** `trash` sends it to the trash of the server's machine; otherwise it is gone for good. */
  fileDelete: (at: TaskRef, path: string, trash: boolean) => invoke<void>(octopus("file_delete"), { at, path, trash }),
  /** Copies `local`, a file or folder on this machine, to the new `path`. */
  fileUpload: (at: TaskRef, path: string, local: string) => invoke<void>(octopus("file_upload"), { at, path, local }),
  /** Sends `content`, a file this client holds, as the new `path`; the IPC carries the bytes as a JSON array. */
  fileUploadContent: (at: TaskRef, path: string, content: Uint8Array) =>
    invoke<void>(octopus("file_upload_content"), { at, path, content }),
  fileSearch: (at: TaskRef, pattern: string) => invoke<Match[]>(octopus("file_search"), { at, pattern }),
};

export const on =
  <T>(event: string) =>
  (handle: (payload: T) => void) =>
    listen<T>(event, (heard) => handle(heard.payload));

export const events = {
  model: on<Model>(EVENTS.model),
  servers: on<ServerView[]>(EVENTS.servers),
  account: on<AccountView>(EVENTS.account),
  filesChanged: on<FilesChanged[]>(EVENTS.filesChanged),
  ui: on<Ui>(EVENTS.ui),
  windowShown: on<boolean>(EVENTS.windowShown),
  windowFocused: on<void>(EVENTS.windowFocused),
  fileDrag: on<FileDrag>(EVENTS.fileDrag),
  microphoneDenied: on<boolean>(EVENTS.microphoneDenied),
};

export { Channel };
