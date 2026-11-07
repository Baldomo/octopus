/**
 * Whether a terminal's next stream claims its free lock. A look claims: the client starting to show the terminal while
 * its server is reachable. So does a stream that reopens on its own, after a drop or a reconnect, for a client that
 * held the lock when its stream ended. A lock another client frees stays free until someone looks.
 */
export function lockClaim() {
  let due = false;
  return {
    looked: () => {
      due = true;
    },
    ended: (held: boolean) => {
      due ||= held;
    },
    opened: () => {
      const claims = due;
      due = false;
      return claims;
    },
  };
}
