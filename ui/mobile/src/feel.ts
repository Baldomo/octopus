import { phone } from "./phone";

/**
 * What a haptic says happened, which each OS plays as its own: a choice changed, a key sent, a held finger lifted or
 * dropped a task, a pull crossed where letting go acts, and an action's outcome.
 */
export type Feel = "select" | "tap" | "lift" | "drop" | "threshold" | "success" | "warning" | "error";

export const feel = (kind: Feel) => void phone.feel(kind).catch(console.error);
