import assert from "node:assert/strict";
import { test } from "node:test";
import { moves, shownAt, slideFrom } from "./slide.ts";

/** A section's thread that grew a row of 24 as it moved down 10. */
const before = { x: 12, y: 100, width: 2, height: 96 };
const after = { x: 12, y: 110, width: 2, height: 120 };

test("a stretching item starts drawn over its old box", () => {
  assert.deepEqual(shownAt(after, slideFrom(before, after, true)), before);
});

test("a sliding item keeps its own size and starts at its old corner", () => {
  const slide = slideFrom(before, after, false);
  assert.deepEqual(slide, { a: 1, d: 1, e: 0, f: -10 });
  assert.deepEqual(shownAt(after, slide), { ...before, height: after.height });
});

test("a change of size alone moves only an item that stretches", () => {
  const grown = { ...before, height: 120 };
  assert.equal(moves(before, grown, false), false);
  assert.equal(moves(before, grown, true), true);
  assert.equal(moves(before, { ...before, y: 100.4 }, true), false);
});
