import { createSignal } from "solid-js";

/**
 * Whether a terminal's next stream claims its free lock, and whether a claim is under way. A look claims: the client
 * starting to show the terminal while its server is reachable. So does a stream that reopens on its own, after a drop
 * or a reconnect, for a client that held the lock when its stream ended. A lock another client frees stays free until
 * someone looks. A claim is under way from the look, or the reopening, until it settles.
 */
export function lockClaim() {
  let looked = false;
  let held = false;
  const [pending, setPending] = createSignal(false);
  return {
    pending,
    looked: () => {
      looked = true;
      setPending(true);
    },
    ended: (wasHeld: boolean) => {
      looked = false;
      held ||= wasHeld;
      setPending(held);
    },
    opened: () => {
      const claims = looked || held;
      looked = held = false;
      if (claims) setPending(true);
      return claims;
    },
    settled: () => setPending(false),
  };
}
