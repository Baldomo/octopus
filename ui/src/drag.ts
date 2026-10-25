import { createRoot, createSelector, createSignal } from "solid-js";
import { backend } from "./backend";
import { TASK_STATES, type Row, type TaskState } from "./generated";
import { act, dragged, setDragged } from "./state";

/** Pointer travel that turns a press on a task into a drag. */
const DRAG_PX = 4;

type Target = { section: string; state: TaskState };
const [over, setOver] = createSignal<Target | null>(null);

function targetAt(x: number, y: number, section: string): Target | null {
  const element = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-drop-section]");
  const state = TASK_STATES.find((candidate) => candidate === element?.dataset.dropState);
  return element?.dataset.dropSection === section && state !== undefined ? { section, state } : null;
}

/** Drags run on pointer events, so every platform's webview behaves the same. */
export const dragSource = (row: Row, section: string, state: TaskState) => ({
  onPointerDown: (down: PointerEvent) => {
    if (down.button !== 0) return;
    const move = (event: PointerEvent) => {
      if (dragged() === null) {
        if (Math.hypot(event.clientX - down.clientX, event.clientY - down.clientY) < DRAG_PX) return;
        setDragged({ key: row.key, at: row.at, section, state });
      }
      setOver(targetAt(event.clientX, event.clientY, section));
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      const task = dragged();
      const target = over();
      setDragged(null);
      setOver(null);
      if (task === null) return;
      const swallow = (click: MouseEvent) => click.stopPropagation();
      window.addEventListener("click", swallow, { capture: true, once: true });
      setTimeout(() => window.removeEventListener("click", swallow, { capture: true }));
      if (target !== null && target.state !== task.state) act(backend.taskMove(task.at, target.state));
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up, { once: true });
  },
});

/** A group or column that takes tasks of its own section and moves them to its state. */
export const dropTarget = (section: string, state: TaskState) => ({
  over: () => over()?.section === section && over()?.state === state,
  attributes: { "data-drop-section": section, "data-drop-state": state },
});

export const showsDropHint = (section: string) => dragged()?.section === section;

export const isDragged = createRoot(() => createSelector(() => dragged()?.key ?? null));
