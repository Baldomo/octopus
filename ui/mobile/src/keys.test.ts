import assert from "node:assert/strict";
import { test } from "node:test";
import { caretMove, cursorKeys, pasteKeys } from "./keys.ts";

const normal = { applicationCursorKeysMode: false, bracketedPasteMode: false };
const application = { applicationCursorKeysMode: true, bracketedPasteMode: true };

test("a caret's move counts the lines and columns it crossed in a field of fixed-length lines", () => {
  assert.deepEqual(caretMove(171, 174, 49), { lines: 0, cols: 3 });
  assert.deepEqual(caretMove(171, 167, 49), { lines: 0, cols: -4 });
  assert.deepEqual(caretMove(171, 220, 49), { lines: 1, cols: 0 });
  assert.deepEqual(caretMove(171, 121, 49), { lines: -1, cols: -1 });
});

test("cursor keys repeat once a step, in the encoding the terminal asked for", () => {
  assert.equal(cursorKeys(3, "D", "C", normal), "\x1b[C\x1b[C\x1b[C");
  assert.equal(cursorKeys(-2, "D", "C", application), "\x1bOD\x1bOD");
  assert.equal(cursorKeys(0, "A", "B", normal), "");
});

test("a paste writes its line breaks as Enter, bracketed only when the program asked", () => {
  assert.equal(pasteKeys("ls\r\necho hi\n", normal), "ls\recho hi\r");
  assert.equal(pasteKeys("a\nb", application), "\x1b[200~a\rb\x1b[201~");
});
