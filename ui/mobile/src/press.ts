import type { Lift, Point } from "@octopus/shared/lift.ts";
import { feel } from "./feel";
import { COMMIT_PX, keepTouch } from "./touch";

export type { Point };

export const LONG_PRESS_MS = 450;

/** Every press under a finger, which a gesture that takes the finger over ends. */
const presses = new Set<() => void>();
export const cancelPresses = () => {
  for (const cancel of presses) cancel();
  presses.clear();
};

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
      let lifted: Lift | null = null;
      let dragging = false;
      const reset = () => {
        presses.delete(cancel);
        clearTimeout(timer);
        finger = null;
        lifted = null;
        dragging = false;
      };
      const cancel = () => {
        lifted?.cancel();
        reset();
      };
      const ours = (touches: TouchList) => [...touches].find((touch) => touch.identifier === finger?.id);
      element.addEventListener(
        "touchstart",
        (event) => {
          const touch = event.changedTouches[0];
          if (event.touches.length > 1 || touch === undefined) return;
          reset();
          const from = { x: touch.clientX, y: touch.clientY };
          finger = { id: touch.identifier, from };
          presses.add(cancel);
          timer = setTimeout(() => {
            keepTouch();
            feel("lift");
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
          const far = Math.hypot(to.x - finger.from.x, to.y - finger.from.y) >= COMMIT_PX;
          if (lifted === null) {
            if (far) reset();
            return;
          }
          event.preventDefault();
          dragging ||= far;
          lifted.move(to);
        },
        { passive: lift === undefined },
      );
      element.addEventListener("touchend", (event) => {
        if (ours(event.changedTouches) === undefined) return;
        if (lifted !== null && dragging) lifted.drop();
        else if (lifted !== null) {
          lifted.cancel();
          hold();
        }
        reset();
      });
      element.addEventListener("touchcancel", (event) => {
        if (ours(event.changedTouches) === undefined) return;
        cancel();
      });
      element.addEventListener("click", tap);
      element.addEventListener("contextmenu", (event) => event.preventDefault());
    },
  };
}
