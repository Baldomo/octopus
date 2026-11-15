import assert from "node:assert/strict";
import { test } from "node:test";
import { heirOf } from "./browserPanels.ts";

test("closing the selected tab selects the next tab, else the one before", () => {
  assert.equal(heirOf(["a", "b", "c"], ["a", "c"], "b"), "c");
  assert.equal(heirOf(["a", "b", "c"], ["a", "b"], "c"), "b");
  assert.equal(heirOf(["a", "b", "c"], ["a"], "b"), "a", "a tab closed with it is passed over");
  assert.equal(heirOf(["a"], [], "a"), undefined);
});
