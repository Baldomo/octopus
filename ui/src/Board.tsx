import { For, Show } from "solid-js";
import { dragSource, dropTarget, isDragged, showsDropHint } from "./drag";
import type { Column } from "./generated";
import { Glyph, statusWord } from "./Glyph";
import { dropHint, GroupHead } from "./Group";
import { board, model, openTask } from "./state";

function BoardColumn(props: { section: string; column: Column }) {
  const target = dropTarget(props.section, props.column.state);
  return (
    <Show when={props.column.rows.length > 0 || showsDropHint(props.section)}>
      <div class="col" classList={{ over: target.over() }} {...target.attributes}>
        <GroupHead column={props.column} />
        <Show when={props.column.rows.length === 0}>
          <div class="drophint">{dropHint(props.column.state)}</div>
        </Show>
        <For each={props.column.rows}>
          {(row) => (
            <div
              class="tcard"
              classList={{ done: !props.column.live, dragging: isDragged(row.key) }}
              style={{ "--depth": row.depth }}
              onClick={() => openTask(row.key)}
              {...dragSource(row, props.section, props.column.state)}
            >
              <div class="tt">
                <Glyph glyph={row.glyph} />
                {row.title}
              </div>
              <div class="meta">
                <span>{row.at.server}</span>
                <span>{statusWord[row.glyph]}</span>
                <Show when={row.subtasks > 0}>
                  <span>{row.subtasks} sub</span>
                </Show>
                <Show when={row.via}>{(via) => <span>↳ {via()}</span>}</Show>
              </div>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}

export function Board() {
  const section = () => model.lists.sections.find((candidate) => candidate.name === board());
  return (
    <div class="pane">
      <Show when={section()} fallback={<div class="empty">Pick a section in the sidebar to see its tasks.</div>}>
        {(section) => (
          <div class="board">
            <For each={section().columns}>{(column) => <BoardColumn section={section().name} column={column} />}</For>
          </div>
        )}
      </Show>
    </div>
  );
}
