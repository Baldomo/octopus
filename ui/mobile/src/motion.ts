import { getOwner, onCleanup, onMount, runWithOwner } from "solid-js";
import { curve, SNAPPY, standing, type Spring } from "@octopus/shared/motion.ts";

/** An exit takes this share of its entry's time. */
const EXIT = 0.8;
/** A screen pushes in, and pops out. */
export const PUSH: Spring = { response: 0.32, damping: 1 };
export const POP: Spring = { response: PUSH.response * EXIT, damping: 1 };
/** A sheet rises and settles, and leaves. */
export const SHEET: Spring = { response: 0.28, damping: 1 };
export const LEAVE: Spring = { response: SHEET.response * EXIT, damping: 1 };
/** The foot of the screen riding the on-screen keyboard, on a curve near the keyboard's own. */
export const KEYBOARD: Spring = { response: 0.3, damping: 1 };

/** What a value draws: each element with the inline styles it takes at that value, the same elements at every value. */
export type Look = [element: HTMLElement, style: Record<string, string>][];

export const draw = (look: Look) => {
  for (const [element, style] of look) Object.assign(element.style, style);
};

export type Gliding = { stop: () => number; done: Promise<void> };

const FRAME_S = 1 / 60;

/**
 * Moves a value like `animate`, drawing `look` of it, but hands the whole move to the compositor up front as keyframes,
 * one a frame, so work on the page's thread never stalls it. It ends drawn at `to`; stopping draws it where it stands
 * and returns that value.
 */
export function glide(from: number, to: number, look: (value: number) => Look, spring: Spring, velocity = 0): Gliding {
  const values = [from];
  for (let t = FRAME_S; ; t += FRAME_S) {
    const at = standing(spring, from, to, velocity, t);
    if (at === null) break;
    values.push(at);
  }
  values.push(to);
  const looks = values.map((value) => look(value));
  const duration = (values.length - 1) * FRAME_S * 1000;
  const moves =
    from === to && velocity === 0
      ? []
      : looks[0]!.map(([element], nth) =>
          element.animate(
            looks.map((each) => each[nth]![1]),
            { duration, easing: "linear", fill: "forwards" },
          ),
        );
  let finish!: () => void;
  const done = new Promise<void>((resolve) => (finish = resolve));
  let ended = false;
  const end = (value: number) => {
    if (!ended) {
      ended = true;
      draw(look(value));
      for (const move of moves) move.cancel();
      finish();
    }
    return value;
  };
  if (moves.length === 0) end(to);
  else
    moves[0]!.finished.then(
      () => end(to),
      () => {},
    );
  return {
    stop: () => {
      if (ended) return to;
      const frames = Number(moves[0]!.currentTime ?? 0) / 1000 / FRAME_S;
      const nth = Math.min(Math.floor(frames), values.length - 1);
      const next = values[Math.min(nth + 1, values.length - 1)]!;
      return end(values[nth]! + (next - values[nth]!) * (frames - nth));
    },
    done,
  };
}

/**
 * Runs `work` in the next frame after the one being drawn, which the OS has taken by then, and returns what cancels it.
 * A move started there leaves the cost of drawing new contents to the frame before, so its first frame shows its start.
 */
export function afterFrame(work: () => void): () => void {
  let frame = requestAnimationFrame(() => (frame = requestAnimationFrame(() => work())));
  return () => cancelAnimationFrame(frame);
}

/**
 * Runs `work` in the component's scope once the OS has taken the first frame of the move that shows it, so its heavy
 * setup, the keyboard included, runs while the compositor carries the move.
 */
export function onPainted(work: () => void) {
  const owner = getOwner();
  let cancel = () => {};
  let frame = 0;
  let timer = 0;
  const later = () => (timer = window.setTimeout(() => runWithOwner(owner, work)));
  onMount(() => (cancel = afterFrame(() => (frame = requestAnimationFrame(later)))));
  onCleanup(() => {
    cancel();
    cancelAnimationFrame(frame);
    clearTimeout(timer);
  });
}

/** Hands the springs to the stylesheet's transitions as `--snappy` and `--snappy-ms`. */
export function followMotion() {
  const { easing, duration } = curve(SNAPPY);
  document.documentElement.style.setProperty("--snappy", easing);
  document.documentElement.style.setProperty("--snappy-ms", `${duration}ms`);
}
