import assert from "node:assert/strict";
import { test } from "node:test";
import { armOf, cellsOf, edge, reach, reaching, recoiling, releasing, tipOf } from "./tentacle.ts";

const strand = reach(armOf("leftBottom"), edge({ x: -2200, y: -870 }, Math.PI, -1), -1, 1);

test("a tentacle grows out of its arm's curl and its tip lands where it hooks", () => {
  assert.equal(reaching(strand, 0).len, strand.rest.len);
  const hooked = tipOf(strand, reaching(strand, 1));
  const end = tipOf(strand, { len: strand.path.len, curl: 0, open: 0, whip: 0 });
  assert.ok(Math.hypot(hooked.x - end.x, hooked.y - end.y) < 1);
  const rested = tipOf(strand, releasing(strand, 80, 1));
  assert.ok(
    rested.x > -2200 && Math.abs(rested.y - -870) < 40,
    `the released tip rests beside the edge, not at ${rested.x}`,
  );
});

test("a recoiled tentacle is back to its arm's length, and every pose fills with squares", () => {
  assert.equal(recoiling(strand, reaching(strand, 1), 1).len, strand.rest.len);
  for (const x of [0, 0.4, 1, 1.6]) {
    let squares = 0;
    cellsOf(strand, reaching(strand, x), (sx, sy) => {
      assert.ok(Number.isFinite(sx) && Number.isFinite(sy));
      squares++;
    });
    assert.ok(squares > 100);
  }
});
