import type { Offer, PairedClient } from "./generated";
import { myKey } from "./state";
import { ago } from "./time";

export const grouped = (code: string) => code.match(/.{1,4}/g)?.join(" ") ?? code;

export const inviteCode = (offered: Offer) => new URL(offered.invite).searchParams.get("code") ?? "";

/** Whether a server's paired client is this one: the server pins it by its key. */
export const isThisClient = (client: PairedClient) => client.key === myKey();

/** What a paired client's row says of it beside its name. */
export const clientStatus = (client: PairedClient, now: number) =>
  isThisClient(client)
    ? "this client"
    : client.connected
      ? "connected"
      : client.seen_at === null
        ? "never connected"
        : `seen ${ago(client.seen_at, now)}`;

/** The time left on an offer as m:ss, or "expired". */
export function expiry(offered: Offer, now: number) {
  const seconds = Math.max(0, Math.ceil((offered.expires_at - now) / 1000));
  return seconds === 0
    ? "expired"
    : `one pairing · expires in ${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}
