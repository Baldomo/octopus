import assert from "node:assert/strict";
import { test } from "node:test";
import { holds } from "./detent.ts";

test("a divider catches within 6px of the middle and lets go only once the pointer pulls 12px away", () => {
  assert.equal(holds(7, false), false);
  assert.equal(holds(6, false), true);
  assert.equal(holds(-6, false), true);
  assert.equal(holds(12, true), true);
  assert.equal(holds(-12, true), true);
  assert.equal(holds(13, true), false);
});
