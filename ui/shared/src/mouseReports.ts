import type { Terminal } from "@xterm/xterm";

/** A cell of the program's screen, counted from 0 at its top left. */
export type ScreenCell = { col: number; row: number };

const LEFT = 0;
const RELEASED = 3;
const WHEEL_UP = 64;
const WHEEL_DOWN = 65;
const SGR_MOUSE = 1006;
/** The furthest column or row an X10 report reaches while its byte stays ASCII, as the terminal's text input carries it. */
const X10_MOST = 0x7f - 33;

/**
 * One mouse report: SGR's `CSI < b ; x ; y M` (`m` on release), else X10's `CSI M` with each byte offset by 32, or
 * nothing for a cell past X10's reach.
 */
function report(button: number, { col, row }: ScreenCell, sgr: boolean, released = false): string {
  if (sgr) return `\x1b[<${button};${col + 1};${row + 1}${released ? "m" : "M"}`;
  if (col > X10_MOST || row > X10_MOST) return "";
  return `\x1b[M${String.fromCharCode(32 + (released ? RELEASED : button), 33 + col, 33 + row)}`;
}

/** One turn of the wheel at `cell`, toward the top for `way` -1 and the bottom for 1. */
export const wheelReport = (way: 1 | -1, cell: ScreenCell, sgr: boolean) =>
  report(way < 0 ? WHEEL_UP : WHEEL_DOWN, cell, sgr);

/** A left click at `cell`: its press, then its release. */
export const clickReport = (cell: ScreenCell, sgr: boolean) => report(LEFT, cell, sgr) + report(LEFT, cell, sgr, true);

/**
 * Whether the program has asked for SGR mouse reports (DECSET 1006), which xterm.js keeps out of its public modes. It
 * reads the program's own DECSET and DECRST as they pass, and `reset` forgets them with the terminal's reset.
 */
export function sgrMouse(parser: Pick<Terminal["parser"], "registerCsiHandler" | "registerEscHandler">) {
  let on = false;
  const heard = (set: boolean) => (params: (number | number[])[]) => {
    if (params.includes(SGR_MOUSE)) on = set;
    return false;
  };
  parser.registerCsiHandler({ prefix: "?", final: "h" }, heard(true));
  parser.registerCsiHandler({ prefix: "?", final: "l" }, heard(false));
  parser.registerEscHandler({ final: "c" }, () => {
    on = false;
    return false;
  });
  return { on: () => on, reset: () => void (on = false) };
}
