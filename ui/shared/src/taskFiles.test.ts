import assert from "node:assert/strict";
import { test } from "node:test";
import type { ServerView, TaskView } from "./generated.ts";
import { setServers } from "./state.ts";
import { pastedPaths } from "./taskFiles.ts";

setServers([{ name: "here", relay: null } as ServerView]);
const here = { at: { server: "here" }, task: { directory: "/work" } } as TaskView;
const away = { at: { server: "away" }, task: { directory: "/work" } } as TaskView;
const UNSAFE = "A path with a line break or a control character can't be pasted";

test("a pasted path reaches the shell as one word, quoted only when it needs to be", async () => {
  const paths = ["/Users/me/clip-1_final.mov", "/Users/me/My Clip (2).mov", "/tmp/it's here", "/tmp/café$HOME"];
  assert.equal(
    await pastedPaths(here, paths, assert.fail),
    "/Users/me/clip-1_final.mov '/Users/me/My Clip (2).mov' '/tmp/it'\\''s here' '/tmp/café$HOME'",
  );
});

test("a path with a line break or another control character pastes nothing, before anything is copied", async () => {
  for (const [task, incoming] of [
    [here, ["/tmp/a\nrm -rf ~"]],
    [here, ["/tmp/ok", "/tmp/a\x1b[2J"]],
    [away, [new File([], "a\nb")]],
  ] as const) {
    const problems: string[] = [];
    assert.equal(await pastedPaths(task, [...incoming], (problem) => problems.push(problem)), null);
    assert.deepEqual(problems, [UNSAFE]);
  }
});
