/** Where the first open stands; each beat moves on only on its event, never on its motion. */
export type Beat = "off" | "wake" | "reach" | "offer" | "hand_off" | "done";

/**
 * What the beats wait on: whether this launch installs this machine's server, whether the show still covers the
 * screens, and what the window holds now.
 */
export type Facts = {
  installs: boolean;
  showing: boolean;
  answered: boolean;
  sidebar: boolean;
  workspaces: number;
  tasks: number;
};

/**
 * The beat after `was`: a launch that installs nothing has no first open; no beat moves while a show covers the
 * screens, as its tour works the window; once the server answers, the first open goes on only while no server holds a
 * workspace; the reach ends with the sidebar out, the offer with the first workspace, and the hand-off with the first
 * task.
 */
export function nextBeat(was: Beat, facts: Facts): Beat {
  if (was === "wake" && !facts.installs) return "off";
  if (facts.showing) return was;
  if (was === "wake" && facts.answered) return nextBeat(facts.workspaces > 0 ? "done" : "reach", facts);
  if (was === "reach" && facts.sidebar) return nextBeat("offer", facts);
  if (was === "offer" && facts.workspaces > 0) return nextBeat("hand_off", facts);
  if (was === "hand_off" && facts.tasks > 0) return "done";
  return was;
}
