import assert from "node:assert/strict";
import { test } from "node:test";
import { heirOf, openedAt, typedPage } from "./browserPanels.ts";

test("a typed web address opens over https", () => {
  assert.equal(typedPage("example.com"), "https://example.com");
  assert.equal(typedPage("docs.example.com:8443/guide?q=1"), "https://docs.example.com:8443/guide?q=1");
});

test("a typed local address opens over http", () => {
  for (const local of ["localhost:5173/app", "app.localhost", "192.168.1.20", "[::1]:3000", "printer.local"])
    assert.equal(typedPage(local), `http://${local}`, local);
});

test("closing the selected tab selects the next tab, else the one before", () => {
  assert.equal(heirOf(["a", "b", "c"], ["a", "c"], "b"), "c");
  assert.equal(heirOf(["a", "b", "c"], ["a", "b"], "c"), "b");
  assert.equal(heirOf(["a", "b", "c"], ["a"], "b"), "a", "a tab closed with it is passed over");
  assert.equal(heirOf(["a"], [], "a"), undefined);
});

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
