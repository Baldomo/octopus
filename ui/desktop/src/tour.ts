/**
 * octopus's window's side of the first open's show: whether the show still covers the screens, and its tour of the
 * window, which each layer follows by working the targets it holds and telling the show where they sit as they move.
 */
import { createSignal } from "solid-js";
import { report, scale, showModal, showSidebar, showSurface, showView, ui } from "@octopus/shared/state.ts";
import { desktop, desktopEvents } from "./backend";
import { TOUR, type ShowDrive, type ShowTarget } from "./generated";

/** Whether the show covers the screens: so until read, then until it ends. */
const [showing, setShowing] = createSignal(true);
/** Whether the show is touring the window, during which the first open steps aside. */
const [touring, setTouring] = createSignal(false);
export { showing, touring };

const all = <T extends HTMLElement>(selector: string) => [...document.querySelectorAll<T>(selector)];
const saying = (selector: string, text: string) => all(selector).find((element) => element.textContent?.includes(text));

/** Each target the tour points at, as this layer finds it. */
const TARGETS: Record<ShowTarget, () => HTMLElement | null | undefined> = {
  edge: () => document.querySelector<HTMLElement>(".edge"),
  sidebar: () => document.querySelector<HTMLElement>(".sidebar"),
  add_workspace: () => document.querySelector<HTMLElement>(".sidebar .addws"),
  host: () => saying(".folder-pick .host", TOUR.server),
  folder: () => saying(".folder-pick .folders .mi", TOUR.project),
  choose: () => document.querySelector<HTMLElement>(".folder-pick .pick-foot .btn"),
  new_task: () => saying(".sidebar .wsh", TOUR.project)?.querySelector<HTMLElement>(".acts .mini"),
  form: () => document.querySelector<HTMLElement>(".new-task"),
  title: () => document.querySelector<HTMLElement>(".new-task input"),
  create: () => document.querySelector<HTMLElement>(".new-task .foot button[type=submit]"),
  terminal: () => document.querySelector<HTMLElement>(".task-split .splitter + *"),
};

/** What the window showed before the tour, which the tour's end puts back. */
const shown = () => ({ view: ui.prefs.view, surface: ui.prefs.surface, sidebar: ui.sidebar });
let before: ReturnType<typeof shown>;

/** Tells the show where each target this layer holds sits, every frame one has moved, until the tour ends. */
function spot() {
  const told = new Map<ShowTarget, string>();
  const frame = () => {
    if (!touring()) return;
    const k = scale(ui.prefs);
    for (const [target, find] of Object.entries(TARGETS) as [ShowTarget, (typeof TARGETS)[ShowTarget]][]) {
      const box = find()?.getBoundingClientRect();
      if (!box?.width) continue;
      const rect = { x: box.x * k, y: box.y * k, width: box.width * k, height: box.height * k };
      const key = JSON.stringify(rect);
      if (told.get(target) === key) continue;
      told.set(target, key);
      desktop.showSpot({ target, rect }).catch(report);
    }
    requestAnimationFrame(frame);
  };
  frame();
}

/** Types `text` into a field as typing it does, so the field's own handlers take it. */
function type(target: ShowTarget, text: string) {
  const field = TARGETS[target]();
  if (!(field instanceof HTMLInputElement)) return;
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(field, text);
  field.dispatchEvent(new Event("input", { bubbles: true }));
}

/** Puts the window back as the tour found it, if the tour began; the base layer, which holds the state's side, does it. */
function end(base: boolean) {
  if (!touring()) return;
  setTouring(false);
  if (!base) return;
  showModal(null);
  showSurface(before.surface);
  showView(before.view);
  showSidebar(before.sidebar);
}

/** Follows the show and its tour, in both layers; `base` is whether this layer holds the state's side. */
export async function followShow(base: boolean) {
  await desktopEvents.showEnd(() => setShowing(false));
  await desktopEvents.showDrive((step: ShowDrive) => {
    if (step.do === "start") {
      before = shown();
      setTouring(true);
      spot();
    } else if (step.do === "press") TARGETS[step.target]()?.click();
    else if (step.do === "type") type(step.target, step.text);
    else if (step.do === "sidebar") {
      if (base) showSidebar(step.shown);
    } else end(base);
  });
  if (!(await desktop.showPlaying())) setShowing(false);
}
