import { Show } from "solid-js";
import type { TaskState } from "./generated";

export const GroupHead = (props: { state: TaskState; count?: number }) => (
  <div class="gh">
    {props.state}
    <Show when={props.count !== undefined}>
      <span class="count">{props.count}</span>
    </Show>
  </div>
);

export const dropHint = (state: TaskState) => `drop to mark ${state}`;
