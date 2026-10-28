import { selectionFeedback } from "@tauri-apps/plugin-haptics";
import { createRoot, createSelector, createSignal } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import { TASK_STATES, type Row, type TaskState } from "@octopus/shared/generated.ts";
import { act, scale, ui } from "@octopus/shared/state.ts";
import type { Lift, Point } from "./press";

type Lifted = { key: string; section: string; state: TaskState };
const [lifted, setLifted] = createSignal<Lifted | null>(null);
/** The state the lifted task drops to under the finger: a state of its own section other than its own. */
const [over, setOver] = createSignal<TaskState | null>(null);

/** How near a scroller's edge the finger scrolls it, and how far it scrolls each frame at the very edge. */
const EDGE_PX = 48;
const EDGE_SCROLL_PX = 12;

export const isLifted = createRoot(() => createSelector(() => lifted()?.key ?? null));

export const liftsIn = (section: string) => lifted()?.section === section;

/** A group or state tab that takes a task lifted in its section to its state. */
export const dropTarget = (section: string, state: TaskState) => ({
  over: () => liftsIn(section) && over() === state,
  attributes: { "data-drop-section": section, "data-drop-state": state },
});

function targetAt({ x, y }: Point, task: Lifted) {
  const element = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-drop-state]");
  const state = TASK_STATES.find((candidate) => candidate === element?.dataset.dropState);
  return element?.dataset.dropSection === task.section && state !== undefined && state !== task.state ? state : null;
}

/** How far to scroll along one axis of a box toward the edge the finger is near. */
function edgePush(at: number, start: number, end: number) {
  const near = (distance: number) => (EDGE_SCROLL_PX * Math.max(0, EDGE_PX - distance)) / EDGE_PX;
  return near(end - at) - near(at - start);
}

/** Scrolls every scroller under the finger toward the edge of it the finger is near; true when one moved. */
function autoscroll({ x, y }: Point) {
  let moved = false;
  for (let element = document.elementFromPoint(x, y); element !== null; element = element.parentElement) {
    const box = element.getBoundingClientRect();
    const { overflowX, overflowY } = getComputedStyle(element);
    const [top, left] = [element.scrollTop, element.scrollLeft];
    if (overflowY === "auto") element.scrollTop += edgePush(y, box.top, box.bottom);
    if (overflowX === "auto") element.scrollLeft += edgePush(x, box.left, box.right);
    moved ||= element.scrollTop !== top || element.scrollLeft !== left;
  }
  return moved;
}

/**
 * Lifts a top-level task's row or card as a ghost that follows the finger up and down its list, below any group label
 * it would cover, so the labels and a board's state tabs stay in sight. It scrolls what the finger nears the edge of.
 * Dropping it on another state of its section moves it there; every other drop leaves it.
 */
export function lift(row: Row, section: string, element: HTMLElement, from: Point): Lift | null {
  const content = element.closest<HTMLElement>(".content");
  const list = element.closest<HTMLElement>(".list, .cards");
  if (row.depth > 0 || content === null || list === null) return null;
  const task = { key: row.key, section, state: row.state };
  const zoom = scale(ui.prefs);
  const box = content.getBoundingClientRect();
  const lifted = element.getBoundingClientRect();
  const within = list.getBoundingClientRect();
  const ghost = element.cloneNode(true) as HTMLElement;
  ghost.classList.add("ghost");
  ghost.style.top = `${(lifted.top - box.top) / zoom}px`;
  ghost.style.setProperty("--ws", getComputedStyle(element).getPropertyValue("--ws"));
  content.append(ghost);
  setLifted(task);
  let finger = from;
  let frame = 0;
  const follow = () => {
    let top = Math.min(Math.max(lifted.top + finger.y - from.y, within.top), within.bottom - lifted.height);
    for (const label of list.querySelectorAll(".gl")) {
      const { top: start, bottom } = label.getBoundingClientRect();
      const [above, below] = [start - lifted.height, bottom];
      if (top > above && top < below) top = top - above < below - top ? above : below;
    }
    ghost.style.translate = `0 ${(top - lifted.top) / zoom}px`;
    const target = targetAt(finger, task);
    if (target === over()) return;
    setOver(target);
    if (target !== null) void selectionFeedback();
  };
  const scroll = () => {
    if (autoscroll(finger)) follow();
    frame = requestAnimationFrame(scroll);
  };
  const end = () => {
    cancelAnimationFrame(frame);
    ghost.remove();
    setLifted(null);
    setOver(null);
  };
  return {
    move: (to) => {
      finger = to;
      follow();
      if (frame === 0) frame = requestAnimationFrame(scroll);
    },
    drop: () => {
      const target = over();
      end();
      if (target !== null) act(backend.taskMove(row.at, target));
    },
    cancel: end,
  };
}
