import { Show } from "solid-js";
import type { TaskState } from "@octopus/shared/generated.ts";

export const GroupHead = (props: { state: TaskState; count?: number }) => (
  <div class="gh">
    {props.state}
    <Show when={props.count !== undefined}>
      <span class="count">{props.count}</span>
    </Show>
  </div>
);
