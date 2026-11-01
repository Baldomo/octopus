import { onCleanup } from "solid-js";
import { scale, ui } from "@octopus/shared/state.ts";
import { feel } from "./feel";
import { give, SNAPPY } from "@octopus/shared/motion.ts";
import { draw, glide, type Gliding, type Look } from "./motion";
import { owns } from "./touch";

/** Where letting go refreshes, and where the list waits while it does. */
const ARMS_PX = 64;
const HOLDS_PX = 48;
/** A refresh shows its spinner at least this long, so a fast one still reads as done. */
const SHOWN_MS = 500;

/**
 * A drag down from the list's top draws it down under a spinner that turns as it nears where letting go refreshes,
 * with a tick as it gets there; let go past it, the list waits under the spinning spinner until `refresh` settles.
 */
export function pullToRefresh(refresh: () => Promise<unknown>) {
  return (list: HTMLElement) => {
    const spinner = document.createElement("i");
    spinner.className = "pull";
    onCleanup(() => spinner.remove());
    let pulled = 0;
    let refreshing = false;
    /** Where the list and spinner stand; the finger alone turns the spinner, which spins on its own while refreshing. */
    const look = (y: number): Look => [
      [list, { translate: y > 0 ? `0 ${y}px` : "none" }],
      [spinner, { top: `${list.offsetTop + (y - 20) / 2}px`, opacity: String(Math.min(y / ARMS_PX, 1)) }],
    ];
    const pose = (y: number) => {
      pulled = y;
      draw(look(y));
      spinner.style.rotate = `${(y / ARMS_PX) * 270}deg`;
    };
    let running: Gliding | null = null;
    const settle = (to: number) => {
      if (running) pulled = running.stop();
      const run = glide(pulled, to, look, SNAPPY);
      [pulled, running] = [to, run];
      return run.done;
    };
    const release = async () => {
      if (pulled < ARMS_PX) return void settle(0);
      refreshing = true;
      spinner.classList.add("on");
      await Promise.allSettled([settle(HOLDS_PX), refresh(), new Promise((done) => setTimeout(done, SHOWN_MS))]);
      spinner.style.rotate = getComputedStyle(spinner).rotate;
      spinner.classList.remove("on");
      await settle(0);
      refreshing = false;
    };
    const atTop = () => list.classList.toggle("pull-top", list.scrollTop <= 0);
    atTop();
    list.addEventListener("scroll", atTop, { passive: true });
    owns(list, ({ axis, way, at }) => {
      if (axis !== "y" || way < 0 || list.scrollTop > 0 || refreshing) return null;
      if (!spinner.isConnected) list.before(spinner);
      if (running) [pulled, running] = [running.stop(), null];
      return {
        move: (to) => {
          const was = pulled;
          pose(give(Math.max((to.y - at.y) / scale(ui.prefs), 0)));
          if (was < ARMS_PX && pulled >= ARMS_PX) feel("threshold");
        },
        end: (velocity) => void (velocity ? release() : settle(0)),
      };
    });
  };
}
