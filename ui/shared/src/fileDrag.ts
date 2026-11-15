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
