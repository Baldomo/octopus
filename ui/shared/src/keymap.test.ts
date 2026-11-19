import assert from "node:assert/strict";
import { test } from "node:test";
import { EDITS, exactKeys, type KeyAction, keyAction, keyCap, type Platform } from "./keymap.ts";

const press = (chord: string) => {
  const parts = chord.split("+");
  return {
    key: parts.at(-1)!,
    ctrlKey: parts.includes("ctrl"),
    altKey: parts.includes("alt"),
    metaKey: parts.includes("meta"),
    shiftKey: parts.includes("shift"),
  };
};
const acts = (chord: string, platform: Platform, exact = false): KeyAction | undefined =>
  keyAction(press(chord), platform, exact);

test("macOS and Linux bind their own line-editing keys to the bytes a shell's line editor reads", () => {
  const table: [Platform, string, string][] = [
    ["mac", "alt+Backspace", "\x1b\x7f"],
    ["mac", "meta+Backspace", "\x15"],
    ["mac", "alt+Delete", "\x1bd"],
    ["mac", "alt+ArrowLeft", "\x1bb"],
    ["mac", "alt+ArrowRight", "\x1bf"],
    ["mac", "meta+ArrowLeft", "\x01"],
    ["mac", "meta+ArrowRight", "\x05"],
    ["mac", "shift+Enter", "\x1b\r"],
    ["linux", "ctrl+Backspace", "\x1b\x7f"],
    ["linux", "ctrl+Delete", "\x1bd"],
    ["linux", "ctrl+ArrowLeft", "\x1bb"],
    ["linux", "ctrl+ArrowRight", "\x1bf"],
    ["linux", "Home", "\x01"],
    ["linux", "End", "\x05"],
    ["linux", "shift+Enter", "\x1b\r"],
  ];
  for (const [platform, chord, bytes] of table) assert.deepEqual(acts(chord, platform), { write: bytes }, chord);
  assert.deepEqual(acts("ctrl+shift+C", "linux"), { clipboard: "copy" });
  assert.deepEqual(acts("ctrl+shift+V", "linux"), { clipboard: "paste" });
});

test("app keys, typed characters and an interrupt stay with the app and xterm", () => {
  for (const chord of ["meta+c", "meta+v", "meta+k", "meta+e", "alt+é", "Backspace", "shift+meta+ArrowLeft"])
    assert.equal(acts(chord, "mac"), undefined, chord);
  for (const chord of ["ctrl+c", "ctrl+v", "alt+Backspace", "Backspace"])
    assert.equal(acts(chord, "linux"), undefined, chord);
});

test("a program reading exact keys gets the edit keys unmapped, and the newline and clipboard still act", () => {
  assert.equal(acts("alt+Backspace", "mac", true), undefined);
  assert.equal(acts("Home", "linux", true), undefined);
  assert.deepEqual(acts("shift+Enter", "mac", true), { write: "\x1b\r" });
  assert.deepEqual(acts("ctrl+shift+V", "linux", true), { clipboard: "paste" });
});

test("each edit's key cap shows its macOS key", () => {
  assert.deepEqual(EDITS.map(keyCap), ["⌘←", "⌥←", "⌥→", "⌘→", "⌥⌫", "⌥⌦", "⌘⌫"]);
});

function fakeProgram() {
  const csi: Record<string, (params: (number | number[])[]) => boolean | Promise<boolean>> = {};
  let esc = () => false;
  let screen: "normal" | "alternate" = "normal";
  const exact = exactKeys(
    {
      registerCsiHandler: (id: { prefix?: string; final: string }, callback) => {
        csi[`${id.prefix}${id.final}`] = callback;
        return { dispose: () => {} };
      },
      registerEscHandler: (_id, callback) => {
        esc = callback as () => boolean;
        return { dispose: () => {} };
      },
    },
    () => screen,
  );
  return {
    exact,
    csi: (id: string, ...params: number[]) => assert.equal(csi[id]!(params), false),
    screen: (shown: typeof screen) => (screen = shown),
    fullReset: () => esc(),
  };
}

test("kitty flags hold per screen until popped, and modifyOtherKeys level 2 until turned off or reset", () => {
  const program = fakeProgram();
  assert.equal(program.exact.on(), false);
  program.screen("alternate");
  program.csi(">u", 1);
  assert.equal(program.exact.on(), true);
  program.screen("normal");
  assert.equal(program.exact.on(), false);
  program.screen("alternate");
  program.csi("=u", 0, 1);
  assert.equal(program.exact.on(), false);
  program.csi("=u", 3, 2);
  assert.equal(program.exact.on(), true);
  program.csi("<u", 0);
  assert.equal(program.exact.on(), false);

  program.csi(">m", 4, 2);
  assert.equal(program.exact.on(), true);
  program.csi(">m", 4, 0);
  assert.equal(program.exact.on(), false);
  program.csi(">m", 4, 1);
  assert.equal(program.exact.on(), false);
  program.csi(">m", 4, 2);
  program.csi(">n", 4);
  assert.equal(program.exact.on(), false);
  program.csi(">m", 1, 2);
  assert.equal(program.exact.on(), false);

  program.csi(">u", 1);
  program.csi(">m", 4, 2);
  program.fullReset();
  assert.equal(program.exact.on(), false);
  program.csi(">u", 1);
  program.exact.reset();
  assert.equal(program.exact.on(), false);
});
