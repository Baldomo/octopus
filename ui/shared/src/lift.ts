import { animate, give, SNAPPY, tracker, type Running } from "./motion";

export type Point = { x: number; y: number };
export type Spot = { left: number; top: number };

export type Lift = { move: (to: Point) => void; drop: () => void; cancel: () => void };

/** `zoom` is how many screen pixels one of the layout's takes; `covered` points take no drop. */
type Surface = {
  stage: HTMLElement;
  list: HTMLElement;
  free: boolean;
  throws: boolean;
  covered: (at: Point) => boolean;
  zoom: number;
  feel: (kind: "select" | "drop") => void;
};

export type Start = (element: HTMLElement, from: Point, surface: Surface) => Lift;

export type Within = { stage: string; list: string; free: boolean };

export type LiftOnDrag = (
  within: Within,
  start: Start,
) => { onPointerDown: (down: PointerEvent & { currentTarget: HTMLElement }) => void };

const EDGE_PX = 48;
const EDGE_SCROLL_PX = 12;
export const LANDS_MS = 1500;

function edgePush(at: number, start: number, end: number) {
  const near = (distance: number) => (EDGE_SCROLL_PX * Math.max(0, EDGE_PX - distance)) / EDGE_PX;
  return near(end - at) - near(at - start);
}

const scrolls = (overflow: string) => overflow === "auto" || overflow === "scroll";

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

const within = (at: number, min: number, max: number) =>
  at < min ? min + give(at - min) : at > max ? max + give(at - max) : at;

/** Where a list's item lays out on the screen, without the slide it may be in the middle of. */
export function rest(element: Element, zoom: number): Spot {
  const box = element.getBoundingClientRect();
  const slide = new DOMMatrix(getComputedStyle(element).transform);
  return { left: box.left - slide.m41 * zoom, top: box.top - slide.m42 * zoom };
}

type Along = "column" | "row" | "anywhere";

let ending: (() => void) | null = null;

export const endLift = () => ending?.();

export function holdLift(end: () => void) {
  ending = end;
}

export const underWay = (end: () => void) => ending === end;

export function release(end: () => void) {
  if (ending !== end) return false;
  ending = null;
  return true;
}

/** `fly` resolves whether the ghost got there before another flight took over. */
export function raise(element: HTMLElement, { stage, zoom }: Surface) {
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
      left: { column: origin.left, row: within(left, bounds.left, bounds.right - origin.width), anywhere: left }[along],
      top: along === "row" ? origin.top : within(top, bounds.top, bounds.bottom - origin.height),
    });
    place(spot.x, spot.y);
    return spot;
  };
  return { ghost, origin, motion, fly, follow };
}

export function chase(follow: (pointer: Point) => void) {
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
