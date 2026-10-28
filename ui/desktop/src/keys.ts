import { commands, mac, type Command } from "@octopus/shared/commands.ts";
import { showModal, ui } from "@octopus/shared/state.ts";

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
