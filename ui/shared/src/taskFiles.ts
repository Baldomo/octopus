import { createStore, produce, type SetStoreFunction } from "solid-js/store";
import { backend } from "./backend";
import type { Entry, EntryKind, FilesChanged, TaskRef, TaskView } from "./generated";
import { fetched, openingOf, renderingOf, SNIFF_BYTES, textOf, type MediaKind } from "./opening";
import { moveTo } from "./sectionOrder";
import { isLocal, report } from "./state";
import { each } from "./Toast";

type Monaco = typeof import("./monaco");
type Model = import("monaco-editor").editor.ITextModel;
type ViewState = import("monaco-editor").editor.ICodeEditorViewState | null;

let loading: Promise<Monaco> | undefined;
/** A file read to show it, with its text and the editor's module when it shows as code. */
type Read = { view: FileView; code?: { text: string; monaco: Monaco["monaco"] } };

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

/** How an open file shows: as code in its buffer, as media, or as a line naming it with its size. A media file's
 * `load` changes with each change to the file, so its view loads it again. */
export type MediaView = { as: MediaKind; load: number; size: number } | { as: "binary" | "large"; size: number };
export type FileView = { as: "text" } | MediaView;

/** Numbers each load of a media file, so the webview never reuses an earlier one. */
let loads = 0;

type Tree = {
  listings: Record<string, Entry[]>;
  expanded: Record<string, boolean>;
  tabs: string[];
  active: string | null;
  views: Record<string, FileView>;
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
  /** The URL the task's files load under, which the first read asks this client for. */
  private root: string | undefined;
  /** Counts the reads of each file, so a read that a newer read or a save overtook shows nothing. */
  private readonly reads = new Map<string, number>();

  constructor(
    readonly key: string,
    readonly at: TaskRef,
  ) {
    [this.state, this.set] = createStore<Tree>({
      listings: {},
      expanded: {},
      tabs: [],
      active: null,
      views: {},
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
    const reads = [...this.reads];
    this.reads.clear();
    for (const [path, nth] of reads) this.reads.set(moved(path), nth);
    this.set(
      produce((state) => {
        state.tabs = state.tabs.map(moved);
        state.active = state.active === null ? null : moved(state.active);
        state.listings = rekey(state.listings);
        state.expanded = rekey(state.expanded);
        state.views = rekey(state.views);
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

  /** Whether the file shows as code or rendered, as a text file with a rendering does. */
  switchable(path: string) {
    return renderingOf(path) !== undefined && this.state.views[path]?.as === "text";
  }

  toggleRendered() {
    const path = this.state.active;
    if (path !== null && this.switchable(path)) this.set("rendered", path, !this.state.rendered[path]);
  }

  async open(path: string) {
    if (!this.state.tabs.includes(path)) this.set("tabs", (tabs) => [...tabs, path]);
    this.set("active", path);
    if (this.state.views[path] !== undefined) return;
    const nth = this.overtake(path);
    const read = await this.read(path).catch((error: unknown) => {
      this.set("unreadable", path, String(error));
      throw error;
    });
    if (this.reads.get(path) === nth && this.state.views[path] === undefined && this.state.tabs.includes(path))
      this.show(path, read);
  }

  /** Decides how the file shows, reading its text when it shows as code. */
  private async read(path: string): Promise<Read> {
    this.root ??= await backend.fileRootUrl(this.at);
    const url = this.urlOf(path);
    const head = await fetched(url, `bytes=0-${SNIFF_BYTES - 1}`);
    const opening = openingOf(path, head.size, head.bytes);
    if (opening === "binary" || opening === "large") return { view: { as: opening, size: head.size } };
    if (opening !== "text") {
      return { view: { as: opening, load: ++loads, size: head.size } };
    }
    const [whole, { monaco }] = await Promise.all([head.size > head.bytes.length ? fetched(url) : head, loadMonaco()]);
    const text = textOf(whole.bytes);
    return text === undefined
      ? { view: { as: "binary", size: head.size } }
      : { view: { as: "text" }, code: { text, monaco } };
  }

  /** A text file shows in its buffer, made here or set to the text; any other view leaves no buffer. */
  private show(path: string, read: Read) {
    const buffer = this.opened.get(path);
    const code = read.code;
    if (code === undefined) {
      buffer?.model.dispose();
      this.opened.delete(path);
    } else if (buffer === undefined) {
      const { monaco, text } = code;
      const uri = monaco.Uri.from({ scheme: "octopus", path: `/${this.key}/${++buffers}/${nameOf(path)}` });
      const model = monaco.editor.createModel(text, undefined, uri);
      const opened: Opened = { path, model, saved: model.getAlternativeVersionId(), view: null };
      model.onDidChangeContent(() => this.set("dirty", opened.path, model.getAlternativeVersionId() !== opened.saved));
      this.opened.set(path, opened);
    } else if (buffer.model.getValue() !== code.text) {
      buffer.model.setValue(code.text);
      buffer.saved = buffer.model.getAlternativeVersionId();
    }
    this.set(
      produce((state) => {
        delete state.unreadable[path];
        state.views[path] = read.view;
        state.dirty[path] = false;
      }),
    );
  }

  /** The file's view when it shows with no code. */
  media(path: string) {
    const view = this.state.views[path];
    return view === undefined || view.as === "text" ? undefined : view;
  }

  /** Where a file of the task's directory loads from, once a read has named its URL. */
  urlOf(path: string) {
    return `${this.root}/${path.split("/").map(encodeURIComponent).join("/")}`;
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
    this.reads.delete(path);
    this.opened.get(path)?.model.dispose();
    this.opened.delete(path);
    this.set(
      produce((state) => {
        delete state.dirty[path];
        delete state.views[path];
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
    this.overtake(path);
    backend.fileWrite(this.at, path, opened.model.getValue()).then(() => {
      opened.saved = version;
      this.set("dirty", path, opened.model.getAlternativeVersionId() !== version);
    }, report);
  }

  /** An open file follows the file on the server, but for a dirty buffer, which keeps the user's edits. */
  changed(paths: string[]) {
    this.relist(...paths);
    for (const path of paths) {
      if (!this.state.tabs.includes(path) || this.state.dirty[path]) continue;
      const nth = this.overtake(path);
      this.read(path).then(
        (read) => {
          if (this.reads.get(path) === nth && this.state.tabs.includes(path) && !this.state.dirty[path])
            this.show(path, read);
        },
        (error: unknown) => this.reads.get(path) === nth && this.set("unreadable", path, String(error)),
      );
    }
  }

  private overtake(path: string) {
    const nth = (this.reads.get(path) ?? 0) + 1;
    this.reads.set(path, nth);
    return nth;
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
 * copied into the task's directory. */
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
