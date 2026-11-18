import { createComputed, createEffect, createMemo, createRoot, createSignal, on, untrack } from "solid-js";
import { createStore } from "solid-js/store";
import { backend } from "./backend";
import type { Folder } from "./generated";
import type { Toward } from "./motion";

/** A folder completing the path: its name, the path picking it leads to, the path completing into it, and whether it
 * is the top of a git repo. */
type Completion = { name: string; pick: string; into: string; repo: boolean };

/** What the list shows while no folder completes the path: why, and whether the server refused to read the folder. */
type Note = { text: string; refused: boolean };

/** How long a reading stands before showing its folder again asks the server anew, the reading staying meanwhile. */
const FRESH_MS = 10_000;

/** Every folder the server has read this session, by server and folder: the folder, or why it is none. */
const [reads, setReads] = createStore<Record<string, Folder | string>>({});
/** When each folder was last asked for, so one asked for again while fresh, or still on its way, is not. */
const asked = new Map<string, number>();

const keyOf = (server: string, folder: string) => `${server}\n${folder.replace(/(.)\/+$/, "$1")}`;

/** The server's reading of `folder`: the folder, or why it is none; undefined until first read. */
export const readingOf = (server: string, folder: string): Folder | string | undefined => reads[keyOf(server, folder)];

/** Asks `server` to read `folder` unless its reading is fresh or on its way. */
export const readFolder = (server: string, folder: string) =>
  untrack(() => {
    const key = keyOf(server, folder);
    const at = asked.get(key);
    if (at !== undefined && performance.now() - at < FRESH_MS) return;
    asked.set(key, performance.now());
    backend.folderGet(server, folder).then(
      (read) => setReads(key, read),
      (refused) => setReads(key, String(refused)),
    );
  });

/** The start every folder under `root` shares. */
const below = (root: string) => (root === "/" ? "/" : `${root}/`);

const inside = (folder: string, root: string) => root !== "" && folder.startsWith(below(root));

/** `path` as typed in a folder field: absolute when it starts with `/`, else under `root`. */
const absolute = (path: string, root: string) =>
  path.startsWith("/") ? path : path === "" ? root : `${below(root)}${path}`;

/** `folder` as a folder field shows it: under `root` relative to it, elsewhere absolute. */
export const relative = (folder: string, root: string) =>
  folder === root ? "" : inside(folder, root) ? folder.slice(below(root).length) : folder;

/** What a folder field shows dimmed before `path`: the root with its slash while the path is under it. */
export const prefixOf = (path: string, root: string) => (path.startsWith("/") ? "" : below(root));

export const parentOf = (folder: string) => folder.slice(0, folder.lastIndexOf("/")) || "/";

export const baseName = (folder: string) => folder.slice(folder.lastIndexOf("/") + 1) || "/";

/** Where a list showing folder `from` goes to show folder `to`. */
const towardOf = (to: string, from: string): Toward =>
  inside(to, from) ? "deeper" : inside(from, to) ? "back" : "across";

export const repoTag = (folder: Folder) =>
  folder.repo === null ? null : folder.branch === null ? "git" : `git · ${folder.branch}`;

export type FolderPicker = ReturnType<typeof folderPicker>;

/**
 * A folder picker on `server` holding its own path, typed under `root` and opening on the folder `from` names with a
 * slash after it, read through `folder.get` as it changes: the folder it names, and the folders completing its last
 * segment. `choose` hands the folder to `chosen` once the server reads it as a directory and ends the picker, unless
 * `chosen` refuses it with false; `cancel` ends it leaving everything as it was.
 */
export function folderPicker(server: string, root: string, from: string, chosen: (folder: string) => boolean | void) {
  return createRoot((end) => {
    const [path, setPath] = createSignal(from === "" || from.endsWith("/") ? from : `${from}/`);
    const reading = (typed: string) => readingOf(server, absolute(typed, root));
    const peek = (typed: string) => readFolder(server, absolute(typed, root));
    const cut = () => path().lastIndexOf("/") + 1;
    /** The path of the folder whose subfolders complete the path's last segment, and that segment. */
    const listedPath = () => path().slice(0, cut());
    const stem = () => path().slice(cut());
    /** Whether the path can name a folder: its last segment is empty, `.` or `..`, or one its listed folder holds. */
    const possible = () => {
      const listed = reading(listedPath());
      const typedStem = stem().toLowerCase();
      return (
        typeof listed !== "object" ||
        ["", ".", ".."].includes(typedStem) ||
        listed.folders.some((name) => name.toLowerCase() === typedStem)
      );
    };
    createEffect(() => {
      peek(listedPath());
      if (possible()) peek(path());
    });
    /** The folder the path names, once the server has read it as one. */
    const folder = () => {
      const now = reading(path());
      return typeof now === "object" ? now : undefined;
    };
    /**
     * The folder listed, its folders completing the path, `..` first while the last segment is empty, and the note
     * shown while no folder completes it. A folder still being read keeps the last listing.
     */
    const listing = createMemo<{ folder: string; rows: Completion[]; note: Note | null }>(
      (was) => {
        const listed = reading(listedPath());
        if (listed === undefined) return was;
        if (typeof listed !== "object") return { folder: was.folder, rows: [], note: { text: listed, refused: true } };
        const typedStem = stem().toLowerCase();
        const named = listed.folders
          .filter(
            (name) => name.toLowerCase().startsWith(typedStem) && (typedStem.startsWith(".") || !name.startsWith(".")),
          )
          .map((name) => ({ name, pick: `${listedPath()}${name}`, repo: listed.repos.includes(name) }));
        const up =
          typedStem === "" && listed.folder !== "/"
            ? [{ name: "..", pick: relative(parentOf(listed.folder), root), repo: false }]
            : [];
        const rows = [...up, ...named].map((each) => ({
          ...each,
          into: each.pick === "" ? "" : `${each.pick.replace(/\/$/, "")}/`,
        }));
        const why = stem() === "" ? "No folders inside" : `No folder starts with “${stem()}”`;
        return { folder: listed.folder, rows, note: named.length > 0 ? null : { text: why, refused: false } };
      },
      { folder: "", rows: [], note: null },
    );
    const completions = () => listing().rows;
    /** Calls `turn` in the caller's scope as the list goes to another folder, before its rows change. */
    const onTurn = (turn: (toward: Toward) => void) =>
      createComputed(
        on(
          () => listing().folder,
          (to, from) => from && to !== from && turn(towardOf(to, from)),
        ),
      );
    /** Whether the folder to list is still on its way from the server, the last listing standing in for it. */
    const pending = () => reading(listedPath()) === undefined;
    /** The path Tab leads to with no completion chosen: the completions' shared start, or into the only one. */
    const tab = () => {
      const [first, ...rest] = completions().filter((each) => each.name !== "..");
      if (first === undefined) return path();
      if (rest.length === 0) return first.into;
      let shared = first.name;
      for (const { name } of rest)
        while (!name.toLowerCase().startsWith(shared.toLowerCase())) shared = shared.slice(0, -1);
      return shared.length > stem().length ? `${listedPath()}${shared}` : path();
    };
    /** The folder's repo tag, which a path still being read keeps from the last folder read. */
    const tag = createMemo<string | null>((was) => {
      const now = reading(path());
      return now === undefined ? was : typeof now === "object" ? repoTag(now) : null;
    }, null);
    const choose = () => {
      const now = folder();
      if (now === undefined || chosen(now.folder) === false) return false;
      end();
      return true;
    };
    return {
      path,
      setPath,
      prefix: () => prefixOf(path(), root),
      folder,
      stem,
      listing,
      onTurn,
      completions,
      pending,
      tab,
      tag,
      peek,
      choose,
      cancel: end,
    };
  });
}
