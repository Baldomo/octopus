import type { Column, TaskState } from "./generated";

export const GroupHead = (props: { column: Column }) => (
  <div class="gh">
    {props.column.state}
    <span class="count">{props.column.rows.length}</span>
  </div>
);

export const dropHint = (state: TaskState) => `drop to mark ${state}`;
