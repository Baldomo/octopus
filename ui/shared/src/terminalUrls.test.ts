import assert from "node:assert/strict";
import { test } from "node:test";
import { onLoopback, urlsIn } from "./terminalUrls.ts";

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

test("a URL is on a loopback host when its host is localhost or a loopback address", () => {
  for (const url of ["http://localhost:5173/a", "http://127.0.0.1:8080", "https://127.3.2.1/", "http://[::1]:3000/x"])
    assert.ok(onLoopback(url), url);
  for (const url of ["https://example.com", "http://localhost.example.com", "http://192.168.1.2:3000", "localhost"])
    assert.ok(!onLoopback(url), url);
});
