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
  ui,
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

const held = (event: KeyboardEvent) =>
  mac
    ? event.metaKey && !event.ctrlKey && !event.altKey
    : event.ctrlKey && event.shiftKey && !event.metaKey && !event.altKey;

/** The physical key under each command letter, so Shift on Linux changes nothing. */
const PUNCTUATION: Record<string, string> = {
  "\\": "Backslash",
  ",": "Comma",
  "/": "Slash",
  "=": "Equal",
  "-": "Minus",
};
const codeOf = (key: string) => PUNCTUATION[key] ?? (/\d/.test(key) ? `Digit${key}` : `Key${key}`);
const chordOf = (code: string, shift: boolean) => (mac && shift ? `Shift+${code}` : code);
const byChord = new Map(
  Object.values(commands).flatMap((command: Command) =>
    command.key === null ? [] : [[chordOf(codeOf(command.key), command.shift === true), command]],
  ),
);

export function bindKeys() {
  window.addEventListener(
    "keydown",
    (event) => {
      const chorded = held(event) ? byChord.get(chordOf(event.code, event.shiftKey)) : undefined;
      const command = chorded?.applies?.() === false ? undefined : chorded;
      if (command !== undefined) {
        event.preventDefault();
        event.stopPropagation();
        command.run();
      } else if (event.key === "Escape" && ui.modal !== null) {
        event.preventDefault();
        showModal(null);
      }
    },
    { capture: true },
  );
}
