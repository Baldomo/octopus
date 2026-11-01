import { createEffect, createMemo } from "solid-js";

/** Scrolls the element into view each time `inView` turns true, so a scroll the user made since stays. */
export const keepInView = (inView: () => boolean) => (element: HTMLElement) => {
  const wanted = createMemo(inView);
  createEffect(() => wanted() && element.scrollIntoView({ block: "nearest", inline: "nearest" }));
};
