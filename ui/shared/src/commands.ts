import { backend } from "./backend";
import {
  act,
  openView,
  showModal,
  showSurface,
  showTasks,
  showView,
  toggleClientMenu,
  toggleSidebar,
  zoom,
} from "./state";
import { filesOf, renderable } from "./taskFiles";

export const mac = navigator.userAgent.includes("Mac");

/**
 * `keys` are written as macOS shows them, such as ⌘⇧V or ⌃⇥, the first being the one a hint names. While `applies` is
 * false the key passes through to the focused element; `inPage` tells whether a browser page holds the keyboard.
 */
export type Command = { label: string; keys: string[]; applies?: (inPage: boolean) => boolean; run: () => void };

const openFiles = () => {
  const open = openView();
  return open === undefined ? undefined : filesOf(open);
};
const selectedRenders = () => {
  const active = openFiles()?.state.active;
  return active !== null && active !== undefined && renderable(active);
};

/** What the Browser surface does on its keys, on a client that has one. */
type BrowserKeys = {
  /** Whether Browser shows a page. */
  shows: () => boolean;
  /** Whether Browser shows its tabs. */
  showsTabs: () => boolean;
  /** Whether this client closed a tab of the open task that it can reopen. */
  reopens: () => boolean;
  locate: () => void;
  find: () => void;
  go: (step: "back" | "forward" | "reload" | "reload_from_origin") => void;
  close: () => void;
  add: () => void;
  reopen: () => void;
  cycle: (step: 1 | -1) => void;
};
let browser: BrowserKeys | null = null;
export const provideBrowser = (keys: BrowserKeys) => (browser = keys);
const showsPage = () => browser?.shows() === true;
const showsTabs = () => browser?.showsTabs() === true;
const inBrowser = (inPage: boolean) =>
  browser !== null && (inPage || Boolean(document.activeElement?.closest(".browser")));
const inBrowserSurface = (run: () => void) => () => {
  showSurface("browser");
  run();
};

/** Every action a key or the palette runs. Where two commands share a key, the first that applies takes it. */
export const commands = {
  taskView: { label: "Task view", keys: ["⌘E"], run: () => showView("task") },
  newTab: { label: "New browser tab", keys: ["⌘T"], applies: inBrowser, run: inBrowserSurface(() => browser?.add()) },
  tasksView: { label: "Tasks view", keys: ["⌘T"], run: showTasks },
  files: { label: "Files", keys: ["⌘1"], run: () => showSurface("files") },
  browser: { label: "Browser", keys: ["⌘2"], run: () => showSurface("browser") },
  location: {
    label: "Open location",
    keys: ["⌘L"],
    applies: () => browser !== null,
    run: inBrowserSurface(() => browser?.locate()),
  },
  back: { label: "Back", keys: ["⌘["], applies: showsPage, run: () => browser?.go("back") },
  forward: { label: "Forward", keys: ["⌘]"], applies: showsPage, run: () => browser?.go("forward") },
  reload: { label: "Reload page", keys: ["⌘R"], applies: showsPage, run: () => browser?.go("reload") },
  reloadFromOrigin: {
    label: "Reload page ignoring its cache",
    keys: ["⌘⇧R"],
    applies: showsPage,
    run: () => browser?.go("reload_from_origin"),
  },
  nextTab: { label: "Next browser tab", keys: ["⌃⇥", "⌘⇧]"], applies: showsTabs, run: () => browser?.cycle(1) },
  previousTab: {
    label: "Previous browser tab",
    keys: ["⌃⇧⇥", "⌘⇧["],
    applies: showsTabs,
    run: () => browser?.cycle(-1),
  },
  find: { label: "Find in page", keys: ["⌘F"], applies: showsPage, run: () => browser?.find() },
  closePage: { label: "Close browser tab", keys: ["⌘W"], applies: showsPage, run: () => browser?.close() },
  reopenTab: {
    label: "Reopen closed browser tab",
    keys: ["⌘⇧T"],
    applies: () => browser?.reopens() === true,
    run: inBrowserSurface(() => browser?.reopen()),
  },
  rendered: {
    label: "Code or rendered view",
    keys: ["⌘⇧V"],
    applies: () => Boolean(document.activeElement?.closest(".files")) && selectedRenders(),
    run: () => openFiles()?.toggleRendered(),
  },
  newTask: { label: "New task", keys: ["⌘N"], run: () => showModal("new_task") },
  search: { label: "Search tasks, files, commands", keys: ["⌘K"], run: () => showModal("palette") },
  sidebar: { label: "Toggle sidebar", keys: ["⌘\\"], run: toggleSidebar },
  pairServer: { label: "Pair a server", keys: [], run: () => showModal("pair_server") },
  clientMenu: { label: "Client menu", keys: ["⌘,"], run: toggleClientMenu },
  shortcuts: { label: "Keyboard shortcuts", keys: ["⌘/"], run: () => showModal("shortcuts") },
  zoomIn: { label: "Zoom in", keys: ["⌘="], run: () => zoom("in") },
  zoomOut: { label: "Zoom out", keys: ["⌘-"], run: () => zoom("out") },
  actualSize: { label: "Actual size", keys: ["⌘0"], run: () => zoom("reset") },
  quit: { label: "Quit octopus", keys: ["⌘Q"], run: () => act(backend.quit()) },
} satisfies Record<string, Command>;

/** A key's modifiers and the key they hold. */
export const parseKey = (written: string) => ({
  command: written.includes("⌘"),
  control: written.includes("⌃"),
  shift: written.includes("⇧"),
  key: written.replace(/[⌘⌃⇧]/g, ""),
});

/** A key as this platform writes it. On Linux, ⌘ is Ctrl+Shift and ⌃ is Ctrl. */
const written = (key: string) => {
  if (mac) return key;
  const { command, control, shift, key: held } = parseKey(key);
  const name = held === "⇥" ? "Tab" : held;
  if (command) return `Ctrl+Shift+${name}`;
  return `${control ? "Ctrl+" : ""}${shift ? "Shift+" : ""}${name}`;
};

export const hint = (command: Command) => (command.keys[0] === undefined ? "" : written(command.keys[0]));
export const hints = (command: Command) => command.keys.map(written).join("  ");
