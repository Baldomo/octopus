import { createEffect, createRoot, createSelector, createSignal } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import type { Boot, Model, ServerView, TaskRef, TaskState, TaskView } from "./generated";

const VIEWS = ["task", "tasks"] as const;
const SURFACES = ["files", "browser"] as const;
export type View = (typeof VIEWS)[number];
export type Surface = (typeof SURFACES)[number];
export type Overlay = "palette" | "new-task" | "pair-server";
export type Dragged = { key: string; at: TaskRef; section: string; state: TaskState };

/** How many opened tasks stay mounted, so switching among them skips the mount. */
const RECENT = 8;
const PROBLEM_MS = 8000;
const PREFS = "octopus.prefs";

export const [model, setModel] = createStore<Model>({
  version: -1,
  lists: { sections: [], workspaces: [] },
  tasks: {},
});
export const [servers, setServers] = createSignal<ServerView[]>([]);
export const [me, setMe] = createSignal("");

type Prefs = {
  pinned: boolean;
  collapsed: string[];
  surface: Surface;
  view: View;
  open: string | null;
  board: string | null;
};

const oneOf = <T extends string>(options: readonly T[], value: unknown, fallback: T): T =>
  options.find((option) => option === value) ?? fallback;
const text = (value: unknown) => (typeof value === "string" ? value : null);

/** Prefs come from an earlier build or a hand edit, so each field is checked and falls back on its own. */
function loadPrefs(): Prefs {
  let saved: Record<string, unknown> = {};
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(PREFS) ?? "{}");
    if (typeof parsed === "object" && parsed !== null) saved = parsed as Record<string, unknown>;
  } catch {
    localStorage.removeItem(PREFS);
  }
  return {
    pinned: saved.pinned === true,
    collapsed: Array.isArray(saved.collapsed) ? saved.collapsed.filter((name) => typeof name === "string") : [],
    surface: oneOf(SURFACES, saved.surface, "files"),
    view: oneOf(VIEWS, saved.view, "task"),
    open: text(saved.open),
    board: text(saved.board),
  };
}

const prefs = loadPrefs();
export const [view, setView] = createSignal<View>(prefs.view);
export const [open, setOpen] = createSignal<string | null>(prefs.open);
export const [recent, setRecent] = createSignal<string[]>(prefs.open === null ? [] : [prefs.open]);
/** Lets each of a thousand rows track whether it is the open one without each re-running on a switch. */
export const isOpen = createRoot(() => createSelector(open));
export const [surface, setSurface] = createSignal<Surface>(prefs.surface);
export const [board, setBoard] = createSignal<string | null>(prefs.board);
export const [pinned, setPinned] = createSignal(prefs.pinned);
export const [collapsed, setCollapsed] = createSignal<string[]>(prefs.collapsed);
export const [sidebar, setSidebar] = createSignal(prefs.pinned);
export const [overlay, setOverlay] = createSignal<Overlay | null>(null);
export const [dragged, setDragged] = createSignal<Dragged | null>(null);
export const [problem, setProblem] = createSignal<string | null>(null);

export function persistPrefs() {
  createEffect(() => {
    const saved: Prefs = {
      pinned: pinned(),
      collapsed: collapsed(),
      surface: surface(),
      view: view(),
      open: open(),
      board: board(),
    };
    localStorage.setItem(PREFS, JSON.stringify(saved));
  });
}

let problemTimer: ReturnType<typeof setTimeout> | undefined;
export function report(error: unknown) {
  setProblem(String(error));
  clearTimeout(problemTimer);
  problemTimer = setTimeout(() => setProblem(null), PROBLEM_MS);
}

export function act(promise: Promise<unknown>) {
  promise.catch(report);
}

/** The boot reply and model events travel apart, so the newest version wins. */
export function applyModel(next: Model) {
  if (next.version > model.version) setModel(reconcile(next, { key: "key" }));
}

export function applyBoot(boot: Boot) {
  setMe(boot.me);
  setServers(boot.servers);
  applyModel(boot.model);
}

export const openView = (): TaskView | undefined => {
  const key = open();
  return key === null ? undefined : model.tasks[key];
};

export const serverView = (name: string) => servers().find((server) => server.name === name);

export function openTask(key: string) {
  setOpen(key);
  setRecent((keys) => [key, ...keys.filter((candidate) => candidate !== key)].slice(0, RECENT));
  setView("task");
}

export function showBoard(section: string) {
  setBoard(section);
  setView("tasks");
}

export function showTasks() {
  const section = openView()?.section ?? board();
  if (section !== null) setBoard(section);
  setView("tasks");
}

export function showSurface(next: Surface) {
  setSurface(next);
  setView("task");
}

export function toggleCollapsed(section: string) {
  setCollapsed((names) => (names.includes(section) ? names.filter((name) => name !== section) : [...names, section]));
}
