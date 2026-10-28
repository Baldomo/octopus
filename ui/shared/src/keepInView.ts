import { createEffect } from "solid-js";

export const keepInView = (inView: () => boolean) => (element: HTMLElement) =>
  createEffect(() => inView() && element.scrollIntoView({ block: "nearest", inline: "nearest" }));
