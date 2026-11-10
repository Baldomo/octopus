import { createStore, produce, type SetStoreFunction } from "solid-js/store";
import { backend } from "./backend";
import type { Entry, EntryKind, FilesChanged, TaskRef, TaskView } from "./generated";
import { moveTo } from "./sectionOrder";
import { isLocal, report } from "./state";
import { each } from "./Toast";

type Monaco = typeof import("./monaco");
type Model = import("monaco-editor").editor.ITextModel;
type ViewState = import("monaco-editor").editor.ICodeEditorViewState | null;

let loading: Promise<Monaco> | undefined;
/** Numbers each buffer's URI, which outlives its file's path when the file moves and a new one takes the path. */
let buffers = 0;
/** Monaco loads on the first file opened, off the launch path. */
export const loadMonaco = () => (loading ??= import("./monaco"));

export const join = (dir: string, name: string) => (dir === "." ? name : `${dir}/${name}`);
export const parentOf = (path: string) => (path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : ".");
export const nameOf = (path: string) => path.slice(path.lastIndexOf("/") + 1);

const dirsOf = (path: string) => path.split("/").slice(0, -1);
const tail = (dirs: string[], depth: number) => dirs.slice(-depth).join("/");

export function tabHints(tabs: string[]) {
  const hints: Record<string, string> = {};
  for (const path of tabs) {
    const rivals = tabs.filter((tab) => tab !== path && nameOf(tab) === nameOf(path)).map(dirsOf);
    if (rivals.length === 0) continue;
    const dirs = dirsOf(path);
    let depth = 1;
    while (depth < dirs.length && rivals.some((rival) => tail(rival, depth) === tail(dirs, depth))) depth++;
    hints[path] = dirs.length === 0 ? "." : tail(dirs, depth);
  }
  return hints;
}

/** `path` follows the file through moves, so its edits stay marked under its current name. */
type Opened = { path: string; model: Model; saved: number; view: ViewState };

type Tree = {
  listings: Record<string, Entry[]>;
  expanded: Record<string, boolean>;
  tabs: string[];
  active: string | null;
  loaded: Record<string, boolean>;
  dirty: Record<string, boolean>;
  unreadable: Record<string, string>;
  rendered: Record<string, boolean>;
  revealing: string | null;
  selected: string[];
  /** The row the keyboard moves from, and the far end of a Shift range. */
  focus: string | null;
  anchor: string | null;
  clip: { paths: string[]; cut: boolean } | null;
};

export const within = (path: string, dir: string) => dir === "." || path === dir || path.startsWith(`${dir}/`);

export function freeName(name: string, taken: Set<string>) {
  const dot = name.lastIndexOf(".");
  const [stem, extension] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ""];
  const nth = (count: number) => (count === 0 ? name : `${stem} copy${count === 1 ? "" : ` ${count}`}${extension}`);
  let count = 0;
  while (taken.has(nth(count))) count++;
  return nth(count);
}

export const extensionOf = (path: string) => {
  const name = nameOf(path);
  return name.includes(".") ? name.slice(name.lastIndexOf(".") + 1).toLowerCase() : "";
};
const RENDERINGS = new Map<string, "markdown" | "page" | "image">([
  ["md", "markdown"],
  ["markdown", "markdown"],
  ["html", "page"],
  ["htm", "page"],
  ["svg", "image"],
]);
export const renderingOf = (path: string) => RENDERINGS.get(extensionOf(path));
export const renderable = (path: string) => renderingOf(path) !== undefined;
const ancestorsOf = (path: string) =>
  path
    .split("/")
    .slice(0, -1)
    .map((_, at, dirs) => dirs.slice(0, at + 1).join("/"));

/** A file or folder dropped from this machine, by its path, or a file this client holds, as a paste brings it. */
export type Incoming = string | File;

export class TaskFiles {
  readonly state: Tree;
  private readonly set: SetStoreFunction<Tree>;
  private readonly opened = new Map<string, Opened>();

  constructor(
    readonly key: string,
    readonly at: TaskRef,
  ) {
    [this.state, this.set] = createStore<Tree>({
      listings: {},
      expanded: {},
      tabs: [],
      active: null,
      loaded: {},
      dirty: {},
      unreadable: {},
      rendered: {},
      revealing: null,
      selected: [],
      focus: null,
      anchor: null,
      clip: null,
    });
    this.list(".");
  }

  select(paths: string[], focus: string | null, anchor = focus) {
    this.set({ selected: paths, focus, anchor });
  }

  setClip(clip: Tree["clip"]) {
    this.set("clip", clip);
  }

  /** Lists again the folders holding `paths` that the tree shows, ahead of the server's change events. */
  private relist(...paths: string[]) {
    for (const dir of new Set(paths.map(parentOf))) if (this.state.listings[dir] !== undefined) this.list(dir);
  }

  /** Names in `dir` as the server holds them now, which a listing on screen may trail. */
  async names(dir: string) {
    return new Set((await backend.fileList(this.at, dir)).map((entry) => entry.name));
  }

  async create(path: string, kind: EntryKind) {
    await backend.fileCreate(this.at, path, kind);
    this.relist(path);
    this.reveal(path, kind === "directory");
    this.select([path], path);
    if (kind === "file") await this.open(path);
  }

  async move(from: string, to: string) {
    await backend.fileMove(this.at, from, to);
    this.relist(from, to);
    const moved = (path: string) => (within(path, from) ? to + path.slice(from.length) : path);
    const rekey = <T>(record: Record<string, T>) =>
      Object.fromEntries(Object.entries(record).map(([path, value]) => [moved(path), value]));
    const opened = [...this.opened.values()];
    this.opened.clear();
    for (const buffer of opened) {
      buffer.path = moved(buffer.path);
      this.opened.set(buffer.path, buffer);
    }
    this.set(
      produce((state) => {
        state.tabs = state.tabs.map(moved);
        state.active = state.active === null ? null : moved(state.active);
        state.listings = rekey(state.listings);
        state.expanded = rekey(state.expanded);
        state.loaded = rekey(state.loaded);
        state.dirty = rekey(state.dirty);
        state.unreadable = rekey(state.unreadable);
        state.rendered = rekey(state.rendered);
        state.selected = state.selected.map(moved);
        state.focus = state.focus === null ? null : moved(state.focus);
        state.anchor = state.anchor === null ? null : moved(state.anchor);
      }),
    );
  }

  copy(from: string, to: string) {
    return backend.fileCopy(this.at, from, to).then(() => this.relist(to));
  }

  /** Closes the entry's clean tabs; a dirty one keeps its edits. */
  async remove(path: string, trash: boolean) {
    await backend.fileDelete(this.at, path, trash);
    this.relist(path);
    for (const tab of this.state.tabs.filter((tab) => within(tab, path) && !this.state.dirty[tab])) this.close(tab);
    this.set("selected", (selected) => selected.filter((selected) => !within(selected, path)));
  }

  /** Copies files and folders into `dir`, each under a name `dir` lacks; resolves to the copies, and says why each
   * other one failed. */
  async bringIn(incoming: Incoming[], dir: string) {
    const taken = await this.names(dir);
    const copies: string[] = [];
    await each(incoming, async (item) => {
      const name = freeName(typeof item === "string" ? nameOf(item) : item.name, taken);
      taken.add(name);
      const copy = join(dir, name);
      const sent =
        typeof item === "string"
          ? backend.fileUpload(this.at, copy, item)
          : item.arrayBuffer().then((content) => backend.fileUploadContent(this.at, copy, new Uint8Array(content)));
      await sent.finally(() => this.relist(copy));
      copies.push(copy);
    });
    return copies;
  }

  collapseAll() {
    this.set("expanded", {});
  }

  list(dir: string) {
    backend.fileList(this.at, dir).then((entries) => this.set("listings", dir, entries), report);
  }

  toggle(dir: string) {
    this.revealed();
    this.expand(dir, !this.state.expanded[dir]);
  }

  private expand(dir: string, expanding: boolean) {
    this.set("expanded", dir, expanding);
    if (expanding && this.state.listings[dir] === undefined) this.list(dir);
  }

  reveal(path: string, directory: boolean) {
    for (const dir of directory ? [...ancestorsOf(path), path] : ancestorsOf(path)) this.expand(dir, true);
    this.set("revealing", path);
  }

  revealed() {
    this.set("revealing", null);
  }

  toggleRendered() {
    const path = this.state.active;
    if (path !== null && renderable(path)) this.set("rendered", path, !this.state.rendered[path]);
  }

  async open(path: string) {
    if (!this.state.tabs.includes(path)) this.set("tabs", (tabs) => [...tabs, path]);
    this.set("active", path);
    if (this.opened.has(path)) return;
    const [{ monaco }, text] = await Promise.all([loadMonaco(), backend.fileText(this.at, path)]).catch((error) => {
      this.set("unreadable", path, String(error));
      throw error;
    });
    if (this.opened.has(path) || !this.state.tabs.includes(path)) return;
    const uri = monaco.Uri.from({ scheme: "octopus", path: `/${this.key}/${++buffers}/${nameOf(path)}` });
    const model = monaco.editor.createModel(text, undefined, uri);
    const opened: Opened = { path, model, saved: model.getAlternativeVersionId(), view: null };
    model.onDidChangeContent(() => this.set("dirty", opened.path, model.getAlternativeVersionId() !== opened.saved));
    this.opened.set(path, opened);
    this.set(
      produce((state) => {
        delete state.unreadable[path];
        state.loaded[path] = true;
      }),
    );
  }

  moveTab(path: string, to: number) {
    this.set("tabs", (tabs) => moveTo(tabs, path, to));
  }

  /** A dirty buffer outlives its tab, so reopening the file brings its edits back. */
  close(path: string) {
    this.set(
      produce((state) => {
        const at = state.tabs.indexOf(path);
        state.tabs = state.tabs.filter((tab) => tab !== path);
        delete state.rendered[path];
        if (state.active === path) state.active = state.tabs[Math.min(at, state.tabs.length - 1)] ?? null;
      }),
    );
    if (this.state.dirty[path]) return;
    this.opened.get(path)?.model.dispose();
    this.opened.delete(path);
    this.set(
      produce((state) => {
        delete state.dirty[path];
        delete state.loaded[path];
      }),
    );
  }

  buffer(path: string) {
    return this.opened.get(path);
  }

  get dirty() {
    return Object.values(this.state.dirty).some(Boolean);
  }

  save(path: string) {
    const opened = this.opened.get(path);
    if (opened === undefined) return;
    const version = opened.model.getAlternativeVersionId();
    backend.fileWrite(this.at, path, opened.model.getValue()).then(() => {
      opened.saved = version;
      this.set("dirty", path, opened.model.getAlternativeVersionId() !== version);
    }, report);
  }

  /** A clean buffer follows the file on the server; a dirty one keeps the user's edits. */
  changed(paths: string[]) {
    this.relist(...paths);
    for (const path of paths) {
      const opened = this.opened.get(path);
      if (opened === undefined || this.state.dirty[path]) continue;
      backend.fileText(this.at, path).then(
        (text) => {
          this.set(produce((state) => delete state.unreadable[path]));
          if (this.state.dirty[path] || opened.model.getValue() === text) return;
          opened.model.setValue(text);
          opened.saved = opened.model.getAlternativeVersionId();
          this.set("dirty", path, false);
        },
        (error) => this.set("unreadable", path, String(error)),
      );
    }
  }

  dispose() {
    for (const opened of this.opened.values()) opened.model.dispose();
    this.opened.clear();
  }
}

const files = new Map<string, TaskFiles>();

export function filesOf(task: TaskView) {
  let found = files.get(task.key);
  if (found === undefined) {
    found = new TaskFiles(task.key, task.at);
    files.set(task.key, found);
  }
  return found;
}

/** Where files are on the task's server: dropped ones are there already when it runs here; anything else is first
 * copied into the checkout's root. */
export async function onServer(task: TaskView, incoming: Incoming[]) {
  if (isLocal(task.at.server) && incoming.every((item) => typeof item === "string")) return incoming;
  const copies = await filesOf(task).bringIn(incoming, ".");
  return copies.map((copy) => `${task.task.directory}/${copy}`);
}

/** Unsaved edits outlive their task leaving the recent ones. */
export function forgetFilesExcept(keep: string[]) {
  for (const [key, taskFiles] of files) {
    if (keep.includes(key) || taskFiles.dirty) continue;
    taskFiles.dispose();
    files.delete(key);
  }
}

export function applyFilesChanged(changed: FilesChanged[]) {
  for (const change of changed) files.get(change.key)?.changed(change.paths);
}
