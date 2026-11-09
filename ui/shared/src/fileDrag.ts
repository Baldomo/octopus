import { onCleanup } from "solid-js";
import type { FileDrag } from "./generated";

type Handle = (drag: FileDrag, under: Element | null) => void;
const handles = new Set<Handle>();

/** Tells every surface where files from the system are dragged, and the element under the pointer. */
export function fileDragged(drag: FileDrag) {
  const under = drag.kind === "leave" ? null : document.elementFromPoint(drag.x, drag.y);
  for (const handle of handles) handle(drag, under);
}

/** Calls `handle` with each drag of files from the system, for as long as the calling component lives. */
export function onFileDrag(handle: Handle) {
  handles.add(handle);
  onCleanup(() => handles.delete(handle));
}

const SHELL_SAFE = /^[\w@%+=:,./-]+$/;

/** `path` as a shell reads it back: as it is when it holds no character the shell treats specially, else quoted. */
export const shellQuoted = (path: string) => (SHELL_SAFE.test(path) ? path : `'${path.replaceAll("'", `'\\''`)}'`);

const CONTROL = /[\x00-\x1f\x7f]/;

/** Whether pasting `path` types only the path: a line break or other control character could run a command. */
export const pastesSafely = (path: string) => !CONTROL.test(path);
