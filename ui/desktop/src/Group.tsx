import { For, Show, type JSX } from "solid-js";
import type { Column, Row } from "@octopus/shared/generated.ts";
import { dropHint } from "@octopus/shared/drop.ts";
import { dropTarget, liftsIn } from "@octopus/shared/liftTasks.ts";

/** A state's group of a section, which shows while empty only to take a task lifted in its section. */
export function Group(props: {
  class: string;
  section: string;
  column: Column;
  counted: boolean;
  children: (row: Row) => JSX.Element;
}) {
  const target = dropTarget(props.section, props.column.state);
  return (
    <Show when={props.column.rows.length > 0 || liftsIn(props.section)}>
      <div
        classList={{ [props.class]: true, over: target.over() }}
        data-state={props.column.state}
        {...target.attributes}
      >
        <div class="gh">
          {props.column.state}
          <Show when={props.counted}>
            <span class="count">{props.column.rows.length}</span>
          </Show>
        </div>
        <For
          each={props.column.rows}
          fallback={
            <div class="drophint" data-key={`${props.section}/${props.column.state}/empty`}>
              {dropHint(props.column.state)}
            </div>
          }
        >
          {props.children}
        </For>
      </div>
    </Show>
  );
}
