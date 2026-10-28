import type { TaskState } from "./generated";

/** What a group or column with no rows reads while a task of its section is dragged. */
export const dropHint = (state: TaskState) => `drop to mark ${state}`;
