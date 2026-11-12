import { getOwner, onCleanup, onMount, runWithOwner } from "solid-js";
import { curve, FADE_MS, play, reduced, SNAPPY, standing, type Spring, type Toward } from "@octopus/shared/motion.ts";

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

/** How far the page a move goes deeper from slides the other way, as the screen under a push does. */
const PARALLAX = "-30%";

/** The turn each page is in, which a turn of the same page stops where it stands. */
const turning = new WeakMap<HTMLElement, () => void>();

/**
 * Turns `page` to its new contents as a push and a pop turn a screen, `left` holding what it showed before: `left`
 * lies where it or `page` stood mid-turn, raised by the `scrolled` it was scrolled, and leaves once out of sight.
 * Going deeper the page slides in from the right over `left`, which shifts a third of the way left; going back `left`
 * slides off to the right, uncovering it; going across, or under reduced motion, `left` fades out over it. Both pages
 * are opaque, so each covers the other as screens do.
 */
export function turnPage(page: HTMLElement, left: HTMLElement, toward: Toward, scrolled: number) {
  const stood = getComputedStyle(turning.has(left) ? left : page).translate;
  turning.get(page)?.();
  turning.get(left)?.();
  const fade = toward === "across" || reduced();
  left.classList.add("gone");
  left.style.top = `${page.offsetTop - scrolled}px`;
  if (toward === "deeper" && !fade) page.before(left);
  else page.after(left);
  page.scrollTop = 0;
  const from = stood === "none" ? "0 0" : stood;
  const timing = fade ? { duration: FADE_MS, easing: "ease-out" } : curve(PUSH);
  const [enters, leaves] = toward === "deeper" ? ["100%", PARALLAX] : [PARALLAX, "100%"];
  const entering = fade ? null : page.animate([{ translate: `${enters} 0` }, { translate: "0 0" }], timing);
  const leaving = left.animate(
    fade
      ? [
          { translate: from, opacity: 1 },
          { translate: from, opacity: 0 },
        ]
      : [{ translate: from }, { translate: `${leaves} 0` }],
    { ...timing, fill: "forwards" },
  );
  const stop = () => {
    entering?.cancel();
    leaving.cancel();
    left.remove();
    if (turning.get(page) === stop) turning.delete(page);
  };
  leaving.finished.then(stop, () => {});
  turning.set(page, stop);
}

/** Opens `element` to its full height from nothing, so what is under it makes room on a spring rather than at once. */
export const unfold = (element: HTMLElement) =>
  queueMicrotask(() => {
    const height = `${element.offsetHeight}px`;
    play(
      element,
      [
        { height: "0px", minHeight: "0px", opacity: 0 },
        { height, minHeight: height, opacity: 1 },
      ],
      SNAPPY,
    );
  });

/** Hands the springs to the stylesheet's transitions as `--snappy` and `--snappy-ms`. */
export function followMotion() {
  const { easing, duration } = curve(SNAPPY);
  document.documentElement.style.setProperty("--snappy", easing);
  document.documentElement.style.setProperty("--snappy-ms", `${duration}ms`);
}
