import { createEffect, untrack } from "solid-js";
import { createStore } from "solid-js/store";
import { backend } from "./backend.ts";
import {
  BACKGROUND_BROWSER_PANEL,
  NEW_BROWSER_PANEL,
  type BrowserPanel,
  type BrowserTarget,
  type TaskView,
} from "./generated.ts";
import { report } from "./state.ts";

/** A browser panel this client selects, or `new`, which makes the next page open a panel of its own. */
export type Target = Exclude<BrowserTarget, typeof BACKGROUND_BROWSER_PANEL>;

/** The panel each task's tabs select on this client. */
const [selected, setSelected] = createStore<Record<string, Target>>({});
/** Per task key, its panels as this client last saw them; a panel whose `load` moved since selects itself. */
const seen = new Map<string, BrowserPanel[]>();

/** The task's chosen panel while it exists, else its last one. */
function targetOf(task: TaskView): Target {
  const chosen = selected[task.key];
  const panels = task.task.browsers;
  if (chosen === NEW_BROWSER_PANEL || panels.some((panel) => panel.panel === chosen))
    return chosen ?? NEW_BROWSER_PANEL;
  return panels.at(-1)?.panel ?? NEW_BROWSER_PANEL;
}

/** The tab that takes the selection once the selected `closed` leaves `before` for `after`: the next, else the one before. */
export function heirOf(before: readonly string[], after: readonly string[], closed: string) {
  const at = before.indexOf(closed);
  const stays = (tab: string) => after.includes(tab);
  return before.slice(at + 1).find(stays) ?? before.slice(0, Math.max(at, 0)).findLast(stays);
}

/**
 * Follows the task's panels as they change: a panel whose `load` moved since this client last saw the task selects
 * itself, a new panel counting from 0; otherwise the selection holds on to its panel, so panels moving around it leave
 * it where it is, and passes to its heir when it closes.
 */
function followPanels(task: TaskView) {
  const panels = task.task.browsers;
  const before = seen.get(task.key);
  seen.set(
    task.key,
    panels.map((panel) => ({ ...panel })),
  );
  const loadBefore = (panel: BrowserPanel) => before?.find((known) => known.panel === panel.panel)?.load ?? 0;
  const moved = before === undefined ? undefined : panels.findLast((panel) => loadBefore(panel) !== panel.load);
  const order = panels.map((panel) => panel.panel);
  const chosen = untrack(() => selected[task.key]);
  const gone = chosen !== undefined && chosen !== NEW_BROWSER_PANEL && !order.includes(chosen);
  const heir = gone
    ? (heirOf(
        (before ?? panels).map((panel) => panel.panel),
        order,
        chosen,
      ) ?? NEW_BROWSER_PANEL)
    : undefined;
  setSelected(task.key, moved?.panel ?? heir ?? untrack(() => targetOf(task)));
}

/**
 * The task's browser panel selection on this client, following the task's panels for as long as the calling component
 * lives: `target` is the selected panel or `new`, `panel` the selected panel, and `select` takes the selection.
 */
export function createPanelSelection(view: () => TaskView | undefined) {
  createEffect(() => {
    const task = view();
    if (task !== undefined) followPanels(task);
  });
  const target = (): Target => {
    const task = view();
    return task === undefined ? NEW_BROWSER_PANEL : targetOf(task);
  };
  const panel = () => view()?.task.browsers.find((each) => each.panel === target());
  const select = (next: Target) => {
    const task = view();
    if (task !== undefined) setSelected(task.key, next);
  };
  return { target, panel, select };
}

/** Shows `url` in the task's target panel, which stays the target. */
export const openPage = (task: TaskView, url: string) =>
  backend.browserOpen(task.at, targetOf(task), url).then((panel) => setSelected(task.key, panel), report);

function isLocal(address: string) {
  if (!URL.canParse(`http://${address}`)) return false;
  const host = new URL(`http://${address}`).hostname;
  return (
    !host.includes(".") ||
    host.startsWith("[") ||
    /^[\d.]+$/.test(host) ||
    /\.(localhost|local|test|internal|lan)$/.test(host)
  );
}

/** What the user typed as a page: a URL with its scheme, a file's absolute path, or a web address. */
export const typedPage = (typed: string) => {
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(typed)) return typed;
  if (typed.startsWith("/")) return new URL(`file://${typed}`).href;
  return `${isLocal(typed) ? "http" : "https"}://${typed}`;
};

/** Opens what the user typed as a page in the task's target panel, unless it is blank. */
export function openTyped(task: TaskView, typed: string) {
  const page = typed.trim();
  if (page !== "") openPage(task, typedPage(page));
}

export const panelName = (panel: BrowserPanel) => panel.title || pageName(panel.url);

/** What a page is called before its title arrives: a file's name, else its host. */
export const pageName = (url: string) => {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "file:" ? decodeURIComponent(parsed.pathname.split("/").at(-1) ?? "") : parsed.host;
  } catch {
    return url;
  }
};
