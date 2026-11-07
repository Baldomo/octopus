import { createResource } from "solid-js";
import { backend } from "./backend";
import type { Offer, PairedAccount, PairedClient } from "./generated";
import { myKey, serverView } from "./state";
import { ago } from "./time";

export const grouped = (code: string) => code.match(/.{1,4}/g)?.join(" ") ?? code;

export const inviteCode = (offered: Offer) => new URL(offered.invite).searchParams.get("code") ?? "";

export const isThisClient = (client: PairedClient) => client.key === myKey();

export const isThisAccount = (account: PairedAccount) => account.devices.some(isThisClient);

/** The items `is` holds for first, then the rest, each in the order given. */
export const firstWhere = <T>(items: T[], is: (item: T) => boolean) =>
  [...items].sort((a, b) => Number(is(b)) - Number(is(a)));

export const shortAccount = (id: string) => id.slice(0, 8);

export const accountLabel = (paired: PairedAccount) =>
  isThisAccount(paired) ? "this account" : `account ${shortAccount(paired.account)}`;

/** `self` is what this client calls itself. */
export function clientStatus(client: PairedClient, now: number, self = "this client") {
  if (isThisClient(client)) return self;
  if (client.hosts) return "runs this server";
  if (client.connected) return "connected";
  return client.seen_at === null ? "never connected" : `seen ${ago(client.seen_at, now)}`;
}

export function expiry(expiresAt: number, now: number, what: string) {
  const seconds = Math.max(0, Math.ceil((expiresAt - now) / 1000));
  return seconds === 0
    ? "expired"
    : `${what} · expires in ${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

/** A server's paired accounts, this client's first, listed while it is reachable. */
export function pairedAccounts(server: string) {
  const reachable = () => serverView(server)?.reachable === true;
  const [accounts, { refetch }] = createResource(() => reachable() && server, backend.serverAccounts);
  const listed = () => {
    const shown = reachable() && !accounts.error ? accounts() : undefined;
    return shown && firstWhere(shown, isThisAccount);
  };
  const note = () =>
    !reachable() ? "unknown while unreachable" : accounts.error ? String(accounts.error) : "listing…";
  return { listed, note, refetch };
}

/** Whether `text` is a link to join an account rather than an invite to pair a server. */
export const isLink = (text: string) => {
  try {
    return new URL(text.trim()).searchParams.has("join");
  } catch {
    return false;
  }
};
