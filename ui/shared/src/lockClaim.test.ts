import assert from "node:assert/strict";
import { test } from "node:test";
import { lockClaim } from "./lockClaim.ts";

test("a stream opened by a look claims, once", () => {
  const claim = lockClaim();
  claim.looked();
  assert.equal(claim.opened(), true);
  assert.equal(claim.opened(), false);
});

test("a stream that reopens under a lock another client freed does not claim", () => {
  const claim = lockClaim();
  claim.looked();
  claim.opened();
  claim.ended(false);
  assert.equal(claim.opened(), false);
});

test("a stream that reopens under a lock this client held claims it back", () => {
  const claim = lockClaim();
  claim.looked();
  claim.opened();
  claim.ended(true);
  assert.equal(claim.opened(), true);
});
