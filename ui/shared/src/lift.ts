import { createRoot, createSelector, createSignal } from "solid-js";
import { backend } from "./backend";
import { awaitArrival, followChange } from "./flip";
import { TASK_STATES, type Row, type TaskState } from "./generated";
import { animate, curve, give, play, PROJECT_S, reduced, SNAPPY, tracker, type Running } from "./motion";
import { model, report } from "./state";

export type Point = { x: number; y: number };

/** A task lifted under a held pointer: moving the pointer drags it, letting go drops it, and cancelling flies it home. */
export type Lift = { move: (to: Point) => void; drop: () => void; cancel: () => void };

/**
 * Where a task lifts: its ghost stands in `stage`, which holds still while `list`, the scroller of its rows, scrolls.
 * A `free` ghost follows the pointer anywhere on the stage, else only up and down the list. A surface that `throws`
 * drops a task where the pointer's speed carries it, else where the pointer lets go. `covered` tells the points where
 * other UI stands over the surface, which take no drop. `zoom` is how many screen pixels one of the layout's takes,
 * and `feel` plays a haptic where the device has one.
 */
export type Surface = {
  stage: HTMLElement;
  list: HTMLElement;
  free: boolean;
  throws: boolean;
  covered: (at: Point) => boolean;
  zoom: number;
  feel: (kind: "select" | "drop") => void;
};

/**
 * The task in hand: its own row and its subtasks' travel with it, and `toward` is the state it goes to, the one under
 * the pointer while held and the one it dropped on until it lands there.
 */
type Hand = {
  key: string;
  section: string;
  state: TaskState;
  carried: string[];
  toward: TaskState | null;
  dropped: boolean;
};
const [hand, setHand] = createSignal<Hand | null>(null);
/** The rows of a dropped task that stay out of sight while its ghost lands where they arrive. */
const [landing, setLanding] = createSignal<string[]>([]);

/** How near a scroller's edge the pointer scrolls it, and how far it scrolls each frame at the very edge. */
const EDGE_PX = 48;
const EDGE_SCROLL_PX = 12;
/** How long a dropped ghost waits for its row to arrive before it flies back. */
const LANDS_MS = 1500;

/** The row the ghost stands in for, hidden under it. */
export const isLifted = createRoot(() => createSelector(() => hand()?.key ?? null));
/** A row the held task carries, which gives up its place while the task heads to another state. */
export const isAway = createRoot(() =>
  createSelector(
    () => {
      const held = hand();
      return held?.toward == null ? [] : held.carried;
    },
    (key: string, carried) => carried.includes(key),
  ),
);
export const isLanding = createRoot(() => createSelector(landing, (key: string, keys) => keys.includes(key)));

export const lifting = () => hand() !== null;

/** Whether a pointer holds a task lifted in `section`, or one dropped there still lands. */
export const liftsIn = (section: string) => {
  const held = hand();
  return held?.section === section && (!held.dropped || held.toward !== null);
};

/** A group, column or state tab that takes a task lifted in its section to its state. */
export const dropTarget = (section: string, state: TaskState) => ({
  over: () => {
    const held = hand();
    return held?.section === section && !held.dropped && held.toward === state;
  },
  attributes: { "data-drop-section": section, "data-drop-state": state },
});

function targetAt(at: Point, task: { section: string; state: TaskState }, covered: (at: Point) => boolean) {
  if (covered(at)) return null;
  const element = document.elementFromPoint(at.x, at.y)?.closest<HTMLElement>("[data-drop-state]");
  if (!element || element.dataset.dropSection !== task.section) return null;
  const state = TASK_STATES.find((candidate) => candidate === element.dataset.dropState);
  return state === undefined || state === task.state ? null : { state, element };
}

/** How far to scroll along one axis of a box toward the edge the pointer is near. */
function edgePush(at: number, start: number, end: number) {
  const near = (distance: number) => (EDGE_SCROLL_PX * Math.max(0, EDGE_PX - distance)) / EDGE_PX;
  return near(end - at) - near(at - start);
}

const scrolls = (overflow: string) => overflow === "auto" || overflow === "scroll";

/** Scrolls every scroller under the pointer toward the edge of it the pointer is near; true when one moved. */
function autoscroll({ x, y }: Point) {
  let moved = false;
  for (let element = document.elementFromPoint(x, y); element !== null; element = element.parentElement) {
    const box = element.getBoundingClientRect();
    const { overflowX, overflowY } = getComputedStyle(element);
    const [top, left] = [element.scrollTop, element.scrollLeft];
    if (scrolls(overflowY)) element.scrollTop += edgePush(y, box.top, box.bottom);
    if (scrolls(overflowX)) element.scrollLeft += edgePush(x, box.left, box.right);
    moved ||= element.scrollTop !== top || element.scrollLeft !== left;
  }
  return moved;
}

/** Keeps a position within `min` to `max`, giving way past either end. */
const within = (at: number, min: number, max: number) =>
  at < min ? min + give(at - min) : at > max ? max + give(at - max) : at;

/** Where a list's item lays out on the screen, without the slide it may be in the middle of. */
function rest(element: Element, zoom: number) {
  const box = element.getBoundingClientRect();
  const slide = new DOMMatrix(getComputedStyle(element).transform);
  return { left: box.left - slide.m41 * zoom, top: box.top - slide.m42 * zoom };
}

/** The height an item takes in its list, its top margin included. */
const footprint = (element: HTMLElement) => element.offsetHeight + parseFloat(getComputedStyle(element).marginTop);

/** Whether `a` sorts before `b` in a group, which orders its rows by title and then by where each task lives. */
function sortsBefore(a: Row, b: Row) {
  const order = (row: Row) => [row.title.toLowerCase(), row.at.server, row.at.task];
  const [left, right] = [order(a), order(b)];
  const differs = left.findIndex((part, nth) => part !== right[nth]);
  return differs >= 0 && left[differs]! < right[differs]!;
}

/** Ends the lift before, which a new one takes over from wherever it stands. */
let ending: (() => void) | null = null;

/**
 * Lifts a top-level task's row or card as a ghost that rises under the pointer and follows it one to one, rubber-banding
 * past the list's top and foot. Over another state of its section its rows give up their place, and that state's group
 * or column opens one where the task will sit or, on a state tab, the ghost thins so the tab's ring shows. Letting go
 * drops it where the throw projects: the ghost flies there at the pointer's speed and the rows show once they arrive,
 * or it flies home.
 */
export function lift(row: Row, section: string, element: HTMLElement, from: Point, surface: Surface): Lift {
  const { stage, list, free, throws, covered, zoom, feel } = surface;
  ending?.();
  const shown = model.lists.sections.find((each) => each.name === section);
  const rowsOf = (state: TaskState) =>
    (free ? shown?.columns : shown?.groups)?.find((each) => each.state === state)?.rows ?? [];
  const own = rowsOf(row.state);
  const after = own.slice(own.findIndex((each) => each.key === row.key) + 1);
  const nextTopLevel = after.findIndex((each) => each.depth === 0);
  const subtasks = nextTopLevel < 0 ? after : after.slice(0, nextTopLevel);
  const keys = [row.key, ...subtasks.map((each) => each.key)];
  const carried = keys
    .map((key) => list.querySelector<HTMLElement>(`[data-key="${CSS.escape(key)}"]`))
    .filter((each) => each !== null);
  const room = carried.reduce((height, each) => height + footprint(each), 0);
  const margin = parseFloat(getComputedStyle(element).marginTop) * zoom;

  const origin = element.getBoundingClientRect();
  const stageBox = stage.getBoundingClientRect();
  const ghost = element.cloneNode(true) as HTMLElement;
  ghost.classList.add("ghost");
  ghost.removeAttribute("data-pressed");
  ghost.style.top = `${(origin.top - stageBox.top) / zoom - stage.clientTop}px`;
  ghost.style.setProperty("--from-left", `${(origin.left - stageBox.left) / zoom - stage.clientLeft}px`);
  ghost.style.setProperty("--from-width", `${origin.width / zoom}px`);
  ghost.style.setProperty("--ws", getComputedStyle(element).getPropertyValue("--ws"));
  stage.append(ghost);
  void ghost.offsetWidth;
  ghost.classList.add("up");

  const task = { key: row.key, section, state: row.state, carried: keys };
  setHand({ ...task, toward: null, dropped: false });
  const takeOver = () => {
    ghost.remove();
    closeGap();
    end();
  };
  const end = () => {
    if (ending !== takeOver) return;
    ending = null;
    setLanding([]);
    setHand(null);
  };
  ending = takeOver;

  /** Where the ghost stands off the row it lifted from, in the layout's pixels. */
  let at = { x: 0, y: 0 };
  const motion = tracker();
  const place = (x: number, y: number) => {
    at = { x, y };
    motion.add(x, y);
    ghost.style.translate = `${x}px ${y}px`;
  };
  let flight: Running[] = [];
  let flights = 0;
  /** Carries the ghost to `to` from where it stands at the speed it moves, resolving whether it got there. */
  const fly = async (to: { x: number; y: number }) => {
    const flown = ++flights;
    const speed = motion.velocity();
    for (const running of flight) running.stop();
    flight = [
      animate(at.x, to.x, (x) => place(x, at.y), SNAPPY, speed.x),
      animate(at.y, to.y, (y) => place(at.x, y), SNAPPY, speed.y),
    ];
    await Promise.all(flight.map((running) => running.done));
    return flown === flights;
  };
  const offset = (spot: { left: number; top: number }) => ({
    x: (spot.left - origin.left) / zoom,
    y: (spot.top - origin.top) / zoom,
  });

  /**
   * The place a state's group or column opens where the task will sit among its rows, ordered by title, and where
   * that is on the screen; an empty one's hint stands where it will sit.
   */
  let gap: { element: HTMLElement; side: "marginTop" | "paddingBottom" } | null = null;
  const ownPlace = () => rest(element, zoom);
  let slot = ownPlace;
  const closeGap = () => {
    if (gap) gap.element.style[gap.side] = "";
    [gap, slot] = [null, ownPlace];
  };
  const openGap = (state: TaskState) => {
    const group = list.querySelector<HTMLElement>(
      `[data-state="${state}"][data-drop-section="${CSS.escape(section)}"]`,
    );
    if (group === null) return;
    const hint = group.querySelector(".drophint");
    if (hint)
      return void (slot = () => ({ left: free ? rest(hint, zoom).left : origin.left, top: rest(hint, zoom).top }));
    const rowElement = (key: string) => group.querySelector<HTMLElement>(`[data-key="${CSS.escape(key)}"]`);
    const rows = rowsOf(state).filter((each) => each.depth === 0);
    const first = rows[0] && rowElement(rows[0].key);
    const next = rows.find((each) => sortsBefore(row, each));
    const nextElement = next && rowElement(next.key);
    const left = () => (free && first ? rest(first, zoom).left : origin.left);
    if (nextElement) {
      gap = { element: nextElement, side: "marginTop" };
      nextElement.style.marginTop = `${parseFloat(getComputedStyle(nextElement).marginTop) + room}px`;
      slot = () => ({ left: left(), top: rest(nextElement, zoom).top - room * zoom });
    } else {
      const style = getComputedStyle(group);
      const below = parseFloat(style.paddingBottom) + parseFloat(style.borderBottomWidth) + room;
      gap = { element: group, side: "paddingBottom" };
      group.style.paddingBottom = `${parseFloat(style.paddingBottom) + room}px`;
      slot = () => ({ left: left(), top: group.getBoundingClientRect().bottom - below * zoom + margin });
    }
  };

  /** Heads the task to `state`, or back to its own place at null, the list making room as it goes. */
  const aim = (target: { state: TaskState; element: HTMLElement } | null, dropped: boolean) => {
    const was = hand();
    const state = target?.state ?? null;
    if (was === null || (state === was.toward && dropped === was.dropped)) return;
    if (state !== was.toward) closeGap();
    setHand({ ...was, toward: state, dropped });
    ghost.classList.toggle("aiming", target !== null && !list.contains(target.element));
    if (state !== null && state !== was.toward) openGap(state);
    followChange(list);
    if (state !== null && !dropped) feel("select");
  };

  let pointer = from;
  let frame = 0;
  const follow = () => {
    const bounds = (free ? stage : list).getBoundingClientRect();
    const top = within(origin.top + pointer.y - from.y, bounds.top, bounds.bottom - origin.height);
    const left = free ? origin.left + pointer.x - from.x : origin.left;
    const spot = offset({ left, top });
    place(spot.x, spot.y);
    aim(targetAt(pointer, task, covered), false);
  };
  const scroll = () => {
    if (autoscroll(pointer)) follow();
    frame = requestAnimationFrame(scroll);
  };

  /** Flies the ghost back onto its own row, wherever the list has moved it. */
  const home = () => {
    cancelAnimationFrame(frame);
    setLanding([]);
    aim(null, true);
    ghost.classList.remove("up");
    const finish = () => {
      ghost.remove();
      end();
    };
    if (reduced() || !element.isConnected) return finish();
    void fly(offset(ownPlace())).then((arrived) => arrived && finish());
  };
  /** Lands the ghost where the task's rows arrive in the group of `state`, the gap held open for them until then. */
  const land = (state: TaskState) => {
    setLanding(keys);
    ghost.classList.remove("up");
    let [arrived, settled] = [false, false];
    const show = () => {
      if (!arrived || !settled || ending !== takeOver) return;
      ghost.remove();
      end();
      for (const key of keys.slice(1)) {
        const subtask = list.querySelector(`[data-key="${CSS.escape(key)}"]`);
        if (subtask) play(subtask, [{ opacity: 0 }, { opacity: 1 }], SNAPPY);
      }
    };
    const settle = (spot: { left: number; top: number }) =>
      void fly(offset(spot)).then((there) => {
        settled = there;
        show();
      });
    settle(slot());
    const stop = awaitArrival(row.key, state, (arrival) => {
      if (ending !== takeOver) return;
      arrived = true;
      closeGap();
      setHand((held) => held && { ...held, carried: [] });
      settle(arrival.getBoundingClientRect());
    });
    return () => {
      if (arrived) return;
      stop();
      home();
    };
  };
  /** Shrinks the ghost into the state tab it dropped on; the card gives up its column's place until it arrives. */
  const into = (tab: Element, state: TaskState) => {
    const box = tab.getBoundingClientRect();
    const middle = {
      left: box.left + box.width / 2 - origin.width / 2,
      top: box.top + box.height / 2 - origin.height / 2,
    };
    void fly(offset(middle));
    const shrink = ghost.animate([{ opacity: 0, scale: 0.3 }], curve(SNAPPY));
    shrink.finished.then(
      () => ghost.remove(),
      () => ghost.remove(),
    );
    let arrived = false;
    const stop = awaitArrival(row.key, state, () => {
      arrived = true;
      end();
    });
    return () => {
      if (arrived) return;
      stop();
      end();
      followChange(list);
    };
  };

  /** Where the pointer's throw carries it, kept within the surface. */
  const projected = () => {
    const speed = motion.velocity();
    const bounds = (free ? stage : list).getBoundingClientRect();
    return {
      x: Math.min(Math.max(pointer.x + speed.x * zoom * PROJECT_S, bounds.left + 1), bounds.right - 1),
      y: Math.min(Math.max(pointer.y + speed.y * zoom * PROJECT_S, bounds.top + 1), bounds.bottom - 1),
    };
  };

  return {
    move: (to) => {
      pointer = to;
      follow();
      if (frame === 0) frame = requestAnimationFrame(scroll);
    },
    drop: () => {
      cancelAnimationFrame(frame);
      const target = targetAt(throws ? projected() : pointer, task, covered);
      if (target === null) return home();
      aim(target, true);
      feel("drop");
      const refused = list.contains(target.element) ? land(target.state) : into(target.element, target.state);
      let failed = false;
      const fail = () => {
        if (failed || ending !== takeOver) return;
        failed = true;
        refused();
      };
      setTimeout(fail, LANDS_MS);
      backend.taskMove(row.at, target.state).catch((error) => {
        fail();
        report(error);
      });
    },
    cancel: home,
  };
}
