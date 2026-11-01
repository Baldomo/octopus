import assert from "node:assert/strict";
import { test } from "node:test";
import { clickReport, sgrMouse, wheelReport } from "./mouseReports.ts";

test("an SGR click reports its press and release at the 1-based cell", () => {
  assert.equal(clickReport({ col: 4, row: 9 }, true), "\x1b[<0;5;10M\x1b[<0;5;10m");
});

test("an X10 click reports button 0 then the release button 3, each byte offset by 32", () => {
  assert.equal(clickReport({ col: 0, row: 0 }, false), "\x1b[M !!\x1b[M#!!");
});

test("an SGR wheel turn reports 64 toward the top and 65 toward the bottom", () => {
  assert.equal(wheelReport(-1, { col: 2, row: 3 }, true), "\x1b[<64;3;4M");
  assert.equal(wheelReport(1, { col: 2, row: 3 }, true), "\x1b[<65;3;4M");
});

test("an X10 wheel turn offsets its button and cell by 32", () => {
  assert.equal(wheelReport(-1, { col: 2, row: 3 }, false), "\x1b[M`#$");
  assert.equal(wheelReport(1, { col: 2, row: 3 }, false), "\x1b[Ma#$");
});

test("an X10 report past the last cell whose byte stays ASCII reports nothing, where SGR still reaches", () => {
  assert.equal(clickReport({ col: 94, row: 0 }, false), "\x1b[M \x7f!\x1b[M#\x7f!");
  assert.equal(clickReport({ col: 95, row: 0 }, false), "");
  assert.equal(wheelReport(1, { col: 0, row: 94 }, true), "\x1b[<65;1;95M");
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

test("SGR reports follow the program's DECSET and DECRST of 1006, and xterm still handles them", () => {
  const program = fakeParser();
  const sgr = sgrMouse(program.parser);
  assert.equal(sgr.on(), false);
  assert.equal(program.set(1000, 1006), false);
  assert.equal(sgr.on(), true);
  program.unset(1000);
  assert.equal(sgr.on(), true);
  program.unset(1006);
  assert.equal(sgr.on(), false);
});

test("a full reset, the program's or the terminal's, forgets SGR reports", () => {
  const program = fakeParser();
  const sgr = sgrMouse(program.parser);
  program.set(1006);
  program.fullReset();
  assert.equal(sgr.on(), false);
  program.set(1006);
  sgr.reset();
  assert.equal(sgr.on(), false);
});
