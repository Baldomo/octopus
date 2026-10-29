import { animate, SNAPPY, type Running } from "./motion";

/** How far a surface dragged `over` past its edge follows the finger: about half as far, less the further it goes. */
const GIVE = 0.55;
const GIVE_SPAN_PX = 600;
export const give = (over: number) => (over * GIVE) / (1 + Math.abs(over) / GIVE_SPAN_PX);
const ungive = (given: number) => given / (GIVE - Math.abs(given) / GIVE_SPAN_PX);
/** `at` with `bend` applied to however far it stands past either end of 0 to `end`. */
const beyond = (at: number, end: number, bend: (over: number) => number) =>
  at < 0 ? bend(at) : at > end ? end + bend(at - end) : at;

/** How much of its speed a thrown scroll keeps each millisecond, as iOS's scroll views do. */
const LN_DECAY = Math.log(0.998);

/**
 * One axis a finger scrolls: where it stands, its far end past 0, where a throw toward a point comes to rest, and what
 * moves it there.
 */
export type Scroll = { at: () => number; end: () => number; rest: (at: number) => number; apply: (at: number) => void };

/**
 * Coasts from `from` at `velocity` units a second, slowing to rest where `scroll.rest` puts the throw; a throw that
 * reaches an edge carries its speed into a spring back to it, and one that starts past an edge springs straight back.
 */
function coast(scroll: Scroll, from: number, velocity: number): Running {
  const end = scroll.end();
  const edge = Math.min(Math.max(from, 0), end);
  if (edge !== from) return animate(from, edge, scroll.apply, SNAPPY, velocity);
  const thrown = from - velocity / 1000 / LN_DECAY;
  const target = thrown < 0 || thrown > end ? thrown : scroll.rest(thrown);
  const speed = (target - from) * -LN_DECAY;
  let frame = 0;
  let bounce: Running | null = null;
  let finish!: () => void;
  const done = new Promise<void>((resolve) => (finish = resolve));
  const start = performance.now();
  const tick = (now: number) => {
    const kept = Math.exp(LN_DECAY * (now - start));
    const at = from + (speed * (kept - 1)) / LN_DECAY;
    if (at < 0 || at > end) {
      const bound = at < 0 ? 0 : end;
      bounce = animate(bound, bound, scroll.apply, SNAPPY, speed * kept * 1000);
      return void bounce.done.then(finish);
    }
    if (Math.abs(target - at) < 0.5) {
      scroll.apply(target);
      return finish();
    }
    scroll.apply(at);
    frame = requestAnimationFrame(tick);
  };
  frame = requestAnimationFrame(tick);
  return {
    stop: () => {
      cancelAnimationFrame(frame);
      bounce?.stop();
      finish();
    },
    done,
  };
}

/**
 * Scrolls `scroll` as iOS scrolls a view: a drag carries it one to one and gives way past its edges, and letting go
 * coasts on the throw. Touching it stops a coast where it stands, and a touch that lets go without dragging settles it.
 */
export function scroller(scroll: Scroll) {
  let running: Running | null = null;
  const stop = () => running?.stop();
  const release = (velocity: number) => {
    stop();
    running = coast(scroll, scroll.at(), velocity);
  };
  return {
    stop,
    settle: () => release(0),
    grab: () => {
      stop();
      const end = scroll.end();
      const from = beyond(scroll.at(), end, ungive);
      return {
        drag: (by: number) => scroll.apply(beyond(from + by, end, give)),
        release,
      };
    },
  };
}
