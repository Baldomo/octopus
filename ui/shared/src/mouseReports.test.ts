import assert from "node:assert/strict";
import { test } from "node:test";
import { clickReport, sgrMouse, wheelReport } from "./mouseReports.ts";

test("clicks and wheel turns report in SGR at the 1-based cell, or in X10 offset by 32 while the byte stays ASCII", () => {
  const reports: [string, string][] = [
    [clickReport({ col: 4, row: 9 }, true), "\x1b[<0;5;10M\x1b[<0;5;10m"],
    [clickReport({ col: 0, row: 0 }, false), "\x1b[M !!\x1b[M#!!"],
    [wheelReport(-1, { col: 2, row: 3 }, true), "\x1b[<64;3;4M"],
    [wheelReport(1, { col: 2, row: 3 }, true), "\x1b[<65;3;4M"],
    [wheelReport(-1, { col: 2, row: 3 }, false), "\x1b[M`#$"],
    [wheelReport(1, { col: 2, row: 3 }, false), "\x1b[Ma#$"],
    [clickReport({ col: 94, row: 0 }, false), "\x1b[M \x7f!\x1b[M#\x7f!"],
    [clickReport({ col: 95, row: 0 }, false), ""],
    [wheelReport(1, { col: 0, row: 94 }, true), "\x1b[<65;1;95M"],
  ];
  for (const [report, expected] of reports) assert.equal(report, expected);
});

function fakeParser() {
  const csi: Record<string, (params: (number | number[])[]) => boolean> = {};
  let esc = () => false;
  return {
    parser: {
      registerCsiHandler: (id: { final: string }, callback: (params: (number | number[])[]) => boolean) => {
        csi[id.final] = callback;
        return { dispose: () => {} };
      },
      registerEscHandler: (_id: unknown, callback: () => boolean) => {
        esc = callback;
        return { dispose: () => {} };
      },
    },
    set: (...params: number[]) => csi.h!(params),
    unset: (...params: number[]) => csi.l!(params),
    fullReset: () => esc(),
  };
}

test("SGR reports follow the program's DECSET and DECRST of 1006 until a full reset, the program's or the terminal's", () => {
  const program = fakeParser();
  const sgr = sgrMouse(program.parser);
  assert.equal(sgr.on(), false);
  assert.equal(program.set(1000, 1006), false);
  assert.equal(sgr.on(), true);
  program.unset(1000);
  assert.equal(sgr.on(), true);
  program.unset(1006);
  assert.equal(sgr.on(), false);
  program.set(1006);
  program.fullReset();
  assert.equal(sgr.on(), false);
  program.set(1006);
  sgr.reset();
  assert.equal(sgr.on(), false);
});
