import type { Bounds } from "@octopus/shared/generated.ts";
import type { Lift, LiftOnDrag, Point } from "@octopus/shared/lift.ts";
import { ui } from "@octopus/shared/state.ts";
import { layer } from "./layer";

/** Pointer travel that turns a press into a lift, so a press that stays put is a click. */
const LIFT_PX = 4;

/** A point where the float layer takes the pointer shows the float layer's UI, whatever the base layer has beneath it. */
const covered =
  layer === "base"
    ? ({ x, y }: Point) =>
        ui.float_input.some(
          ({ x: left, y: top, width, height }: Bounds) => x >= left && x < left + width && y >= top && y < top + height,
        )
    : () => false;

/**
 * Pressing an element and moving the pointer lifts it through `start` within its `within` ancestors, and letting go
 * drops it where the pointer is. Escape, or the pointer or the window lost mid-drag, flies it home. A press in a text
 * field stays the field's. The desktop zooms natively, so the layout's pixels are the screen's.
 */
export const liftOnDrag: LiftOnDrag = (within, start) => ({
  onPointerDown: (down) => {
    if (down.button !== 0 || (down.target instanceof Element && down.target.closest("input"))) return;
    const element = down.currentTarget;
    const stage = element.closest<HTMLElement>(within.stage);
    const list = element.closest<HTMLElement>(within.list);
    if (stage === null || list === null) return;
    const surface = { stage, list, free: within.free, throws: false, covered, zoom: 1, feel: () => {} };
    const from = { x: down.clientX, y: down.clientY };
    let lifted: Lift | null = null;
    let cancelled = false;
    const press = new AbortController();
    const listen = { signal: press.signal };
    window.addEventListener(
      "pointermove",
      (event) => {
        const to = { x: event.clientX, y: event.clientY };
        if (cancelled || (lifted === null && Math.hypot(to.x - from.x, to.y - from.y) < LIFT_PX)) return;
        lifted ??= start(element, from, surface);
        lifted.move(to);
      },
      listen,
    );
    window.addEventListener(
      "pointerup",
      () => {
        press.abort();
        if (lifted === null && !cancelled) return;
        const swallow = (click: MouseEvent) => click.stopPropagation();
        window.addEventListener("click", swallow, { capture: true, once: true });
        setTimeout(() => window.removeEventListener("click", swallow, { capture: true }));
        if (!cancelled) lifted?.drop();
      },
      listen,
    );
    const lose = () => {
      press.abort();
      if (!cancelled) lifted?.cancel();
    };
    window.addEventListener("pointercancel", lose, listen);
    window.addEventListener("blur", lose, listen);
    window.addEventListener(
      "keydown",
      (event) => {
        if (event.key !== "Escape" || cancelled) return;
        event.preventDefault();
        event.stopPropagation();
        cancelled = true;
        lifted?.cancel();
      },
      { capture: true, signal: press.signal },
    );
  },
});
