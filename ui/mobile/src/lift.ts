import { createRoot, createSelector, createSignal } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import { TASK_STATES, type Row, type TaskState } from "@octopus/shared/generated.ts";
import { model, report, scale, ui } from "@octopus/shared/state.ts";
import { feel } from "./feel";
import { awaitArrival, followChange } from "./flip";
import { animate, curve, play, PROJECT_S, reduced, SNAPPY, tracker, type Running } from "./motion";
import type { Lift, Point } from "./press";
import { give } from "./scroll";

/**
 * The task in hand: its own row and its subtasks' travel with it, and `toward` is the state it goes to, the one under
 * the finger while held and the one it dropped on until it lands there.
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

/** How near a scroller's edge the finger scrolls it, and how far it scrolls each frame at the very edge. */
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

/** Whether a finger holds a task lifted in `section`, or one dropped there still lands. */
export const liftsIn = (section: string) => {
  const held = hand();
  return held?.section === section && (!held.dropped || held.toward !== null);
};

/** A group or state tab that takes a task lifted in its section to its state. */
export const dropTarget = (section: string, state: TaskState) => ({
  over: () => {
    const held = hand();
    return held?.section === section && !held.dropped && held.toward === state;
  },
  attributes: { "data-drop-section": section, "data-drop-state": state },
});

function targetAt({ x, y }: Point, task: { section: string; state: TaskState }) {
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

/** Keeps a position within `min` to `max`, giving way past either end. */
const within = (at: number, min: number, max: number) =>
  at < min ? min + give(at - min) : at > max ? max + give(at - max) : at;

/** Where a list's item lays out on the screen, without the slide it may be in the middle of. */
function rest(element: Element, zoom: number) {
  const box = element.getBoundingClientRect();
  const slide = new DOMMatrix(getComputedStyle(element).transform);
  return { left: box.left - slide.m41 * zoom, top: box.top - slide.m42 * zoom };
}

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
 * Lifts a top-level task's row or card as a ghost that rises under the finger and follows it one to one: up and down
 * its list on Tasks, anywhere on its board, rubber-banding past the top and foot. Over another state of its section its
 * rows give up their place, and that state's group opens one where the task will sit or, on a board, the ghost thins
 * over the state's tab so its ring shows. Letting go drops it where the throw projects: the ghost flies there at the
 * finger's speed and the rows show once they arrive, or it flies home.
 */
export function lift(row: Row, section: string, element: HTMLElement, from: Point): Lift | null {
  const content = element.closest<HTMLElement>(".content");
  const list = element.closest<HTMLElement>(".list, .cards");
  if (row.depth > 0 || content === null || list === null) return null;
  ending?.();
  const board = list.classList.contains("cards");
  const zoom = scale(ui.prefs);
  const origin = element.getBoundingClientRect();
  const carried = [element];
  for (let next = element.nextElementSibling; next instanceof HTMLElement; next = next.nextElementSibling) {
    if (Number(next.style.getPropertyValue("--d")) === 0) break;
    carried.push(next);
  }
  const room = carried.reduce((height, each) => height + each.offsetHeight, 0);

  const ghost = element.cloneNode(true) as HTMLElement;
  ghost.classList.add("ghost");
  ghost.removeAttribute("data-pressed");
  ghost.style.top = `${(origin.top - content.getBoundingClientRect().top) / zoom}px`;
  ghost.style.setProperty("--ws", getComputedStyle(element).getPropertyValue("--ws"));
  content.append(ghost);
  void ghost.offsetWidth;
  ghost.classList.add("up");

  const keys = carried.map((each) => each.dataset.key ?? "");
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

  /** Where the ghost stands off the row it lifted from, in the layout's points. */
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
   * The place a state's group opens where the task will sit among its rows, ordered by title, and where that is on
   * the screen; an empty group's hint stands where it will sit.
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
      `.grp[data-state="${state}"][data-drop-section="${CSS.escape(section)}"]`,
    );
    if (group === null) return;
    const hint = group.querySelector(".empty");
    if (hint) return void (slot = () => ({ left: origin.left, top: rest(hint, zoom).top }));
    const rows = model.lists.sections
      .find((shown) => shown.name === section)
      ?.groups.find((each) => each.state === state);
    const after = rows?.rows.find((each) => each.depth === 0 && sortsBefore(row, each));
    const next = after && group.querySelector<HTMLElement>(`[data-key="${CSS.escape(after.key)}"]`);
    gap = next ? { element: next, side: "marginTop" } : { element: group, side: "paddingBottom" };
    gap.element.style[gap.side] = `${room}px`;
    slot = next
      ? () => ({ left: origin.left, top: rest(next, zoom).top - room * zoom })
      : () => ({ left: origin.left, top: group.getBoundingClientRect().bottom - room * zoom });
  };

  /** Heads the task to `state`, or back to its own place at null, the list making room as it goes. */
  const aim = (state: TaskState | null, dropped: boolean) => {
    const was = hand();
    if (was === null || (state === was.toward && dropped === was.dropped)) return;
    if (state !== was.toward) closeGap();
    setHand({ ...was, toward: state, dropped });
    ghost.classList.toggle("aiming", board && state !== null);
    if (state !== null && state !== was.toward) openGap(state);
    followChange(list);
    if (state !== null && !dropped) feel("select");
  };

  let finger = from;
  let frame = 0;
  const follow = () => {
    const bounds = (board ? content : list).getBoundingClientRect();
    const top = within(origin.top + finger.y - from.y, bounds.top, bounds.bottom - origin.height);
    const left = board ? origin.left + finger.x - from.x : origin.left;
    const shown = offset({ left, top });
    place(shown.x, shown.y);
    aim(targetAt(finger, task), false);
  };
  const scroll = () => {
    if (autoscroll(finger)) follow();
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

  return {
    move: (to) => {
      finger = to;
      follow();
      if (frame === 0) frame = requestAnimationFrame(scroll);
    },
    drop: () => {
      cancelAnimationFrame(frame);
      const speed = motion.velocity();
      const bounds = (board ? content : list).getBoundingClientRect();
      const ahead = {
        x: Math.min(Math.max(finger.x + speed.x * zoom * PROJECT_S, bounds.left + 1), bounds.right - 1),
        y: Math.min(Math.max(finger.y + speed.y * zoom * PROJECT_S, bounds.top + 1), bounds.bottom - 1),
      };
      const state = targetAt(ahead, task);
      if (state === null) return home();
      aim(state, true);
      feel("drop");
      const tab = document.elementFromPoint(ahead.x, ahead.y)?.closest(".tab");
      const refused = tab ? into(tab, state) : land(state);
      let failed = false;
      const fail = () => {
        if (failed || ending !== takeOver) return;
        failed = true;
        refused();
      };
      setTimeout(fail, LANDS_MS);
      backend.taskMove(row.at, state).catch((error) => {
        fail();
        report(error);
      });
    },
    cancel: home,
  };
}
