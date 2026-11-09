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

test("a claim is under way from the look until it settles", () => {
  const claim = lockClaim();
  claim.looked();
  assert.equal(claim.opened(), true);
  assert.equal(claim.pending(), true);
  claim.settled();
  assert.equal(claim.pending(), false);
});

test("a look whose stream ends before it opens claims nothing later", () => {
  const claim = lockClaim();
  claim.looked();
  claim.ended(false);
  assert.equal(claim.pending(), false);
  assert.equal(claim.opened(), false);
});
