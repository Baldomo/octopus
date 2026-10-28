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

/** `shift` adds ⇧ on macOS; Linux's Ctrl+Shift already holds it. While `applies` is false the key passes through to
 * the focused element. */
export type Command = { label: string; key: string | null; shift?: true; applies?: () => boolean; run: () => void };

const openFiles = () => {
  const open = openView();
  return open === undefined ? undefined : filesOf(open);
};
const selectedRenders = () => {
  const active = openFiles()?.state.active;
  return active !== null && active !== undefined && renderable(active);
};

/** Every action a key or the palette runs; `key` is the letter after ⌘. */
export const commands = {
  taskView: { label: "Task view", key: "E", run: () => showView("task") },
  tasksView: { label: "Tasks view", key: "T", run: showTasks },
  files: { label: "Files", key: "1", run: () => showSurface("files") },
  browser: { label: "Browser", key: "2", run: () => showSurface("browser") },
  rendered: {
    label: "Code or rendered view",
    key: "V",
    shift: true,
    applies: () => Boolean(document.activeElement?.closest(".files")) && selectedRenders(),
    run: () => openFiles()?.toggleRendered(),
  },
  newTask: { label: "New task", key: "N", run: () => showModal("new_task") },
  search: { label: "Search tasks, files, commands", key: "K", run: () => showModal("palette") },
  sidebar: { label: "Toggle sidebar", key: "\\", run: toggleSidebar },
  pairServer: { label: "Pair a server", key: null, run: () => showModal("pair_server") },
  clientMenu: { label: "Client menu", key: ",", run: toggleClientMenu },
  shortcuts: { label: "Keyboard shortcuts", key: "/", run: () => showModal("shortcuts") },
  zoomIn: { label: "Zoom in", key: "=", run: () => zoom("in") },
  zoomOut: { label: "Zoom out", key: "-", run: () => zoom("out") },
  actualSize: { label: "Actual size", key: "0", run: () => zoom("reset") },
  quit: { label: "Quit octopus", key: "Q", run: () => act(backend.quit()) },
} satisfies Record<string, Command>;

/** On Linux, ⌘ is Ctrl+Shift. */
export const hint = (command: Command) =>
  command.key === null ? "" : mac ? `⌘${command.shift ? "⇧" : ""}${command.key}` : `Ctrl+Shift+${command.key}`;
