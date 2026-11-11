import assert from "node:assert/strict";
import { test } from "node:test";
import { IMAGE_BYTES, openingOf, PDF_BYTES, rangeOf, sizeOf, TEXT_BYTES } from "./opening.ts";

const bytes = (text: string) => new TextEncoder().encode(text);

test("media opens by its extension, and an image or a PDF too big to hold stays unshown", () => {
  assert.equal(openingOf("shots/Home.PNG", 10, new Uint8Array([0x89, 0x50, 0])), "image");
  assert.equal(openingOf("docs/spec.pdf", 10, bytes("%PDF-1.7")), "pdf");
  assert.equal(openingOf("demo.mov", 10, new Uint8Array([0])), "video");
  assert.equal(openingOf("voice.m4a", 10, new Uint8Array([0])), "audio");
  assert.equal(openingOf("huge.png", IMAGE_BYTES + 1, new Uint8Array([0])), "large");
  assert.equal(openingOf("scan.pdf", IMAGE_BYTES + 1, bytes("%PDF-1.7")), "pdf");
  assert.equal(openingOf("archive.pdf", PDF_BYTES + 1, bytes("%PDF-1.7")), "large");
  assert.equal(openingOf("film.mp4", PDF_BYTES + 1, new Uint8Array([0])), "video");
});

test("any other file opens as text from its bytes alone", () => {
  assert.equal(openingOf("Makefile", 5, bytes("all:\n")), "text");
  assert.equal(openingOf("notes.bin", 6, bytes("héllo")), "text");
  assert.equal(openingOf("app.exe", 4, new Uint8Array([0x4d, 0x5a, 0, 0])), "binary");
  assert.equal(openingOf("latin1.txt", 3, new Uint8Array([0x63, 0xe9, 0x74])), "binary");
  assert.equal(openingOf("big.log", TEXT_BYTES + 1, bytes("line\n")), "large");
});

test("a character the first bytes cut in two still reads as text", () => {
  const cut = bytes("ab€").slice(0, 3);
  assert.equal(openingOf("cut.txt", 100, cut), "text");
  assert.equal(openingOf("cut.txt", 3, cut), "binary");
});

test("a size reads in the unit that keeps it short", () => {
  assert.equal(sizeOf(0), "0 bytes");
  assert.equal(sizeOf(1000), "1000 bytes");
  assert.equal(sizeOf(3_400_000), "3.2 MB");
  assert.equal(sizeOf(300 << 20), "300 MB");
});

test("a range the server cuts short is asked for again from where its reply ended", async (t) => {
  const file = Uint8Array.from({ length: 100 }, (_, at) => at);
  const asked: string[] = [];
  t.mock.method(globalThis, "fetch", async (_url: string, init?: RequestInit) => {
    const range = new Headers(init?.headers).get("Range") ?? "";
    asked.push(range);
    const [first = 0, last = 0] = range.replace("bytes=", "").split("-").map(Number);
    const end = Math.min(last, first + 29);
    return new Response(file.slice(first, end + 1), {
      status: 206,
      headers: { "Content-Range": `bytes ${first}-${end}/${file.length}` },
    });
  });
  assert.deepEqual(
    await rangeOf("octopus-file://root/a.pdf", 10, 80, new AbortController().signal),
    file.slice(10, 80),
  );
  assert.deepEqual(asked, ["bytes=10-79", "bytes=40-79", "bytes=70-79"]);
});
