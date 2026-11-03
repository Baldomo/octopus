import assert from "node:assert/strict";
import { test } from "node:test";
import { urlsIn } from "./terminalUrls.ts";

const urls = (text: string) => urlsIn(text).map(({ url }) => url);

test("a URL keeps a balanced pair of parentheses and leaves an unbalanced one", () => {
  assert.deepEqual(urls("see https://en.wikipedia.org/wiki/Rust_(language)."), [
    "https://en.wikipedia.org/wiki/Rust_(language)",
  ]);
  assert.deepEqual(urls("(docs at https://example.com/guide)"), ["https://example.com/guide"]);
  assert.deepEqual(urls("https://example.com/a(b"), ["https://example.com/a"]);
});

test("a URL ends on none of the punctuation that closes a sentence or a bracket", () => {
  for (const end of [".", ",", ":", ";", "!", "?", "*", "~", "]", "}"])
    assert.deepEqual(urls(`open https://example.com/x${end} now`), ["https://example.com/x"], end);
  assert.deepEqual(urls("https://example.com/a.b,c/d"), ["https://example.com/a.b,c/d"]);
});

test("a quote or a backtick ends a URL", () => {
  for (const quote of ['"', "'", "`"])
    assert.deepEqual(urls(`${quote}https://example.com/q${quote}`), ["https://example.com/q"], quote);
  assert.deepEqual(urls("<https://example.com/angle>"), ["https://example.com/angle"]);
});

test("only http and https URLs are found, each at the index it starts", () => {
  assert.deepEqual(urlsIn("http://a.test https://b.test"), [
    { url: "http://a.test", at: 0 },
    { url: "https://b.test", at: 14 },
  ]);
  assert.deepEqual(urls("ftp://a.test file:///etc/hosts mailto:me@a.test httpx://a.test"), []);
});
