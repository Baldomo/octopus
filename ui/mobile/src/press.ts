import { impactFeedback } from "@tauri-apps/plugin-haptics";

const LONG_PRESS_MS = 450;
/** A finger that moves further than this is scrolling, not pressing; after a lift, it is dragging. */
const PRESS_SLOP_PX = 10;

export type Point = { x: number; y: number };

/** An element lifted under a held finger: moving the finger drags it, and lifting the finger drops it. */
export type Lift = { move: (to: Point) => void; drop: () => void; cancel: () => void };

/**
 * A tap runs `tap`, and holding a finger still ticks and runs `hold`. Where `lift` lifts the held element, the hold
 * runs only if the finger leaves it in place; moving the finger drags it instead.
 *
 * The listeners are the element's own, so a drag survives the lists re-rendering around it, and its touchmove is
 * active only where it can lift, so it can keep a drag from scrolling.
 */
export function pressed(tap: () => void, hold: () => void, lift?: (element: HTMLElement, from: Point) => Lift | null) {
  return {
    ref: (element: HTMLElement) => {
      let timer: ReturnType<typeof setTimeout> | undefined;
      /** The finger pressing and where it touched down; any other finger leaves the press to it. */
      let finger: { id: number; from: Point } | null = null;
      let held = false;
      let lifted: Lift | null = null;
      let dragging = false;
      const reset = () => {
        clearTimeout(timer);
        finger = null;
        lifted = null;
        dragging = false;
      };
      const ours = (touches: TouchList) => [...touches].find((touch) => touch.identifier === finger?.id);
      element.addEventListener(
        "touchstart",
        (event) => {
          const touch = event.changedTouches[0];
          if (event.touches.length > 1 || touch === undefined) return;
          reset();
          held = false;
          const from = { x: touch.clientX, y: touch.clientY };
          finger = { id: touch.identifier, from };
          timer = setTimeout(() => {
            held = true;
            void impactFeedback("medium");
            lifted = lift?.(element, from) ?? null;
            if (lifted === null) hold();
          }, LONG_PRESS_MS);
        },
        { passive: true },
      );
      element.addEventListener(
        "touchmove",
        (event) => {
          const touch = ours(event.touches);
          if (finger === null || touch === undefined) return;
          const to = { x: touch.clientX, y: touch.clientY };
          const far = Math.hypot(to.x - finger.from.x, to.y - finger.from.y) > PRESS_SLOP_PX;
          if (lifted === null) {
            if (far) reset();
            return;
          }
          event.preventDefault();
          dragging ||= far;
          if (dragging) lifted.move(to);
        },
        { passive: lift === undefined },
      );
      element.addEventListener("touchend", (event) => {
        if (ours(event.changedTouches) === undefined) return;
        // The click a hold's lift synthesizes would land on whatever the hold opened under the finger.
        if (held) event.preventDefault();
        if (lifted !== null && dragging) lifted.drop();
        else if (lifted !== null) {
          lifted.cancel();
          hold();
        }
        reset();
      });
      element.addEventListener("touchcancel", (event) => {
        if (ours(event.changedTouches) === undefined) return;
        lifted?.cancel();
        reset();
      });
      element.addEventListener("click", () => held || tap());
      element.addEventListener("contextmenu", (event) => event.preventDefault());
    },
  };
}
