import type { TaskState } from "./generated";

export const dropHint = (state: TaskState) => `drop to mark ${state}`;
