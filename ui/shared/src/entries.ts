import { each, said } from "./errors";
import type { EntryKind, TaskView } from "./generated";
import { isLocal, serverLabel } from "./state";
import { freeName, join, nameOf, parentOf, within, type TaskFiles } from "./taskFiles";

/** How a client says what an action on entries did, and why each entry it could not act on failed. */
export type Say = { done: (text: string) => void; problem: (text: string) => void };

const plural = (count: number, one: string) => `${count} ${one}${count === 1 ? "" : "s"}`;
export const subject = (paths: string[]) => (paths.length === 1 ? nameOf(paths[0]!) : plural(paths.length, "item"));
const where = (dir: string) => (dir === "." ? "the task's directory" : dir);

/** The folder an entry's actions land in: a folder itself, or a file's folder. */
export const folderOf = (path: string, kind: EntryKind) => (kind === "directory" ? path : parentOf(path));

/** Runs an action, saying why it failed rather than rejecting. */
const saying = (say: Say, action: () => Promise<unknown>) =>
  action().then(
    () => undefined,
    (error: unknown) => say.problem(said(error)),
  );

/** The path `name` takes in `dir`, which fails while `taken`, the names in `dir`, holds it. */
function claim(taken: Set<string>, dir: string, name: string) {
  if (taken.has(name)) throw new Error(`${where(dir)} already has ${name}`);
  return join(dir, name);
}

const vacant = async (files: TaskFiles, dir: string, name: string) => claim(await files.names(dir), dir, name);

/** Makes a file or folder named `name` in `dir`, refusing a name `dir` already has. */
export const create = async (files: TaskFiles, dir: string, name: string, kind: EntryKind) =>
  files.create(await vacant(files, dir, name), kind);

/** Renames the entry at `path`, refusing a name its folder already has. */
export const rename = async (files: TaskFiles, path: string, name: string) =>
  files.move(path, await vacant(files, parentOf(path), name));

export const outermost = (paths: string[]) =>
  paths.filter((path) => !paths.some((other) => other !== path && within(path, other)));

export const absolute = (task: TaskView, path: string) =>
  path === "." ? task.task.directory : `${task.task.directory}/${path}`;

/** How deleting the entries confirms: a server on this machine sends them to its trash, another one deletes them. */
export function deletion(task: TaskView, paths: string[]) {
  const trash = isLocal(task.at.server);
  return {
    trash,
    question: trash
      ? `Move ${subject(paths)} to the Trash?`
      : `Delete ${subject(paths)} from ${serverLabel(task.at.server)} for good?`,
    button: trash ? "Move to Trash" : "Delete",
  };
}

/** Copies the entries into `dir` under names it lacks, or moves them there, never over an entry of the same name. */
export const place = (files: TaskFiles, paths: string[], dir: string, copying: boolean, say: Say) =>
  saying(say, async () => {
    const sources = outermost(paths).filter((path) => copying || parentOf(path) !== dir);
    const taken = await files.names(dir);
    const done = await each(
      sources,
      async (path) => {
        if (copying) {
          const name = freeName(nameOf(path), taken);
          taken.add(name);
          return files.copy(path, join(dir, name));
        }
        return files.move(path, claim(taken, dir, nameOf(path)));
      },
      say.problem,
    );
    landed(files, dir, done, copying ? "Copied" : "Moved", say);
  });

export function landed(files: TaskFiles, dir: string, done: number, verb: string, say: Say) {
  if (done === 0) return;
  if (dir !== ".") files.reveal(dir, true);
  say.done(`${verb} ${plural(done, "item")} into ${where(dir)}`);
}

/** Copies the held entries into `dir`, or moves the cut ones there, letting go of a cut. */
export const paste = (files: TaskFiles, dir: string, say: Say) =>
  saying(say, async () => {
    const clip = files.state.clip;
    if (clip === null) return;
    if (clip.cut) files.setClip(null);
    await place(files, clip.paths, dir, !clip.cut, say);
  });

export const duplicate = (files: TaskFiles, paths: string[], say: Say) =>
  saying(say, () =>
    each(
      outermost(paths),
      async (path) => files.copy(path, join(parentOf(path), freeName(nameOf(path), await files.names(parentOf(path))))),
      say.problem,
    ),
  );

export const copyPaths = (task: TaskView, paths: string[], full: boolean, say: Say) =>
  saying(say, async () => {
    await navigator.clipboard.writeText(paths.map((path) => (full ? absolute(task, path) : path)).join("\n"));
    say.done(`Copied ${paths.length === 1 ? (full ? "the path" : "the relative path") : plural(paths.length, "path")}`);
  });

/** Sends the entries to the trash of the server's machine, or deletes them for good. */
export const remove = (files: TaskFiles, paths: string[], trash: boolean, say: Say) =>
  saying(say, async () => {
    const removing = outermost(paths);
    const done = await each(removing, (path) => files.remove(path, trash), say.problem);
    if (done > 0) say.done(`${trash ? "Moved to the Trash" : "Deleted"}: ${subject(removing)}`);
  });
