import assert from "node:assert/strict";
import { test } from "node:test";
import { lockClaim } from "./lockClaim.ts";

test("a look claims once, and a reopening claims only a lock this client held", () => {
  const claim = lockClaim();
  claim.looked();
  assert.equal(claim.opened(), true);
  assert.equal(claim.opened(), false);
  claim.ended(false);
  assert.equal(claim.opened(), false);
  claim.ended(true);
  assert.equal(claim.opened(), true);
  claim.looked();
  claim.ended(false);
  assert.equal(claim.pending(), false);
  assert.equal(claim.opened(), false);
});

test("a claim is under way from the look until it settles", () => {
  const claim = lockClaim();
  claim.looked();
  assert.equal(claim.opened(), true);
  assert.equal(claim.pending(), true);
  claim.settled();
  assert.equal(claim.pending(), false);
});
