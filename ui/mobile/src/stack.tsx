import { createEffect, createSignal, For, on, onCleanup, type JSX } from "solid-js";
import { back, screens, sheet, type Screen } from "./nav";
import { afterFrame, draw, FADE, glide, POP, PUSH, reduced, type Look } from "./motion";
import { ownsScreenEdge } from "./touch";
import { releaseFocus } from "./viewport";

/** How far in from the left edge an iOS swipe starts, and how far the screen under the top one sits while covered. */
const EDGE_PX = 24;
const PARALLAX = 0.3;
const COVER = 0.06;
const shade = (screen: HTMLElement) => screen.lastElementChild as HTMLElement;
/** A released swipe pops past half the width, or when thrown at more than this many widths a second. */
const THROW = 0.5;
/** How far across the screen a predictive back gesture at full progress carries it. */
const PREDICTIVE_REACH = 0.4;

/** A gesture that can carry the top sheet back, which the sheet up registers while it shows. */
export type BackDrive = { start: () => void; move: (progress: number) => void; cancel: () => void };
let sheetDrive: BackDrive | null = null;
export const driveSheetBack = (drive: BackDrive) => (sheetDrive = drive);
export const releaseSheetBack = (drive: BackDrive) => sheetDrive === drive && (sheetDrive = null);

/**
 * Every pushed screen, root first, each a layer the one above slides over: a push slides the new one in from the
 * right edge over the one it covers, which shifts left under a shade, and a pop reverses it from wherever the screen
 * stands. A screen shows only while it is the top one or moving.
 */
export function Stack(props: { children: (screen: Screen) => JSX.Element }) {
  const [layers, setLayers] = createSignal<Screen[]>(screens());
  const [moving, setMoving] = createSignal<Screen[]>([]);
  const elements = new Map<Screen, HTMLDivElement>();
  /** How far each moving screen stands off the right edge, 0 covering the screen and 1 gone. */
  const off = new Map<Screen, number>();
  /** Stops the move under way or about to start, keeping where its screen stands. */
  let running: (() => void) | null = null;

  const look = (top: Screen, under: Screen | undefined, at: number): Look => {
    const above = elements.get(top);
    const below = under && elements.get(under);
    if (above === undefined) return [];
    if (reduced()) return [[above, { opacity: String(1 - at) }]];
    const covered: Look = below
      ? [
          [below, { translate: `${-(1 - at) * PARALLAX * 100}% 0` }],
          [shade(below), { opacity: String((1 - at) * COVER) }],
        ]
      : [];
    return [[above, { translate: `${at * 100}% 0` }], ...covered];
  };
  const pose = (top: Screen, under: Screen | undefined, at: number) => {
    off.set(top, at);
    draw(look(top, under, at));
  };
  const rest = (screen: Screen | undefined) => {
    const element = screen && elements.get(screen);
    if (element === undefined) return;
    element.style.translate = element.style.opacity = shade(element).style.opacity = "";
  };

  /**
   * Moves `top` over `under`, resolving to whether it got there before another move took over; a screen sent off the
   * edge stays there until the stack lets it go.
   */
  const slide = (top: Screen, under: Screen | undefined, to: number, velocity = 0) => {
    running?.();
    setMoving(under ? [under, top] : [top]);
    const spring = reduced() ? FADE : to === 0 ? PUSH : POP;
    const run = glide(off.get(top) ?? 0, to, (at) => look(top, under, at), spring, velocity);
    const stop = () => void off.set(top, run.stop());
    running = stop;
    return run.done.then(() => {
      if (running !== stop) return false;
      running = null;
      off.set(top, to);
      rest(under);
      if (to === 0) {
        rest(top);
        setMoving([]);
      }
      return true;
    });
  };

  let before = screens();
  createEffect(
    on(
      screens,
      (now) => {
        const [top, was] = [now.at(-1)!, before.at(-1)!];
        before = now;
        if (top === was) return setLayers(now);
        releaseFocus(elements.get(was));
        if (now.includes(was)) {
          setLayers(now);
          off.set(top, 1);
          queueMicrotask(() => {
            running?.();
            setMoving([was, top]);
            pose(top, was, 1);
            running = afterFrame(() => void slide(top, was, 0));
          });
          return;
        }
        setLayers([...now, was]);
        queueMicrotask(() =>
          slide(was, top, 1).then((arrived) => {
            off.delete(was);
            setLayers((shown) => shown.filter((screen) => screen !== was || screens().includes(screen)));
            setMoving((shown) => shown.filter((screen) => screen !== was && (!arrived || screen !== top)));
          }),
        );
      },
      { defer: true },
    ),
  );

  /** The finger, or Android's predictive back, carrying the top screen off to reveal the one under it. */
  const grab = () => {
    const stack = screens();
    if (sheet() !== null || stack.length < 2) return null;
    running?.();
    running = null;
    const [top, under] = [stack.at(-1)!, stack.at(-2)!];
    setMoving([under, top]);
    return {
      move: (at: number) => pose(top, under, Math.min(Math.max(at, 0), 1)),
      release: (pops: boolean, velocity = 0) => {
        if (!pops) return void slide(top, under, 0, velocity);
        void slide(top, under, 1, velocity).then((arrived) => arrived && back());
      },
    };
  };

  const shown = (screen: Screen) => screen === layers().at(-1) || moving().includes(screen);

  followEdgeSwipe(grab);
  followPredictiveBack(grab);

  return (
    <For each={layers()}>
      {(screen) => (
        <div
          ref={(element) => {
            elements.set(screen, element);
            onCleanup(() => elements.delete(screen));
          }}
          class="app"
          classList={{ cam: screen.at === "scan", shown: shown(screen), over: !reduced() && moving()[1] === screen }}
        >
          <div class="content">{props.children(screen)}</div>
          <div class="shade" />
        </div>
      )}
    </For>
  );
}

type Grab = () => { move: (at: number) => void; release: (pops: boolean, velocity?: number) => void } | null;

/** iOS's edge swipe: a finger from the left edge moving right drags the top screen, and letting go pops or settles. */
function followEdgeSwipe(grab: Grab) {
  if (/Android/.test(navigator.userAgent)) return;
  ownsScreenEdge(({ from, axis, way }) => {
    if (axis !== "x" || way < 0 || from.x > EDGE_PX) return null;
    const held = grab();
    if (held === null) return null;
    let at = 0;
    return {
      move: (to) => {
        at = (to.x - from.x) / innerWidth;
        held.move(at);
      },
      end: (thrown) => {
        const velocity = (thrown?.x ?? 0) / innerWidth;
        held.release(thrown !== null && velocity > -THROW && (at > 0.5 || velocity > THROW), velocity);
      },
    };
  });
}

/**
 * Android hands the page the system back gesture's progress while the page has something to pop: it carries the top
 * sheet or screen, letting go settles it back, and committing pops it from where it stands.
 */
function followPredictiveBack(grab: Grab) {
  let held: ReturnType<Grab> = null;
  let drive: BackDrive | null = null;
  addEventListener("octopus-back", (event) => {
    const { phase, progress } = (event as CustomEvent<{ phase: string; progress: number }>).detail;
    if (phase === "start") {
      drive = sheetDrive;
      if (drive) drive.start();
      else held = grab();
    } else if (phase === "move") {
      if (drive) drive.move(progress);
      else held?.move(progress * PREDICTIVE_REACH);
    } else if (phase === "cancel") {
      drive?.cancel();
      held?.release(false);
      [drive, held] = [null, null];
    } else {
      if (held) held.release(true);
      else back();
      [drive, held] = [null, null];
    }
  });
}
