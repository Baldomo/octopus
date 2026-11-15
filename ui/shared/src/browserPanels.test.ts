import assert from "node:assert/strict";
import { test } from "node:test";
import { heirOf, typedPage } from "./browserPanels.ts";

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
