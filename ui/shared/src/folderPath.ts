import { createComputed, createEffect, createMemo, createRoot, createSignal, on, untrack } from "solid-js";
import { createStore } from "solid-js/store";
import { backend } from "./backend";
import type { Folder } from "./generated";
import type { Toward } from "./motion";
import { isLocal, serverLabel } from "./state";

/** A folder completing the path: its name, the path picking it leads to, the path completing into it, and whether the
 * server's OS guards it. */
type Completion = { name: string; pick: string; into: string; guarded: boolean };

/** What the list shows while no folder completes the path: why, whether it says the folder is empty, waits on the
 * server's OS or was refused, and the URL of the system setting that lets this machine's server in, if one does. */
type Note = { text: string; kind: "empty" | "waiting" | "refused"; setting: string | null };

/** How long a reading stands before showing its folder again asks the server anew, the reading staying meanwhile. */
const FRESH_MS = 10_000;

/** Every folder the server has read this session, by server and folder: the folder, or why it is none. */
const [reads, setReads] = createStore<Record<string, Folder | string>>({});
/** When each reading arrived, or Infinity while it is on its way, which can take as long as the OS asking its user, so
 * a folder shown again while its reading is fresh or coming is not asked for again. */
const arrived = new Map<string, number>();

const trimmed = (folder: string) => folder.replace(/(.)\/+$/, "$1");

const keyOf = (server: string, folder: string) => `${server}\n${trimmed(folder)}`;

/** The server's reading of `folder`: the folder, or why it is none; undefined until first read. */
export const readingOf = (server: string, folder: string): Folder | string | undefined => reads[keyOf(server, folder)];

/** The folders inside a reading, when the server could list them. */
const listedIn = (reading: Folder | string | undefined) =>
  typeof reading === "object" && reading.inside.kind === "listed" ? reading.inside : undefined;

/** Asks `server` to read `folder` unless its reading is fresh or on its way. */
export const readFolder = (server: string, folder: string) =>
  untrack(() => {
    const key = keyOf(server, folder);
    if (performance.now() - (arrived.get(key) ?? -Infinity) < FRESH_MS) return;
    arrived.set(key, Infinity);
    backend
      .folderGet(server, folder)
      .then(
        (read) => setReads(key, read),
        (refused) => setReads(key, String(refused)),
      )
      .finally(() => arrived.set(key, performance.now()));
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
 * segment. A folder the server's OS guards is read only once the path goes into it, since reading it can have the OS
 * ask its user. `choose` hands the folder to `chosen` once the server reads it as a directory and ends the picker,
 * unless `chosen` refuses it with false; `cancel` ends it leaving everything as it was.
 */
export function folderPicker(server: string, root: string, from: string, chosen: (folder: string) => boolean | void) {
  return createRoot((end) => {
    const [path, setPath] = createSignal(from === "" || from.endsWith("/") ? from : `${from}/`);
    const reading = (typed: string) => readingOf(server, absolute(typed, root));
    const peek = (typed: string) => readFolder(server, absolute(typed, root));
    /** Reads a listed folder ahead of the path going into it, unless the server's OS guards it. */
    const readAhead = (row: Completion) => {
      if (!row.guarded) peek(row.into);
    };
    const cut = () => path().lastIndexOf("/") + 1;
    /** The path of the folder whose subfolders complete the path's last segment, and that segment. */
    const listedPath = () => path().slice(0, cut());
    const stem = () => path().slice(cut());
    /** The folder the last segment names in the listed folder, matching its case or not. */
    const stemFolder = () =>
      listedIn(reading(listedPath()))?.folders.find(({ name }) => name.toLowerCase() === stem().toLowerCase());
    /** Whether the path names a folder to read ahead: its last segment is empty, `.` or `..`, or an unguarded folder
     * of the listed folder, or the listed folder is still unread. */
    const possible = () =>
      reading(listedPath()) === undefined || ["", ".", ".."].includes(stem()) || stemFolder()?.guarded === false;
    createEffect(() => {
      peek(listedPath());
      if (possible()) peek(path());
    });
    /** The folder the path names, once the server has listed it. */
    const folder = () => {
      const now = reading(path());
      return typeof now === "object" && now.inside.kind === "listed" ? now : undefined;
    };
    const choosable = () => folder() !== undefined || stemFolder()?.guarded === true;
    const listedFolder = () => trimmed(absolute(listedPath(), root));
    /** The name of the folder listed while its parent lists it as one the server's OS guards. */
    const guardedTarget = () => {
      const name = baseName(listedFolder());
      const listed = listedIn(readingOf(server, parentOf(listedFolder())));
      return listed?.folders.some((each) => each.name === name && each.guarded) ? name : undefined;
    };
    /**
     * The folder listed, its folders completing the path, `..` first while the last segment is empty, and the note
     * shown while no folder completes it. A guarded folder still being read shows that it waits on the server; any
     * other keeps the last listing.
     */
    const listing = createMemo<{ folder: string; rows: Completion[]; note: Note | null }>(
      (was) => {
        const listed = reading(listedPath());
        const label = serverLabel(server);
        if (listed === undefined) {
          const name = guardedTarget();
          if (name === undefined) return was;
          const text = `Waiting for ${label} to let octopus into ${name}. Its screen may be asking.`;
          return { folder: listedFolder(), rows: [], note: { text, kind: "waiting", setting: null } };
        }
        if (typeof listed !== "object")
          return { folder: was.folder, rows: [], note: { text: listed, kind: "refused", setting: null } };
        if (listed.inside.kind === "kept_out") {
          const setting = listed.inside.setting;
          const allow =
            setting === null
              ? "Give the account octopus runs as permission to read and open it."
              : `Turn on octopus in ${setting.name}.`;
          const text = `${label} keeps octopus out of ${baseName(listed.folder)}. ${allow}`;
          const url = isLocal(server) ? (setting?.url ?? null) : null;
          return { folder: listed.folder, rows: [], note: { text, kind: "refused", setting: url } };
        }
        const typedStem = stem().toLowerCase();
        const named = listed.inside.folders
          .filter(
            ({ name }) =>
              name.toLowerCase().startsWith(typedStem) && (typedStem.startsWith(".") || !name.startsWith(".")),
          )
          .map(({ name, guarded }) => ({ name, pick: `${listedPath()}${name}`, guarded }));
        const up =
          typedStem === "" && listed.folder !== "/"
            ? [{ name: "..", pick: relative(parentOf(listed.folder), root), guarded: false }]
            : [];
        const rows = [...up, ...named].map((each) => ({
          ...each,
          into: each.pick === "" ? "" : `${each.pick.replace(/\/$/, "")}/`,
        }));
        const why = stem() === "" ? "No folders inside" : `No folder starts with “${stem()}”`;
        return {
          folder: listed.folder,
          rows,
          note: named.length > 0 ? null : { text: why, kind: "empty", setting: null },
        };
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
    /** Hands the folder over once listed; a path naming a guarded folder goes into it instead, which reads it. */
    const choose = () => {
      const now = folder();
      if (now === undefined) {
        const named = stemFolder();
        if (named?.guarded) setPath(`${listedPath()}${named.name}/`);
        return false;
      }
      if (chosen(now.folder) === false) return false;
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
      readAhead,
      choosable,
      choose,
      cancel: end,
    };
  });
}
