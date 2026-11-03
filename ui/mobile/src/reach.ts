import { HOLDER_GRACE_MS, type PanelView, type TaskRef } from "@octopus/shared/generated.ts";
import { serverView } from "@octopus/shared/state.ts";

/** What the phone knows of the task's server: reachable, reconnecting while the server keeps the lock it held, or gone. */
export type Reach = { is: "reachable" } | { is: "reconnecting"; left: number } | { is: "unreachable"; since: number };

export function reachOf(at: TaskRef, panel: PanelView | undefined, now: number): Reach {
  const server = serverView(at.server);
  if (server === undefined || server.reachable) return { is: "reachable" };
  const left = HOLDER_GRACE_MS - (now - server.since);
  if (panel !== undefined && panel.held_here && left > 0) return { is: "reconnecting", left };
  return { is: "unreachable", since: server.since };
}
