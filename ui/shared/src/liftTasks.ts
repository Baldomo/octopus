import { createRoot, createSelector, createSignal } from "solid-js";
import { backend } from "./backend";
import { awaitArrival, followChange } from "./flip";
import { TASK_STATES, type Row, type TaskState, type TaskView } from "./generated";
import {
  chase,
  endLift,
  holdLift,
  LANDS_MS,
  type Point,
  raise,
  release,
  rest,
  type Spot,
  type Start,
  underWay,
} from "./lift";
import { curve, play, PROJECT_S, reduced, SNAPPY } from "./motion";
import { moveTo, placeAmong } from "./sectionOrder";
import { account, arrange, arranged, model, orderSections, report, sectionView, sections } from "./state";

type Aim = { state: TaskState; under: TaskView | null };

/** Rows stay `away` from the first time the task heads to a state until it flies home, so the list shifts once at most. */
type Hand = {
  key: string;
  section: string;
  carried: string[];
  toward: Aim | null;
  away: boolean;
  dropped: boolean;
};
const [hand, setHand] = createSignal<Hand | null>(null);
const [landing, setLanding] = createSignal<string[]>([]);
const [heldSection, setHeldSection] = createSignal<{ name: string; folded: boolean } | null>(null);

const NEST_EDGE = 0.25;
/** A hold that stays put leaves its section open. */
const FOLD_PX = 4;

export const isLifted = createRoot(() => createSelector(() => hand()?.key ?? null));
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

export const liftsIn = (section: string) => {
  const held = hand();
  return held?.section === section && (!held.dropped || held.toward !== null);
};

export const dropTarget = (section: string, state: TaskState) => ({
  over: () => {
    const held = hand();
    return held?.section === section && !held.dropped && held.toward?.state === state && held.toward.under === null;
  },
  attributes: { "data-drop-section": section, "data-drop-state": state },
});

const stateOf = (element: Element | null | undefined) =>
  TASK_STATES.find((state) => state === element?.closest<HTMLElement>("[data-state]")?.dataset.state);

const footprint = (element: HTMLElement) => element.offsetHeight + parseFloat(getComputedStyle(element).marginTop);

function sortsBefore(a: Row, b: Row) {
  const order = (row: Row) => [row.title.toLowerCase(), row.at.server, row.at.task];
  const [left, right] = [order(a), order(b)];
  const differs = left.findIndex((part, nth) => part !== right[nth]);
  return differs >= 0 && left[differs]! < right[differs]!;
}

export const lift =
  (row: Row, section: string): Start =>
  (element, from, surface) => {
    const { list, free, throws, covered, zoom, feel } = surface;
    endLift();
    const shown = sectionView(section);
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
    /** The task's own place takes no drop until its rows give it up. */
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
      if (!release(takeOver)) return;
      setLanding([]);
      setHand(null);
    };
    holdLift(takeOver);

    /** undefined off the row's middle, where the row's group decides. */
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
    const placed = (toward: Aim) => {
      const under = toward.under === null ? (nested ? null : parent) : toward.under.task.id;
      return (arrival: HTMLElement) =>
        stateOf(arrival) === toward.state && (model.tasks[row.key]?.task.parent ?? null) === under;
    };
    const land = (toward: Aim) => {
      setLanding(keys);
      ghost.classList.remove("up");
      let [arrived, settled] = [false, false];
      const show = () => {
        if (!arrived || !settled || !underWay(takeOver)) return;
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
        if (!underWay(takeOver)) return;
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
    const send = async (toward: Aim) => {
      if (toward.under !== null) return backend.taskSetParent(row.at, toward.under.at);
      if (nested) await backend.taskSetParent(row.at, null);
      if (toward.state !== row.state) await backend.taskMove(row.at, toward.state);
    };

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
          if (failed || !underWay(takeOver)) return;
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

export const liftSection =
  (name: string): Start =>
  (element, from, surface) => {
    const { list, covered, zoom } = surface;
    endLift();
    const order = sections().map((section) => section.name);
    const { ghost, fly, follow } = raise(element, surface);
    setHeldSection({ name, folded: false });
    const finish = () => {
      ghost.remove();
      end();
    };
    const end = () => {
      if (!release(finish)) return;
      arrange(null);
      setHeldSection(null);
      followChange(list);
    };
    holdLift(finish);

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
        settle(orderSections(placeAmong(account().settings.section_order, moved, name)));
      },
      cancel: home,
    };
  };
