import assert from "node:assert/strict";
import { test } from "node:test";
import { holds } from "./detent.ts";

test("a pointer nearing the middle catches the divider within 6px", () => {
  assert.equal(holds(7, false), false);
  assert.equal(holds(6, false), true);
  assert.equal(holds(-6, false), true);
});

test("a held divider lets go only once the pointer pulls 12px away", () => {
  assert.equal(holds(12, true), true);
  assert.equal(holds(-12, true), true);
  assert.equal(holds(13, true), false);
});

test("a pointer that jumps across the middle in one move passes it by", () => {
  assert.equal(holds(-30, false), false);
  assert.equal(holds(30, false), false);
});
