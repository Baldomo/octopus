import assert from "node:assert/strict";
import { test } from "node:test";
import { slotAmong } from "./slot.ts";

/** Three tabs whose middles, with the held tab taken out, stand at 50, 150 and 250. */
const middles = [50, 150, 250];

test("a held tab passes a middle, either way, only once its edge stands the slack past it, so a wiggle turns the order once", () => {
  assert.equal(slotAmong(middles, 153, 1), 1);
  assert.equal(slotAmong(middles, 154.5, 1), 2);
  assert.equal(slotAmong(middles, 147, 2), 2);
  assert.equal(slotAmong(middles, 145.5, 2), 1);
  let at = 1;
  const turns = [148, 155, 147, 152, 149, 151].filter((left) => at !== (at = slotAmong(middles, left, at)));
  assert.deepEqual(turns, [155]);
});

test("a fast pass crosses every middle it clears at once", () => {
  assert.equal(slotAmong(middles, 300, 0), 3);
  assert.equal(slotAmong(middles, -20, 3), 0);
});
