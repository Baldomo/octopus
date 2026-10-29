import { createSignal, getOwner, onCleanup, onMount, runWithOwner } from "solid-js";

/**
 * A spring by its period and damping ratio, as UIKit and SwiftUI name them, its response the time the move takes.
 * Every move the app makes on its own settles on one; a move the finger lets go of leaves at the finger's velocity.
 */
export type Spring = { response: number; damping: number };

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
/** Rows, bars, toasts and controls. */
export const SNAPPY: Spring = { response: 0.2, damping: 1 };
/** What reduced motion keeps: one short fade in place of every move across the screen. */
export const FADE_MS = 180;
export const FADE: Spring = { response: FADE_MS / 1000, damping: 1 };
/** A released throw lands where its speed would carry it this many seconds ahead. */
export const PROJECT_S = 0.2;

const query = matchMedia("(prefers-reduced-motion: reduce)");
const [reduced, setReduced] = createSignal(query.matches);
query.addEventListener("change", () => setReduced(query.matches));
export { reduced };

/** The displacement left after `t` seconds of a spring released `from` its rest at 0 with velocity `v`. */
function position({ response, damping }: Spring, from: number, v: number, t: number) {
  const omega = (2 * Math.PI) / response;
  if (damping >= 1) return Math.exp(-omega * t) * (from + (v + omega * from) * t);
  const decay = damping * omega;
  const wave = omega * Math.sqrt(1 - damping * damping);
  return Math.exp(-decay * t) * (from * Math.cos(wave * t) + ((v + decay * from) / wave) * Math.sin(wave * t));
}

/** Where a move from `from` to `to` stands `t` seconds in, or null once it has settled there. */
function standing(spring: Spring, from: number, to: number, velocity: number, t: number) {
  const distance = Math.abs(to - from) || 1;
  const left = position(spring, from - to, velocity, t);
  const speed = (position(spring, from - to, velocity, t + 0.001) - left) / 0.001;
  return Math.abs(left) < distance * 0.001 && Math.abs(speed) < distance * 0.01 ? null : to + left;
}

export type Running = { stop: () => void; done: Promise<void> };

/**
 * Moves a value from `from` to `to` on `spring`, starting at `velocity` units a second, calling `apply` each frame and
 * last with `to`, on the page's thread. Stopping leaves it where it was.
 */
export function animate(
  from: number,
  to: number,
  apply: (value: number) => void,
  spring: Spring,
  velocity = 0,
): Running {
  let frame = 0;
  let finish!: () => void;
  const done = new Promise<void>((resolve) => (finish = resolve));
  const start = performance.now();
  const tick = (now: number) => {
    const at = standing(spring, from, to, velocity, (now - start) / 1000);
    apply(at ?? to);
    if (at === null) return finish();
    frame = requestAnimationFrame(tick);
  };
  if (from === to && velocity === 0) {
    apply(to);
    finish();
  } else frame = requestAnimationFrame(tick);
  return {
    stop: () => {
      cancelAnimationFrame(frame);
      finish();
    },
    done,
  };
}

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

/** A spring from rest to rest as a CSS `linear()` easing and the time it takes to settle, for CSS and WAAPI. */
const curves = new Map<Spring, { easing: string; duration: number }>();
export function curve(spring: Spring): { easing: string; duration: number } {
  const known = curves.get(spring);
  if (known) return known;
  let settle = 0.05;
  while (settle < 3 && Math.abs(position(spring, 1, 0, settle)) > 0.001) settle += 0.01;
  const steps = 40;
  const stops = Array.from({ length: steps + 1 }, (_, n) =>
    (1 - position(spring, 1, 0, (settle * n) / steps)).toFixed(4),
  );
  const shaped = { easing: `linear(${stops.join(", ")})`, duration: Math.round(settle * 1000) };
  curves.set(spring, shaped);
  return shaped;
}

/** How long `spring` takes from rest to cover `share` of its way, in milliseconds. */
export function reach(spring: Spring, share: number) {
  let t = 0;
  while (t < 3 && 1 - position(spring, 1, 0, t) < share) t += 0.005;
  return Math.round(t * 1000);
}

/**
 * Animates an element along a spring `delay` ms from now, holding its first keyframe until then, or under reduced
 * motion fades it over the keyframes' opacity alone.
 */
export function play(element: Element, keyframes: Keyframe[], spring: Spring, delay = 0): Animation {
  if (reduced()) {
    const faded = keyframes.map((frame) => ({ opacity: frame.opacity ?? 1 }));
    return element.animate(faded, { duration: FADE_MS, easing: "ease-out" });
  }
  return element.animate(keyframes, { ...curve(spring), delay, fill: "backwards" });
}

/** The finger's velocity, in pixels a second, over its last 100 ms, so a finger held still lets go at rest. */
export function tracker() {
  let samples: { at: number; x: number; y: number }[] = [];
  const recent = () => samples.filter((sample) => performance.now() - sample.at < 100);
  return {
    add: (x: number, y: number) => {
      samples = [...recent(), { at: performance.now(), x, y }];
    },
    velocity: () => {
      const kept = recent();
      const [first, last] = [kept[0], kept.at(-1)];
      if (first === undefined || last === undefined || last.at === first.at) return { x: 0, y: 0 };
      const seconds = (last.at - first.at) / 1000;
      return { x: (last.x - first.x) / seconds, y: (last.y - first.y) / seconds };
    },
  };
}

/** Hands the springs to the stylesheet's transitions as `--snappy` and `--snappy-ms`. */
export function followMotion() {
  const { easing, duration } = curve(SNAPPY);
  document.documentElement.style.setProperty("--snappy", easing);
  document.documentElement.style.setProperty("--snappy-ms", `${duration}ms`);
}
