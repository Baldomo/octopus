import { createStore, produce, type SetStoreFunction } from "solid-js/store";
import { backend } from "./backend";
import type { Entry, FilesChanged, TaskRef, TaskView } from "./generated";
import { report } from "./state";

type Monaco = typeof import("./monaco");
type Model = import("monaco-editor").editor.ITextModel;
type ViewState = import("monaco-editor").editor.ICodeEditorViewState | null;

let loading: Promise<Monaco> | undefined;
/** Monaco loads on the first file opened, off the launch path. */
export const loadMonaco = () => (loading ??= import("./monaco"));

export const join = (dir: string, name: string) => (dir === "." ? name : `${dir}/${name}`);
const parentOf = (path: string) => (path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : ".");
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

type Opened = { model: Model; saved: number; view: ViewState };

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
};

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
    });
    this.list(".");
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
    const uri = monaco.Uri.from({ scheme: "octopus", path: `/${this.key}/${path}` });
    const model = monaco.editor.createModel(text, undefined, uri);
    const opened = { model, saved: model.getAlternativeVersionId(), view: null };
    model.onDidChangeContent(() => this.set("dirty", path, model.getAlternativeVersionId() !== opened.saved));
    this.opened.set(path, opened);
    this.set(
      produce((state) => {
        delete state.unreadable[path];
        state.loaded[path] = true;
      }),
    );
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
    for (const dir of new Set(paths.map(parentOf))) {
      if (this.state.listings[dir] !== undefined) this.list(dir);
    }
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
