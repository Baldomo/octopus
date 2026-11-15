import assert from "node:assert/strict";
import { test } from "node:test";
import { cycled } from "./tabOrder.ts";

test("cycling the tabs wraps around both ends", () => {
  const order = ["a", "b", "c"];
  assert.equal(cycled(order, "a", 1), "b");
  assert.equal(cycled(order, "c", 1), "a");
  assert.equal(cycled(order, "a", -1), "c");
  assert.equal(cycled(order, undefined, 1), "a", "from the new page, forward starts at the first tab");
  assert.equal(cycled(order, undefined, -1), "c");
  assert.equal(cycled([], undefined, 1), undefined);
});
