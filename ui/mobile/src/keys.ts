import type { IModes } from "@xterm/xterm";

export const ESC = "\x1b";

/** The cursor key ending in `code`, in the encoding the terminal's modes ask for. */
export const cursorKey = (code: string, modes: Pick<IModes, "applicationCursorKeysMode">) =>
  `${ESC}${modes.applicationCursorKeysMode ? "O" : "["}${code}`;

/** `steps` cursor keys along one axis: `backward` once for each step below zero, else `forward` once for each. */
export const cursorKeys = (
  steps: number,
  backward: string,
  forward: string,
  modes: Pick<IModes, "applicationCursorKeysMode">,
) => cursorKey(steps < 0 ? backward : forward, modes).repeat(Math.abs(steps));

/** How many lines and columns a caret moved between two offsets into a field of lines `lineChars` long. */
export const caretMove = (from: number, to: number, lineChars: number) => ({
  lines: Math.floor(to / lineChars) - Math.floor(from / lineChars),
  cols: (to % lineChars) - (from % lineChars),
});

/** Text as a paste writes it: each line break as Enter, between the bracketed-paste markers when the program asked. */
export const pasteKeys = (text: string, modes: Pick<IModes, "bracketedPasteMode">) => {
  const typed = text.replace(/\r?\n/g, "\r");
  return modes.bracketedPasteMode ? `${ESC}[200~${typed}${ESC}[201~` : typed;
};
