import { createEffect, createRoot } from "solid-js";
import { chase, endLift, holdLift, LANDS_MS, raise, release, rest, type Start } from "./lift";
import { reduced } from "./motion";
import { moveTo } from "./sectionOrder";
import { slotAmong } from "./slot";
import { report } from "./state";

const sameOrder = (a: readonly string[], b: readonly string[]) =>
  a.length === b.length && a.every((key, nth) => key === b[nth]);

export const liftTab =
  (
    key: string,
    keys: () => string[],
    arrange: (order: string[] | null) => void,
    move: (to: number) => Promise<unknown>,
  ): Start =>
  (element, from, surface) => {
    const { list, zoom } = surface;
    endLift();
    const own = keys();
    let order = own;
    let at = own.indexOf(key);
    const scrolled = list.scrollLeft;
    const along = (tab: HTMLElement) => ({
      left: rest(tab, zoom).left / zoom + scrolled,
      width: tab.getBoundingClientRect().width / zoom,
    });
    const tabs = new Map(
      [...list.querySelectorAll<HTMLElement>("[data-key]:not(.flip-ghost)")].map((tab) => [tab.dataset.key!, tab]),
    );
    const held = along(element);
    const middles = own
      .filter((each) => each !== key)
      .map((each) => {
        const { left, width } = along(tabs.get(each)!);
        return left + width / 2 - (left > held.left ? held.width : 0);
      });
    const { ghost, origin, fly, follow } = raise(element, surface);
    element.classList.add("lifting");
    const finish = () => {
      ghost.remove();
      end();
    };
    const end = () => {
      if (!release(finish)) return;
      element.classList.remove("lifting");
      arrange(null);
    };
    holdLift(finish);

    const pointer = chase((to) => {
      const scroll = list.scrollLeft;
      const { x } = follow(to, from, list.getBoundingClientRect(), "row");
      const next = slotAmong(middles, origin.left / zoom + x + scroll, at);
      if (next !== at) arrange((order = moveTo(own, key, (at = next))));
    });

    const settle = (landed: Promise<unknown>) => {
      pointer.stop();
      ghost.classList.remove("up");
      if (reduced()) return void landed.then(finish);
      void Promise.all([fly(rest(element, zoom)), landed]).then(([arrived]) => arrived && finish());
    };
    const home = () => {
      order = own;
      arrange(null);
      settle(Promise.resolve());
    };
    const landed = (to: number) =>
      new Promise<void>((resolve) =>
        createRoot((dispose) => {
          const land = () => {
            dispose();
            resolve();
          };
          setTimeout(land, LANDS_MS);
          createEffect(() => sameOrder(keys(), order) && land());
          move(to).catch((error) => {
            land();
            report(error);
          });
        }),
      );

    return {
      move: (to) => pointer.move(to),
      drop: () => {
        const to = order.indexOf(key);
        if (to === own.indexOf(key)) return home();
        surface.feel("drop");
        settle(landed(to));
      },
      cancel: home,
    };
  };
