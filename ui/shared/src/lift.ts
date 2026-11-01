import { createEffect, createRoot, createSelector, createSignal } from "solid-js";
import { backend } from "./backend";
import { awaitArrival, followChange } from "./flip";
import { TASK_STATES, type Row, type TaskState, type TaskView } from "./generated";
import { animate, curve, give, play, PROJECT_S, reduced, SNAPPY, tracker, type Running } from "./motion";
import { moveTo, placeAmong } from "./sectionOrder";
import { slotAmong } from "./slot";
import { arrange, arranged, model, orderSections, report, sections, ui } from "./state";

export type Point = { x: number; y: number };
type Spot = { left: number; top: number };

/** A row lifted under a held pointer: moving the pointer drags it, letting go drops it, and cancelling flies it home. */
export type Lift = { move: (to: Point) => void; drop: () => void; cancel: () => void };

/**
 * Where a row lifts: its ghost stands in `stage`, which holds still while `list`, the scroller of its rows, scrolls.
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

/** Where a held task goes: the group or column of `state`, at its top level or under the task `under`. */
type Aim = { state: TaskState; under: TaskView | null };

/**
 * The task in hand: its own row and its subtasks' travel with it, and `toward` is where it goes, the place under the
 * pointer while held and the one it dropped on until it lands there. Its rows are `away` from the first time it heads
 * to a state's group or column until it flies home, so the list shifts under the pointer once at most.
 */
type Hand = {
  key: string;
  section: string;
  carried: string[];
  toward: Aim | null;
  away: boolean;
  dropped: boolean;
};
const [hand, setHand] = createSignal<Hand | null>(null);
/** The rows of a dropped task that stay out of sight while its ghost lands where they arrive. */
const [landing, setLanding] = createSignal<string[]>([]);
/** The section whose header is lifted, and whether it has folded to its header. */
const [heldSection, setHeldSection] = createSignal<{ name: string; folded: boolean } | null>(null);

/** How near a scroller's edge the pointer scrolls it, and how far it scrolls each frame at the very edge. */
const EDGE_PX = 48;
const EDGE_SCROLL_PX = 12;
/** How long a dropped ghost waits for its row to arrive before it flies back. */
const LANDS_MS = 1500;
/** A task's row or card takes the held task as a subtask while the pointer is off its top and bottom quarters. */
const NEST_EDGE = 0.25;
/** How far a lifted section's header travels before its section folds, so a hold that stays put leaves it open. */
const FOLD_PX = 4;

/** The row the ghost stands in for, hidden under it. */
export const isLifted = createRoot(() => createSelector(() => hand()?.key ?? null));
/** A row the held task carries, which gives up its place while the task is away from it. */
export const isAway = createRoot(() =>
  createSelector(
    () => {
      const held = hand();
      return held?.away ? held.carried : [];
    },
    (key: string, carried) => carried.includes(key),
  ),
);
export const isLanding = createRoot(() => createSelector(landing, (key: string, keys) => keys.includes(key)));
/** The row the held task nests under if it drops now. */
export const isNesting = createRoot(() =>
  createSelector(() => {
    const held = hand();
    return held === null || held.dropped ? null : (held.toward?.under?.key ?? null);
  }),
);

export const isSectionLifted = (name: string) => heldSection()?.name === name;
export const isSectionFolded = (name: string) => {
  const held = heldSection();
  return held?.name === name && held.folded;
};

export const lifting = () => hand() !== null || heldSection() !== null;

/** Whether a pointer holds a task lifted in `section`, or one dropped there still lands. */
export const liftsIn = (section: string) => {
  const held = hand();
  return held?.section === section && (!held.dropped || held.toward !== null);
};

/** A group, column or state tab that takes a task lifted in its section to its state. */
export const dropTarget = (section: string, state: TaskState) => ({
  over: () => {
    const held = hand();
    return held?.section === section && !held.dropped && held.toward?.state === state && held.toward.under === null;
  },
  attributes: { "data-drop-section": section, "data-drop-state": state },
});

const stateOf = (element: Element | null | undefined) =>
  TASK_STATES.find((state) => state === element?.closest<HTMLElement>("[data-state]")?.dataset.state);

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
function rest(element: Element, zoom: number): Spot {
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

/** Where a ghost goes: up and down a `column` or along a `row`, giving way past either end, or `anywhere` on the stage. */
type Along = "column" | "row" | "anywhere";

/** Ends the lift before, which a new one takes over from wherever it stands. */
let ending: (() => void) | null = null;

/**
 * A ghost of `element` risen on the stage, standing where `place` last put it off the spot it rose from, in the
 * layout's pixels. `fly` carries it to a spot on the screen at the speed it moves, resolving whether it got there
 * before another flight took over, and `follow` stands it where the pointer carried it within `bounds`, returning
 * that offset.
 */
function raise(element: HTMLElement, { stage, zoom }: Surface) {
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

  let at = { x: 0, y: 0 };
  const motion = tracker();
  const place = (x: number, y: number) => {
    at = { x, y };
    motion.add(x, y);
    ghost.style.translate = `${x}px ${y}px`;
  };
  const offset = (spot: Spot) => ({ x: (spot.left - origin.left) / zoom, y: (spot.top - origin.top) / zoom });
  let flight: Running[] = [];
  let flights = 0;
  const fly = async (spot: Spot) => {
    const to = offset(spot);
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
  const follow = (pointer: Point, from: Point, bounds: DOMRect, along: Along) => {
    const [left, top] = [origin.left + pointer.x - from.x, origin.top + pointer.y - from.y];
    const spot = offset({
      left:
        along === "column"
          ? origin.left
          : along === "row"
            ? within(left, bounds.left, bounds.right - origin.width)
            : left,
      top: along === "row" ? origin.top : within(top, bounds.top, bounds.bottom - origin.height),
    });
    place(spot.x, spot.y);
    return spot;
  };
  return { ghost, origin, motion, fly, follow };
}

/** Calls `follow` as the pointer moves, and again each frame the pointer near a scroller's edge scrolls it. */
function chase(follow: (pointer: Point) => void) {
  let pointer: Point | null = null;
  let frame = 0;
  const scroll = () => {
    if (pointer && autoscroll(pointer)) follow(pointer);
    frame = requestAnimationFrame(scroll);
  };
  return {
    move: (to: Point) => {
      pointer = to;
      follow(to);
      if (frame === 0) frame = requestAnimationFrame(scroll);
    },
    stop: () => cancelAnimationFrame(frame),
  };
}

/** Lifts a row, card, header or tab under the pointer at `from` on `surface`. */
export type Start = (element: HTMLElement, from: Point, surface: Surface) => Lift;

/** The ancestors of a lifted element that it lifts within: the ghost's still `stage` and the scrolling `list`. */
export type Within = { stage: string; list: string; free: boolean };

/** A pointer that presses an element and moves lifts it through `start` within its `within` ancestors. */
export type LiftOnDrag = (
  within: Within,
  start: Start,
) => { onPointerDown: (down: PointerEvent & { currentTarget: HTMLElement }) => void };

/**
 * Lifts a task's row or card as a ghost that rises under the pointer and follows it one to one, rubber-banding past
 * the list's top and foot. Over another state of its section its rows give up their place, and that state's group or
 * column opens one where the task will sit or, on a state tab, the ghost thins so the tab's ring shows. Over the middle
 * of another task's row in its workspace, that row lights to take it as a subtask and the ghost thins over it. Letting
 * go drops it where the throw projects: the ghost flies there at the pointer's speed and the rows show once they
 * arrive, or it flies home.
 */
export const lift =
  (row: Row, section: string): Start =>
  (element, from, surface) => {
    const { list, free, throws, covered, zoom, feel } = surface;
    ending?.();
    const shown = model.lists.sections.find((each) => each.name === section);
    const rowsOf = (state: TaskState | undefined) =>
      (free ? shown?.columns : shown?.groups)?.find((each) => each.state === state)?.rows ?? [];
    const own = rowsOf(stateOf(element));
    const after = own.slice(own.findIndex((each) => each.key === row.key) + 1);
    const outside = after.findIndex((each) => each.depth <= row.depth);
    const subtasks = outside < 0 ? after : after.slice(0, outside);
    const keys = [row.key, ...subtasks.map((each) => each.key)];
    const carried = keys
      .map((key) => list.querySelector<HTMLElement>(`[data-key="${CSS.escape(key)}"]`))
      .filter((each) => each !== null);
    const room = carried.reduce((height, each) => height + footprint(each), 0);
    const margin = parseFloat(getComputedStyle(element).marginTop) * zoom;
    const parent = model.tasks[row.key]?.task.parent ?? null;
    /** Whether the point stands on the place the task's rows hold, which takes no drop until they give it up. */
    const onOwnPlace = ({ x, y }: Point) =>
      carried.some((each) => {
        const box = each.getBoundingClientRect();
        return x >= box.left && x < box.right && y >= box.top && y < box.bottom;
      });
    const nested = row.depth > 0;

    const { ghost, origin, motion, fly, follow } = raise(element, surface);
    setHand({ key: row.key, section, carried: keys, toward: null, away: false, dropped: false });
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

    /**
     * The task whose row's middle the pointer is over, which takes this one as a subtask if it is another task of its
     * workspace outside its own subtree; undefined off the middle, where the row's group decides.
     */
    const nestAt = (at: Point, target: HTMLElement, under: TaskView) => {
      const box = target.getBoundingClientRect();
      if (at.y < box.top + box.height * NEST_EDGE || at.y > box.bottom - box.height * NEST_EDGE) return undefined;
      const state = stateOf(target);
      const fits =
        state !== undefined &&
        !keys.includes(under.key) &&
        under.section === section &&
        under.at.server === row.at.server &&
        under.task.id !== parent;
      return fits ? { aim: { state, under }, element: target } : null;
    };
    const targetAt = (at: Point) => {
      if (covered(at) || onOwnPlace(at)) return null;
      const hit = document.elementFromPoint(at.x, at.y);
      const nest = hit?.closest<HTMLElement>("[data-key]");
      const under = model.tasks[nest?.dataset.key ?? ""];
      const nesting = nest && under ? nestAt(at, nest, under) : undefined;
      if (nesting !== undefined) return nesting;
      const target = hit?.closest<HTMLElement>("[data-drop-state]");
      if (!target || target.dataset.dropSection !== section) return null;
      const state = TASK_STATES.find((candidate) => candidate === target.dataset.dropState);
      if (state === undefined || (state === row.state && !nested)) return null;
      return { aim: { state, under: null }, element: target };
    };

    /**
     * Where the task will sit once it lands: below the row it nests under, or in a state's group or column among its
     * top-level rows ordered by title, which opens a place there; an empty one's hint stands where it will sit.
     */
    let gap: { element: HTMLElement; side: "marginTop" | "paddingBottom" } | null = null;
    const ownPlace = () => rest(element, zoom);
    let slot = ownPlace;
    const closeGap = () => {
      if (gap) gap.element.style[gap.side] = "";
      [gap, slot] = [null, ownPlace];
    };
    const below = (under: HTMLElement) => {
      slot = () => ({
        left: free ? rest(under, zoom).left : origin.left,
        top: rest(under, zoom).top + under.offsetHeight * zoom,
      });
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
        const under = parseFloat(style.paddingBottom) + parseFloat(style.borderBottomWidth) + room;
        gap = { element: group, side: "paddingBottom" };
        group.style.paddingBottom = `${parseFloat(style.paddingBottom) + room}px`;
        slot = () => ({ left: left(), top: group.getBoundingClientRect().bottom - under * zoom + margin });
      }
    };

    /** Heads the task toward `target`, or back to its own place at null, the list making room as it goes. */
    const aim = (target: { aim: Aim; element: HTMLElement } | null, dropped: boolean) => {
      const was = hand();
      if (was === null) return;
      const toward = target?.aim ?? null;
      const turned = toward?.state !== was.toward?.state || toward?.under?.key !== was.toward?.under?.key;
      if (!turned && dropped === was.dropped) return;
      if (turned) closeGap();
      setHand({ ...was, toward, dropped, away: was.away || toward?.under === null });
      const thins = target !== null && (!list.contains(target.element) || (target.aim.under !== null && !dropped));
      ghost.classList.toggle("aiming", thins);
      if (target !== null && turned) {
        if (target.aim.under === null) openGap(target.aim.state);
        else below(target.element);
      }
      followChange(list);
      if (toward !== null && turned && !dropped) feel("select");
    };

    let last = from;
    const pointer = chase((at) => {
      follow(at, from, (free ? surface.stage : list).getBoundingClientRect(), free ? "anywhere" : "column");
      aim(targetAt(at), false);
    });

    /** Flies the ghost back onto its own row, wherever the list has moved it. */
    const home = () => {
      pointer.stop();
      setLanding([]);
      aim(null, true);
      setHand((held) => held && { ...held, away: false });
      ghost.classList.remove("up");
      const finish = () => {
        ghost.remove();
        end();
      };
      if (reduced() || !element.isConnected) return finish();
      void fly(ownPlace()).then((arrived) => arrived && finish());
    };
    /** Whether the task's row stands where `toward` puts it: in its state's group, under the parent it heads to. */
    const placed = (toward: Aim) => {
      const under = toward.under === null ? (nested ? null : parent) : toward.under.task.id;
      return (arrival: HTMLElement) =>
        stateOf(arrival) === toward.state && (model.tasks[row.key]?.task.parent ?? null) === under;
    };
    /** Lands the ghost where the task's rows arrive, the place held open for them until then. */
    const land = (toward: Aim) => {
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
      const settle = (spot: Spot) =>
        void fly(spot).then((there) => {
          settled = there;
          show();
        });
      settle(slot());
      const stop = awaitArrival(row.key, placed(toward), (arrival) => {
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
    const into = (tab: Element, toward: Aim) => {
      const box = tab.getBoundingClientRect();
      void fly({
        left: box.left + box.width / 2 - origin.width / 2,
        top: box.top + box.height / 2 - origin.height / 2,
      });
      const shrink = ghost.animate([{ opacity: 0, scale: 0.3 }], curve(SNAPPY));
      shrink.finished.then(
        () => ghost.remove(),
        () => ghost.remove(),
      );
      let arrived = false;
      const stop = awaitArrival(row.key, placed(toward), () => {
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
    /** Nests the task under the task it heads to, or brings a nested one to the top level and then to its state. */
    const send = async (toward: Aim) => {
      if (toward.under !== null) return backend.taskSetParent(row.at, toward.under.at);
      if (nested) await backend.taskSetParent(row.at, null);
      if (toward.state !== row.state) await backend.taskMove(row.at, toward.state);
    };

    /** Where the pointer's throw carries it, kept within the surface. */
    const projected = () => {
      const speed = motion.velocity();
      const bounds = (free ? surface.stage : list).getBoundingClientRect();
      return {
        x: Math.min(Math.max(last.x + speed.x * zoom * PROJECT_S, bounds.left + 1), bounds.right - 1),
        y: Math.min(Math.max(last.y + speed.y * zoom * PROJECT_S, bounds.top + 1), bounds.bottom - 1),
      };
    };

    return {
      move: (to) => {
        last = to;
        pointer.move(to);
      },
      drop: () => {
        pointer.stop();
        const target = targetAt(throws ? projected() : last);
        if (target === null) return home();
        aim(target, true);
        feel("drop");
        const refused = list.contains(target.element) ? land(target.aim) : into(target.element, target.aim);
        let failed = false;
        const fail = () => {
          if (failed || ending !== takeOver) return;
          failed = true;
          refused();
        };
        setTimeout(fail, LANDS_MS);
        send(target.aim).catch((error) => {
          fail();
          report(error);
        });
      },
      cancel: home,
    };
  };

/**
 * Lifts a section by its header as a ghost that follows the pointer up and down the list, the section folding to its
 * header once the pointer travels, and the other sections making room wherever it would land. Letting go over the
 * list saves the order and lands the ghost on the header's new place; letting go off the list, or cancelling, flies
 * it home.
 */
export const liftSection =
  (name: string): Start =>
  (element, from, surface) => {
    const { list, covered, zoom } = surface;
    ending?.();
    const order = sections().map((section) => section.name);
    const { ghost, fly, follow } = raise(element, surface);
    setHeldSection({ name, folded: false });
    const finish = () => {
      ghost.remove();
      end();
    };
    const end = () => {
      if (ending !== finish) return;
      ending = null;
      arrange(null);
      setHeldSection(null);
      followChange(list);
    };
    ending = finish;

    /** Puts the section after every other section whose middle stands above `center`. */
    const arrangeAt = (center: number) => {
      const above = [...list.querySelectorAll<HTMLElement>("[data-section]")].filter((section) => {
        const box = section.getBoundingClientRect();
        return section.dataset.section !== name && box.top + box.height / 2 < center;
      });
      const next = moveTo(order, name, above.length);
      const now = arranged() ?? order;
      if (next.some((each, nth) => each !== now[nth])) arrange(next);
    };
    let last = from;
    const pointer = chase((at) => {
      follow(at, from, list.getBoundingClientRect(), "column");
      if (!heldSection()?.folded && Math.abs(at.y - from.y) >= FOLD_PX) {
        setHeldSection({ name, folded: true });
        followChange(list);
      }
      const box = ghost.getBoundingClientRect();
      arrangeAt(box.top + box.height / 2);
    });

    const settle = (saved: Promise<unknown>) => {
      pointer.stop();
      ghost.classList.remove("up");
      if (reduced()) return void saved.then(finish);
      void Promise.all([fly(rest(element, zoom)), saved]).then(([arrived]) => arrived && finish());
    };
    const home = () => {
      arrange(null);
      settle(Promise.resolve());
    };
    const overList = (at: Point) => {
      const box = list.getBoundingClientRect();
      return !covered(at) && at.x >= box.left && at.x < box.right && at.y >= box.top && at.y < box.bottom;
    };

    return {
      move: (to) => {
        last = to;
        pointer.move(to);
      },
      drop: () => {
        const moved = arranged();
        if (moved === null || !overList(last)) return home();
        surface.feel("drop");
        settle(orderSections(placeAmong(ui.prefs.section_order, moved, name)));
      },
      cancel: home,
    };
  };

const sameOrder = (a: readonly string[], b: readonly string[]) =>
  a.length === b.length && a.every((key, nth) => key === b[nth]);

/**
 * Lifts a tab as a ghost that follows the pointer along its row, the other tabs making room wherever it would land.
 * `keys` tells the tabs' own order, and `arrange` shows them in another one while the tab is held, or in their own at
 * null. Letting go moves the tab to its new index and lands the ghost there once the tabs show in that order; cancelling
 * flies it home.
 */
export const liftTab =
  (
    key: string,
    keys: () => string[],
    arrange: (order: string[] | null) => void,
    move: (to: number) => Promise<unknown>,
  ): Start =>
  (element, from, surface) => {
    const { list, zoom } = surface;
    ending?.();
    const own = keys();
    let order = own;
    let at = own.indexOf(key);
    const scrolled = list.scrollLeft;
    /** Where a tab rests along the row's scrolled content, in the layout's pixels. */
    const along = (tab: HTMLElement) => ({
      left: rest(tab, zoom).left / zoom + scrolled,
      width: tab.getBoundingClientRect().width / zoom,
    });
    const tabs = new Map(
      [...list.querySelectorAll<HTMLElement>("[data-key]:not(.flip-ghost)")].map((tab) => [tab.dataset.key!, tab]),
    );
    const held = along(element);
    /** The other tabs' middles along the row with the held tab taken out. */
    const middles = own
      .filter((each) => each !== key)
      .map((each) => {
        const { left, width } = along(tabs.get(each)!);
        return left + width / 2 - (left > held.left ? held.width : 0);
      });
    const { ghost, origin, fly, follow } = raise(element, surface);
    element.classList.add("lifting");
    const finish = () => {
      ghost.remove();
      end();
    };
    const end = () => {
      if (ending !== finish) return;
      ending = null;
      element.classList.remove("lifting");
      arrange(null);
    };
    ending = finish;

    const pointer = chase((to) => {
      const scroll = list.scrollLeft;
      const { x } = follow(to, from, list.getBoundingClientRect(), "row");
      const next = slotAmong(middles, origin.left / zoom + x + scroll, at);
      if (next !== at) arrange((order = moveTo(own, key, (at = next))));
    });

    const settle = (landed: Promise<unknown>) => {
      pointer.stop();
      ghost.classList.remove("up");
      if (reduced()) return void landed.then(finish);
      void Promise.all([fly(rest(element, zoom)), landed]).then(([arrived]) => arrived && finish());
    };
    const home = () => {
      order = own;
      arrange(null);
      settle(Promise.resolve());
    };
    /** Resolves once the tabs show in the order the drop gave them, the move fails, or the wait runs out. */
    const landed = (to: number) =>
      new Promise<void>((resolve) =>
        createRoot((dispose) => {
          const land = () => {
            dispose();
            resolve();
          };
          setTimeout(land, LANDS_MS);
          createEffect(() => sameOrder(keys(), order) && land());
          move(to).catch((error) => {
            land();
            report(error);
          });
        }),
      );

    return {
      move: (to) => pointer.move(to),
      drop: () => {
        const to = order.indexOf(key);
        if (to === own.indexOf(key)) return home();
        surface.feel("drop");
        settle(landed(to));
      },
      cancel: home,
    };
  };
