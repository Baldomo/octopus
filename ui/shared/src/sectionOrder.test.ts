import assert from "node:assert/strict";
import { test } from "node:test";
import { inOrder, inSectionOrder, moveTo, placeAmong } from "./sectionOrder.ts";

const named = (...names: string[]) => names.map((name) => ({ name }));
const names = (items: { name: string }[]) => items.map((item) => item.name);

test("sections follow the saved order, and new ones append in their default order", () => {
  const sections = named("acme", "dotfiles", "octopus", "scratch");
  assert.deepEqual(names(inSectionOrder(sections, ["octopus", "acme"])), ["octopus", "acme", "dotfiles", "scratch"]);
});

test("a saved name no section shows takes no place on screen", () => {
  assert.deepEqual(names(inSectionOrder(named("acme", "octopus"), ["gone", "octopus"])), ["octopus", "acme"]);
});

test("same-named items, like one section's workspaces, keep their order among themselves", () => {
  const workspaces = [
    { name: "octopus", server: "laptop" },
    { name: "acme", server: "laptop" },
    { name: "octopus", server: "mac-mini" },
  ];
  const sorted = inSectionOrder(workspaces, ["octopus"]).map((each) => `${each.name}@${each.server}`);
  assert.deepEqual(sorted, ["octopus@laptop", "octopus@mac-mini", "acme@laptop"]);
});

test("a moved section lands at its new index among the others", () => {
  assert.deepEqual(moveTo(["a", "b", "c", "d"], "a", 2), ["b", "c", "a", "d"]);
  assert.deepEqual(moveTo(["a", "b", "c", "d"], "d", 0), ["d", "a", "b", "c"]);
});

test("a drag moves only the dragged name, so an offline section keeps its saved place", () => {
  const saved = ["willow", "offline", "acme", "octopus"];
  assert.deepEqual(placeAmong(saved, ["acme", "willow", "octopus"], "willow"), ["offline", "acme", "willow", "octopus"]);
  assert.deepEqual(placeAmong(saved, ["acme", "octopus", "willow"], "willow"), ["offline", "acme", "octopus", "willow"]);
});

test("a drag saves a section new to the saved order where it shows", () => {
  assert.deepEqual(placeAmong(["octopus"], ["acme", "octopus", "dotfiles"], "acme"), ["acme", "octopus", "dotfiles"]);
});

test("tabs held in a dragged order lose one that closes meanwhile, and one that opens follows them", () => {
  const held = moveTo(["a", "b", "c"], "c", 0);
  assert.deepEqual(
    inOrder(["a", "c", "d"], held, (key) => key),
    ["c", "a", "d"],
  );
});
