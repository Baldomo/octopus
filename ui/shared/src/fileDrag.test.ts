import assert from "node:assert/strict";
import { test } from "node:test";
import { pastesSafely, shellQuoted } from "./fileDrag.ts";

test("a dropped path reaches the shell as one word, quoted only when it needs to be", () => {
  assert.equal(shellQuoted("/Users/me/clip-1_final.mov"), "/Users/me/clip-1_final.mov");
  assert.equal(shellQuoted("/Users/me/My Clip (2).mov"), "'/Users/me/My Clip (2).mov'");
  assert.equal(shellQuoted("/tmp/it's here"), "'/tmp/it'\\''s here'");
  assert.equal(shellQuoted("/tmp/café$HOME"), "'/tmp/café$HOME'");
});

test("a path with a line break or another control character is never pasted", () => {
  assert.equal(pastesSafely("/Users/me/My Clip (2).mov"), true);
  assert.equal(pastesSafely("/tmp/a\nrm -rf ~"), false);
  assert.equal(pastesSafely("/tmp/a\x1b[2J"), false);
});
