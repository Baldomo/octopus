import { onCleanup } from "solid-js";
import { curve, play, reach, reduced, SNAPPY } from "./motion";
type Spot = { x: number; y: number; width: number; height: number; element: HTMLElement; ws: string };
type Mover = { spot: Spot; dx: number; dy: number };

/** Rows a drop waits for in the group or column of a state, each told where it arrived before its list measures. */
const arrivals = new Map<string, { state: string; arrive: (element: HTMLElement) => void }>();

/** Waits for the row keyed `key` to show in `state`'s group or column; the returned function stops waiting. */
export function awaitArrival(key: string, state: string, arrive: (element: HTMLElement) => void) {
  arrivals.set(key, { state, arrive });
  return () => arrivals.get(key)?.arrive === arrive && arrivals.delete(key);
}

const lists = new WeakMap<Element, () => void>();
/** Slides a list's items to where a change it cannot see, to a class or a style, put them. */
export const followChange = (list: Element) => lists.get(list)?.();

const GHOST = "flip-ghost";
/** A removed item fades out quicker than the rest move in, so it clears the place they take. */
const LEAVE_MS = 120;
/** How much of a new item a neighbour sliding off its place may still cover as the item starts to fade in. */
const OVERLAP = 0.4;

/** How far through its slide `mover` must get to clear `spot` along one axis, from 0 for at once to 1 for never. */
function clears(start: number, end: number, moverStart: number, moverEnd: number, off: number) {
  const cover = OVERLAP * (end - start);
  if (moverStart >= end - 0.5 && off < 0) return 1 - (moverStart - end + cover) / -off;
  if (moverEnd <= start + 0.5 && off > 0) return 1 - (start - moverEnd + cover) / off;
  return 0;
}

/** How far the neighbours sliding across a new item's place must get before it fades in, so no two overlap. */
function clearance(spot: Spot, movers: Mover[]) {
  let share = 0;
  for (const { spot: moved, dx, dy } of movers) {
    const sideBySide = moved.y < spot.y + spot.height && moved.y + moved.height > spot.y;
    const stacked = moved.x < spot.x + spot.width && moved.x + moved.width > spot.x;
    if (stacked) share = Math.max(share, clears(spot.y, spot.y + spot.height, moved.y, moved.y + moved.height, dy));
    if (sideBySide) share = Math.max(share, clears(spot.x, spot.x + spot.width, moved.x, moved.x + moved.width, dx));
  }
  return share;
}

/**
 * A list whose items, `selector` within it, move to where a change puts them, fade in where one adds them once the
 * items sliding off that place have cleared it, and fade out where one removes them, each keyed by its `data-key` or
 * else by itself. It follows every change to its children before the change paints. An item held `.landing` is left
 * for its drop to place, and an item that lays out nowhere is left out. Under reduced motion items only fade.
 */
export function animateList(selector: string) {
  return (root: HTMLElement) => {
    root.classList.add("flips");
    let spots = new Map<unknown, Spot>();
    let ready = false;
    const moving = new WeakMap<Element, Animation>();
    const keyOf = (element: HTMLElement) => element.dataset.key ?? element;
    const items = () =>
      [...root.querySelectorAll<HTMLElement>(selector)].filter(
        (element) => !element.classList.contains(GHOST) && element.getClientRects().length > 0,
      );
    /** Where an item lays out within the list, untouched by its own animation or the list's scroll. */
    const where = (element: HTMLElement) => {
      let [x, y] = [0, 0];
      for (let at: HTMLElement | null = element; at !== null && at !== root; at = at.offsetParent as HTMLElement) {
        x += at.offsetLeft;
        y += at.offsetTop;
      }
      return { x, y };
    };

    /** Slides an item from `dx`, `dy` off its place, on from wherever a slide still running has it. */
    const move = (element: HTMLElement, dx: number, dy: number) => {
      if (reduced() || (Math.abs(dx) < 0.5 && Math.abs(dy) < 0.5)) return null;
      const running = moving.get(element);
      if (running?.playState === "running") {
        const now = new DOMMatrix(getComputedStyle(element).transform);
        [dx, dy] = [dx + now.m41, dy + now.m42];
        running.cancel();
      }
      moving.set(
        element,
        element.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: "none" }], curve(SNAPPY)),
      );
      return { dx, dy };
    };
    const enter = (element: HTMLElement, after: number) =>
      play(
        element,
        [
          { opacity: 0, transform: "translateY(-6px)" },
          { opacity: 1, transform: "none" },
        ],
        SNAPPY,
        reach(SNAPPY, after),
      );
    const leave = (spot: Spot) => {
      const ghost = spot.element;
      ghost.classList.add(GHOST);
      Object.assign(ghost.style, {
        left: `${spot.x}px`,
        top: `${spot.y}px`,
        width: `${spot.width}px`,
        height: `${spot.height}px`,
      });
      ghost.style.setProperty("--ws", spot.ws);
      root.append(ghost);
      ghost.animate([{ opacity: 1 }, { opacity: 0, transform: "scale(0.98)" }], LEAVE_MS).finished.then(
        () => ghost.remove(),
        () => ghost.remove(),
      );
    };

    /** A list out of sight lays out nowhere, so it starts over once it shows again. */
    const follow = () => {
      if (root.getClientRects().length === 0) {
        [spots, ready] = [new Map(), false];
        return;
      }
      for (const [key, arrival] of arrivals) {
        const element = root.querySelector<HTMLElement>(`[data-key="${CSS.escape(key)}"]`);
        if (element === null || element.closest<HTMLElement>("[data-state]")?.dataset.state !== arrival.state) continue;
        arrivals.delete(key);
        arrival.arrive(element);
      }
      const next = new Map<unknown, Spot>();
      for (const element of items()) {
        const key = keyOf(element);
        const ws = spots.get(key)?.ws ?? getComputedStyle(element).getPropertyValue("--ws");
        next.set(key, { ...where(element), width: element.offsetWidth, height: element.offsetHeight, element, ws });
      }
      if (ready) {
        const [movers, entering] = [[] as Mover[], [] as Spot[]];
        for (const [key, spot] of next) {
          if (spot.element.classList.contains("landing")) continue;
          const from = spots.get(key);
          if (from === undefined) entering.push(spot);
          else {
            const moved = move(spot.element, from.x - spot.x, from.y - spot.y);
            if (moved) movers.push({ spot, ...moved });
          }
        }
        for (const spot of entering) enter(spot.element, clearance(spot, movers));
        for (const [key, spot] of spots) if (!next.has(key) && !spot.element.isConnected) leave(spot);
      }
      [spots, ready] = [next, true];
    };

    lists.set(root, follow);
    const changes = new MutationObserver(follow);
    changes.observe(root, { childList: true, subtree: true });
    onCleanup(() => changes.disconnect());
    requestAnimationFrame(follow);
  };
}
