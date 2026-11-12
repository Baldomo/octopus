import { commands, mac, parseKey, type Command } from "@octopus/shared/commands.ts";
import { floatingOpen } from "@octopus/shared/Menu.tsx";
import { act, showModal, ui } from "@octopus/shared/state.ts";
import { desktop, desktopEvents } from "./backend";
import type { MenuGroup } from "./generated";
import { layer } from "./layer";

/** The physical key under each command key, so Shift on Linux changes nothing. */
const CODES: Record<string, string> = {
  "\\": "Backslash",
  ",": "Comma",
  "/": "Slash",
  "=": "Equal",
  "-": "Minus",
  "[": "BracketLeft",
  "]": "BracketRight",
  "⇥": "Tab",
};
const codeOf = (key: string) => CODES[key] ?? (/\d/.test(key) ? `Digit${key}` : `Key${key}`);

const chord = (control: boolean, command: boolean, shift: boolean, code: string) =>
  `${control ? "⌃" : ""}${command ? "⌘" : ""}${shift ? "⇧" : ""}${code}`;

/** The press a written key takes on this platform: on Linux, ⌘ is Ctrl+Shift and ⌃ is Ctrl. */
const pressOf = (written: string) => {
  const { command, control, shift, key } = parseKey(written);
  const code = codeOf(key);
  return mac ? chord(control, command, shift, code) : chord(true, false, command || shift, code);
};

const byPress = new Map<string, Command[]>();
for (const command of Object.values(commands) as Command[]) {
  for (const key of command.keys) byPress.set(pressOf(key), [...(byPress.get(pressOf(key)) ?? []), command]);
}

/** Runs the first command of the press that applies, and tells whether one did. */
const run = (press: string, inPage: boolean) => {
  const command = byPress.get(press)?.find((each) => each.applies?.(inPage) !== false);
  command?.run();
  return command !== undefined;
};

const BROWSER: (keyof typeof commands)[] = [
  "newTab",
  "location",
  "back",
  "forward",
  "reload",
  "reloadFromOrigin",
  "find",
  "nextTab",
  "previousTab",
  "closePage",
  "reopenTab",
];
/** The app's own menu holds Quit. */
const VIEW = (Object.keys(commands) as (keyof typeof commands)[]).filter(
  (name) => name !== "quit" && !BROWSER.includes(name),
);

const accelerator = (written: string) => {
  const { command, control, shift, key } = parseKey(written);
  return `${command ? "Cmd+" : ""}${control ? "Ctrl+" : ""}${shift ? "Shift+" : ""}${codeOf(key)}`;
};

/** The menu bar takes a command's key while a browser page holds the focus, which the window never hears. */
function listInMenu() {
  const group = (title: string, names: (keyof typeof commands)[]): MenuGroup => ({
    title,
    commands: names.flatMap((name) => {
      const command: Command = commands[name];
      return command.keys.map((key) => ({ key, label: command.label, accelerator: accelerator(key) }));
    }),
  });
  act(desktop.appMenu([group("View", VIEW), group("Browser", BROWSER)]));
  act(desktopEvents.menuCommand(({ key, inPage }) => run(pressOf(key), inPage)));
}

export function bindKeys() {
  window.addEventListener(
    "keydown",
    (event) => {
      const held = (event.metaKey || event.ctrlKey) && !event.altKey;
      if (held && run(chord(event.ctrlKey, event.metaKey, event.shiftKey, event.code), false)) {
        event.preventDefault();
        event.stopPropagation();
      } else if (event.key === "Escape" && ui.modal !== null && !floatingOpen()) {
        event.preventDefault();
        showModal(null);
      }
    },
    { capture: true },
  );
  if (mac && layer === "base") listInMenu();
}
