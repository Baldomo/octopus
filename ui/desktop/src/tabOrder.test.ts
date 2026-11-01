import assert from "node:assert/strict";
import { test } from "node:test";
import { cycled, heirOf, openedAt } from "./tabOrder.ts";

test("a tab a page opens goes right after the page's tab and the tabs it opened before", () => {
  const openers = new Map([
    ["b1", "a"],
    ["b2", "a"],
    ["c1", "c"],
  ]);
  const openerOf = (tab: string) => openers.get(tab);
  assert.equal(openedAt(["a", "c"], "a", openerOf), 1);
  assert.equal(openedAt(["a", "b1", "b2", "c"], "a", openerOf), 3);
  assert.equal(openedAt(["a", "c", "b1"], "a", openerOf), 1, "an opened tab moved away no longer extends the run");
  assert.equal(openedAt(["a", "c", "c1"], "c", openerOf), 3);
  assert.equal(openedAt(["a", "c"], "gone", openerOf), 2, "a gone opener's tab joins the end");
});

test("closing the selected tab selects the next tab, else the one before", () => {
  assert.equal(heirOf(["a", "b", "c"], ["a", "c"], "b"), "c");
  assert.equal(heirOf(["a", "b", "c"], ["a", "b"], "c"), "b");
  assert.equal(heirOf(["a", "b", "c"], ["a"], "b"), "a", "a tab closed with it is passed over");
  assert.equal(heirOf(["a"], [], "a"), undefined);
});

test("cycling the tabs wraps around both ends", () => {
  const order = ["a", "b", "c"];
  assert.equal(cycled(order, "a", 1), "b");
  assert.equal(cycled(order, "c", 1), "a");
  assert.equal(cycled(order, "a", -1), "c");
  assert.equal(cycled(order, undefined, 1), "a", "from the new page, forward starts at the first tab");
  assert.equal(cycled(order, undefined, -1), "c");
  assert.equal(cycled([], undefined, 1), undefined);
});
