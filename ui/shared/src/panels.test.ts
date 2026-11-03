import assert from "node:assert/strict";
import { test } from "node:test";
import { createMemo, createRoot, createSignal, onCleanup } from "solid-js";
import type { PanelView } from "./generated.ts";
import { EachPanel } from "./panels.ts";

const shell = (id: string, holder: string | null = null): PanelView => ({
  id,
  name: id,
  running: true,
  lock_holder: holder === null ? null : { name: holder, key: null },
  cols: 80,
  rows: 24,
  listening: false,
  held_here: false,
});

function mounted(panels: PanelView[]) {
  const [list, setList] = createSignal(panels);
  const mounts: string[] = [];
  const unmounts: string[] = [];
  const children = createRoot(() =>
    EachPanel({
      get panels() {
        return list();
      },
      children: (panel) => {
        const holder = createMemo(() => panel().lock_holder?.name);
        mounts.push(panel().id);
        onCleanup(() => unmounts.push(`${panel().id}:${holder()}`));
        return panel().id;
      },
    }),
  ) as unknown as () => string[];
  children();
  return { children, setList, mounts, unmounts };
}

test("a closed panel's child unmounts reading the panel as it last was", () => {
  const shown = mounted([shell("agent"), shell("zsh", "phone")]);
  shown.setList([shell("agent")]);
  assert.deepEqual(shown.children(), ["agent"]);
  assert.deepEqual(shown.unmounts, ["zsh:phone"]);
});

test("a panel keeps its child while its fields change", () => {
  const shown = mounted([shell("zsh")]);
  shown.setList([shell("zsh", "phone")]);
  assert.deepEqual(shown.children(), ["zsh"]);
  assert.deepEqual(shown.mounts, ["zsh"]);
});
