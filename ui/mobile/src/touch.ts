import { tracker } from "@octopus/shared/motion.ts";
import { cancelPresses, type Point } from "./press";

/** How far a touch travels before it commits to the axis it moved along most. */
export const COMMIT_PX = 10;

type Axis = "x" | "y";

/**
 * A touch as it commits: where it landed, where it stands now, its axis, and which way along it it went, 1 toward the
 * right or the bottom and -1 toward the left or the top.
 */
type Committed = { from: Point; at: Point; axis: Axis; way: 1 | -1 };

/** A gesture holding a touch follows the finger, and ends with its velocity in pixels a second, or null if cancelled. */
type Held = { move: (to: Point) => void; end: (velocity: Point | null) => void };

/** Takes a committed touch by returning what follows it, or leaves it with null. */
export type Gesture = (touch: Committed) => Held | null;

const gestures = new WeakMap<Element, Gesture>();
const screenEdges = new Set<Gesture>();

/** Lets `gesture` take touches that commit while on `element`, ahead of the scrollers around it. */
export const owns = (element: Element, gesture: Gesture) => gestures.set(element, gesture);

/** Lets `gesture` take touches before anything under the finger does, as the OS's screen-edge gestures do. */
export const ownsScreenEdge = (gesture: Gesture) => screenEdges.add(gesture);

/** Whether `element` scrolls natively along `axis`, which it does even at its end, where it rubber-bands. */
function scrolls(element: Element, axis: Axis) {
  const overflow = getComputedStyle(element)[axis === "x" ? "overflowX" : "overflowY"];
  if (overflow !== "auto" && overflow !== "scroll") return false;
  return axis === "x" ? element.scrollWidth > element.clientWidth : element.scrollHeight > element.clientHeight;
}

/**
 * The innermost thing on the committed axis: a gesture that takes the touch, "native" for a scroller, or "free" for
 * none, which leaves the touch a tap the finger can slide off and back onto.
 */
function owner(target: Element, touch: Committed): Held | "native" | "free" {
  for (const gesture of screenEdges) {
    const held = gesture(touch);
    if (held) return held;
  }
  for (let element: Element | null = target; element !== null; element = element.parentElement) {
    const held = gestures.get(element)?.(touch);
    if (held) return held;
    if (scrolls(element, touch.axis)) return "native";
  }
  return "free";
}

const ROW = ".row, .card, .tree, .mi";
/** What a finger taps: every control, every row of a list or menu, and a sheet's scrim. */
const TAPPABLE = `${ROW}, button, [role=button], .key, .scrim`;
const DISABLED = ".off, [aria-disabled=true], :disabled, .inert";
/** A row in a list with room to scroll waits this long before it shows a press, so a finger that scrolls shows none. */
const ROW_DELAY_MS = 60;
/** How far past its edges a finger can slide and still tap what it pressed, as far as UIKit's controls reach. */
const REACH_PX = 70;

type Tap = { element: HTMLElement; on: boolean; shown: boolean; timer?: ReturnType<typeof setTimeout> };

const show = (tap: Tap, on: boolean) => {
  tap.on = on;
  tap.shown ||= on;
  tap.element.toggleAttribute("data-pressed", on);
};

const inScroller = (element: Element) => {
  for (let view = element.parentElement; view !== null; view = view.parentElement)
    if (scrolls(view, "x") || scrolls(view, "y")) return true;
  return false;
};

function press(target: Element): Tap | null {
  const element = target.closest<HTMLElement>(TAPPABLE);
  if (!element || element.matches(DISABLED)) return null;
  const tap: Tap = { element, on: true, shown: false };
  if (element.matches(ROW) && inScroller(element))
    tap.timer = setTimeout(() => tap.on && show(tap, true), ROW_DELAY_MS);
  else show(tap, true);
  return tap;
}

const unpress = (tap: Tap) => {
  clearTimeout(tap.timer);
  tap.element.removeAttribute("data-pressed");
};

export const reaches = (element: Element, at: Point, margin: number) => {
  const box = element.getBoundingClientRect();
  return (
    at.x >= box.left - margin && at.x <= box.right + margin && at.y >= box.top - margin && at.y <= box.bottom + margin
  );
};

export const fingerIn = (touches: TouchList, id: number | null | undefined) =>
  [...touches].find((each) => each.identifier === id);

/**
 * Whether WebKit's mouse events for the last tap are still to come, which they are for a lift that never moved, and
 * would land on whatever the tap opened under the finger. The tap's touchend stays unprevented, as WebKit raises the
 * keyboard for a field the tap focuses only then.
 */
let synthesizing = false;

/** A tap lifted before its press showed still flashes it for a frame, so every tap is seen. */
const tapped = (tap: Tap) => {
  clearTimeout(tap.timer);
  if (tap.shown) tap.element.removeAttribute("data-pressed");
  else {
    show(tap, true);
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        if (touch?.tap?.element !== tap.element) tap.element.removeAttribute("data-pressed");
      }),
    );
  }
  tap.element.click();
};

type Tracked = {
  id: number;
  from: Point;
  target: Element;
  owner: "undecided" | "native" | "free" | "kept" | Held;
  tap: Tap | null;
  track: ReturnType<typeof tracker>;
};
let touch: Tracked | null = null;

const dropTap = (tracked: Tracked) => {
  if (tracked.tap) unpress(tracked.tap);
  tracked.tap = null;
};

/** Gives the touch under way to its caller, which follows the finger on its own; nothing else takes it until lift. */
export const keepTouch = () => {
  if (touch === null || typeof touch.owner === "object") return;
  touch.owner = "kept";
  dropTap(touch);
};

/** Touch events keep going to the element a touch started on, even once it leaves the page, so they are heard there. */
const listen = ({ target }: Tracked) => {
  target.addEventListener("touchmove", move as EventListener, { passive: false });
  target.addEventListener("touchend", end as EventListener);
  target.addEventListener("touchcancel", end as EventListener);
};
const release = (done: Tracked) => {
  done.target.removeEventListener("touchmove", move as EventListener);
  done.target.removeEventListener("touchend", end as EventListener);
  done.target.removeEventListener("touchcancel", end as EventListener);
  touch = null;
};
const cancel = (stale: Tracked) => {
  release(stale);
  dropTap(stale);
  if (typeof stale.owner === "object") stale.owner.end(null);
};

const move = (event: TouchEvent) => {
  const finger = fingerIn(event.touches, touch?.id);
  if (touch === null || finger === undefined || touch.owner === "native" || touch.owner === "kept") return;
  const at = { x: finger.clientX, y: finger.clientY };
  touch.track.add(at.x, at.y);
  if (touch.owner === "undecided") {
    const [dx, dy] = [at.x - touch.from.x, at.y - touch.from.y];
    if (Math.hypot(dx, dy) < COMMIT_PX) return;
    const axis = Math.abs(dx) > Math.abs(dy) ? "x" : "y";
    const way = (axis === "x" ? dx : dy) > 0 ? 1 : -1;
    touch.owner = owner(touch.target, { from: touch.from, at, axis, way });
    if (touch.owner === "free" && touch.tap === null) touch.owner = "native";
    if (touch.owner !== "free") dropTap(touch);
    if (touch.owner === "native") return;
    if (touch.owner !== "free") cancelPresses();
  }
  event.preventDefault();
  if (touch.owner !== "free") touch.owner.move(at);
  else if (touch.tap) show(touch.tap, reaches(touch.tap.element, at, REACH_PX));
};

const end = (event: TouchEvent) => {
  if (touch === null || fingerIn(event.changedTouches, touch.id) === undefined) return;
  const { owner: held, track, tap } = touch;
  release(touch);
  synthesizing = tap !== null && event.type === "touchend" && held === "undecided";
  if (tap && event.type === "touchend" && tap.on) tapped(tap);
  else if (tap) unpress(tap);
  // The mouse events a lift synthesizes would land on whatever the hold or gesture left under the finger.
  if (held === "kept" || typeof held === "object") event.preventDefault();
  if (typeof held === "object") held.end(event.type === "touchcancel" ? null : track.velocity());
};

/**
 * Decides which one thing owns each touch: within its first 10pt it commits to an axis, and the innermost gesture or
 * native scroller on that axis has it until lift. A gesture holding a touch keeps the page from scrolling under it.
 *
 * Until then, and for good when nothing takes it, the touch is a tap, as UIKit's are: what it lands on shows pressed,
 * a row in a list with room to scroll a beat later, and lifting on it clicks it. Sliding off unpresses it and sliding
 * back presses it again. WebKit's own mouse events after a tap never arrive, so a tap acts the same whatever moved under
 * the finger; assistive technology, which clicks without touching, still clicks.
 */
export function followTouches() {
  addEventListener(
    "touchstart",
    (event) => {
      synthesizing = false;
      if (event.touches.length > 1) return keepTouch();
      const first = event.changedTouches[0];
      if (touch) cancel(touch);
      if (first === undefined || !(event.target instanceof Element)) return;
      const from = { x: first.clientX, y: first.clientY };
      const tap = press(event.target);
      touch = { id: first.identifier, from, target: event.target, owner: "undecided", tap, track: tracker() };
      touch.track.add(from.x, from.y);
      listen(touch);
    },
    { capture: true, passive: true },
  );
  for (const synthesized of ["mousedown", "click"])
    addEventListener(
      synthesized,
      (event) => {
        if (!synthesizing || !event.isTrusted) return;
        synthesizing = synthesized === "mousedown";
        event.preventDefault();
        event.stopImmediatePropagation();
      },
      { capture: true },
    );
}
