import { createStore, produce, type SetStoreFunction } from "solid-js/store";
import { backend } from "./backend";
import type { Entry, EntryKind, FilesChanged, TaskRef, TaskView } from "./generated";
import { moveTo } from "./sectionOrder";
import { report } from "./state";

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

/** For each open path whose name another tab shares, the fewest trailing directories that tell it apart. */
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
  /** The tree row to scroll into view once it shows. */
  revealing: string | null;
  /** The tree's selected entries, which its actions act on. */
  selected: string[];
  /** The row the keyboard moves from, and the far end of a Shift range. */
  focus: string | null;
  /** The near end of a Shift range. */
  anchor: string | null;
  /** What the next paste copies, or moves when it was cut. */
  clip: { paths: string[]; cut: boolean } | null;
};

/** Whether `path` is `dir` or lies inside it. */
export const within = (path: string, dir: string) => dir === "." || path === dir || path.startsWith(`${dir}/`);

/** `name`, or the first of "name copy", "name copy 2", … that `taken` lacks, its extension kept at the end. */
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

/** One task's file tree and editor tabs, kept while the task stays among the recent ones. */
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

  /** Selects `paths`, the keyboard at `focus` and a Shift range reaching from `anchor`. */
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

  /** Moves the entry, its open tabs and its place in the tree along with it. */
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

  upload(path: string, bytes: ArrayBuffer) {
    return backend.fileUpload(this.at, path, bytes).then(() => this.relist(path));
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

  /** Expands the tree down to `path`, and `path` itself when it is a directory, and scrolls its row into view. */
  reveal(path: string, directory: boolean) {
    for (const dir of directory ? [...ancestorsOf(path), path] : ancestorsOf(path)) this.expand(dir, true);
    this.set("revealing", path);
  }

  revealed() {
    this.set("revealing", null);
  }

  /** Flips the selected tab between its code and its rendered view. */
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
