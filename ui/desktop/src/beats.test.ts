import assert from "node:assert/strict";
import { test } from "node:test";
import { nextBeat, type Beat, type Facts } from "./beats.ts";

const fresh: Facts = { installs: true, showing: false, answered: false, sidebar: false, workspaces: 0, tasks: 0 };

/** The beats a run of facts walks through, from the launch. */
function walk(run: Partial<Facts>[]): Beat[] {
  let [beat, facts] = ["wake" as Beat, fresh];
  return run.map((change) => {
    facts = { ...facts, ...change };
    beat = nextBeat(beat, facts);
    return beat;
  });
}

test("a first open waits on the server, the sidebar, the first workspace and the first task, in that order", () => {
  const beats = walk([
    {},
    { answered: true },
    { sidebar: true },
    { workspaces: 1 },
    { tasks: 1 },
    { sidebar: false, workspaces: 0, tasks: 0 },
  ]);
  assert.deepEqual(beats, ["wake", "reach", "offer", "hand_off", "done", "done"]);
});

test("a launch that installs nothing, or finds a workspace once the server answers, plays no first open", () => {
  assert.equal(nextBeat("wake", { ...fresh, installs: false }), "off");
  assert.equal(nextBeat("wake", { ...fresh, answered: true, workspaces: 2 }), "done");
});

test("no beat moves on before its event, and facts that arrive together land on the last beat they reach", () => {
  assert.deepEqual(walk([{ sidebar: true }, { workspaces: 1 }]), ["wake", "wake"]);
  assert.equal(nextBeat("wake", { ...fresh, answered: true, sidebar: true }), "offer");
  assert.equal(nextBeat("reach", { ...fresh, answered: true, workspaces: 1 }), "reach");
});

test("the first open waits on the show to end before it reaches", () => {
  assert.deepEqual(walk([{ showing: true }, { answered: true }, { showing: false }]), ["wake", "wake", "reach"]);
});

test("a show played again holds every beat while its tour works the window", () => {
  const beats = walk([
    { answered: true },
    { showing: true },
    { sidebar: true, workspaces: 1, tasks: 1 },
    { showing: false, sidebar: false, workspaces: 0, tasks: 0 },
  ]);
  assert.deepEqual(beats, ["reach", "reach", "reach", "reach"]);
});
