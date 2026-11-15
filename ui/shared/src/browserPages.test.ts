import assert from "node:assert/strict";
import { test } from "node:test";
import { downloadNote } from "./browserPages.ts";

test("a download's toast names its file and the folder it landed in, or says it failed", () => {
  assert.equal(downloadNote({ file: "a.zip", folder: "Downloads", success: true }), "Saved a.zip to Downloads");
  assert.equal(downloadNote({ file: "a.zip", folder: "Downloads", success: false }), "The download of a.zip failed");
});
