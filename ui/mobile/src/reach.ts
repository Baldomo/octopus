import type { Panel, TaskRef } from "@octopus/shared/generated.ts";
import { holds, serverView } from "@octopus/shared/state.ts";

/** How long the server keeps a dropped holder's lock, which the phone counts down while it reconnects. */
const LOCK_KEPT_MS = 10_000;

/** What the phone knows of the task's server: reachable, reconnecting while the lock it held is kept, or gone. */
export type Reach = { is: "reachable" } | { is: "reconnecting"; left: number } | { is: "unreachable"; since: number };

export function reachOf(at: TaskRef, panel: Panel | undefined, now: number): Reach {
  const server = serverView(at.server);
  if (server === undefined || server.reachable) return { is: "reachable" };
  const left = LOCK_KEPT_MS - (now - server.since);
  if (panel !== undefined && holds(at.server, panel.lock_holder) && left > 0) return { is: "reconnecting", left };
  return { is: "unreachable", since: server.since };
}
