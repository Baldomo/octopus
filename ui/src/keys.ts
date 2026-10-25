import { showModal, showSurface, showTasks, showView, toggleSidebar, ui } from "./state";

export const mac = navigator.userAgent.includes("Mac");

export type Command = { label: string; key: string | null; run: () => void };

/** Every action a key or the palette runs; `key` is the letter after ⌘. */
export const commands = {
  taskView: { label: "Task view", key: "E", run: () => showView("task") },
  tasksView: { label: "Tasks view", key: "T", run: showTasks },
  files: { label: "Files", key: "1", run: () => showSurface("files") },
  browser: { label: "Browser", key: "2", run: () => showSurface("browser") },
  newTask: { label: "New task", key: "N", run: () => showModal("new_task") },
  search: { label: "Search tasks, files, commands", key: "K", run: () => showModal("palette") },
  sidebar: { label: "Toggle sidebar", key: "\\", run: toggleSidebar },
  pairServer: { label: "Pair a server", key: null, run: () => showModal("pair_server") },
} satisfies Record<string, Command>;

/** On Linux and Windows, ⌘ is Ctrl+Shift. */
export const hint = (command: Command) =>
  command.key === null ? "" : mac ? `⌘${command.key}` : `Ctrl+Shift+${command.key}`;

const held = (event: KeyboardEvent) =>
  mac
    ? event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey
    : event.ctrlKey && event.shiftKey && !event.metaKey && !event.altKey;

/** The physical key under each command letter, so Shift on Linux and Windows changes nothing. */
const codeOf = (key: string) => (key === "\\" ? "Backslash" : /\d/.test(key) ? `Digit${key}` : `Key${key}`);
const byCode = new Map(
  Object.values(commands).flatMap((command: Command) => (command.key === null ? [] : [[codeOf(command.key), command]])),
);

export function bindKeys() {
  window.addEventListener(
    "keydown",
    (event) => {
      const command = held(event) ? byCode.get(event.code) : undefined;
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
