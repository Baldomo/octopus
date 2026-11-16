import assert from "node:assert/strict";
import { test } from "node:test";
import { inOrder, inSectionOrder, moveTo, placeAmong } from "./sectionOrder.ts";

const named = (...names: string[]) => names.map((name) => ({ name }));
const names = (items: { name: string }[]) => items.map((item) => item.name);

test("sections follow the saved order, new ones append in their default order, and same-named ones keep theirs", () => {
  const sections = named("acme", "dotfiles", "octopus", "scratch");
  const saved = ["gone", "octopus", "acme"];
  assert.deepEqual(names(inSectionOrder(sections, saved)), ["octopus", "acme", "dotfiles", "scratch"]);
  const workspaces = [
    { name: "octopus", server: "laptop" },
    { name: "acme", server: "laptop" },
    { name: "octopus", server: "mac-mini" },
  ];
  const sorted = inSectionOrder(workspaces, ["octopus"]).map((each) => `${each.name}@${each.server}`);
  assert.deepEqual(sorted, ["octopus@laptop", "octopus@mac-mini", "acme@laptop"]);
});

test("a drag moves only the dragged name, keeping an offline section's saved place and saving a new one where it shows", () => {
  const saved = ["willow", "offline", "acme", "octopus"];
  assert.deepEqual(placeAmong(saved, ["acme", "willow", "octopus"], "willow"), [
    "offline",
    "acme",
    "willow",
    "octopus",
  ]);
  assert.deepEqual(placeAmong(saved, ["acme", "octopus", "willow"], "willow"), [
    "offline",
    "acme",
    "octopus",
    "willow",
  ]);
  assert.deepEqual(placeAmong(["octopus"], ["acme", "octopus", "dotfiles"], "acme"), ["acme", "octopus", "dotfiles"]);
});

test("tabs held in a dragged order lose one that closes meanwhile, and one that opens follows them", () => {
  const held = moveTo(["a", "b", "c"], "c", 0);
  assert.deepEqual(
    inOrder(["a", "c", "d"], held, (key) => key),
    ["c", "a", "d"],
  );
});
