import { For, Show } from "solid-js";
import { dragSource, dropTarget, isDragged, showsDropHint } from "./drag";
import type { Column } from "@octopus/shared/generated.ts";
import { Glyph, statusWord } from "@octopus/shared/Glyph.tsx";
import { dropHint } from "@octopus/shared/drop.ts";
import { GroupHead } from "./Group";
import { isMenuOpen, MenuHost, menuTrigger, onRightClick } from "@octopus/shared/Menu.tsx";
import { backend } from "@octopus/shared/backend.ts";
import { isLive, isRenaming, RenameInput, startRename, TaskMenu, Toast } from "@octopus/shared/SidebarControls.tsx";
import { model, openTask, serverLabel, ui } from "@octopus/shared/state.ts";

function BoardColumn(props: { section: string; column: Column }) {
  const target = dropTarget(props.section, props.column.state);
  return (
    <Show when={props.column.rows.length > 0 || showsDropHint(props.section)}>
      <div class="col" classList={{ over: target.over() }} {...target.attributes}>
        <GroupHead state={props.column.state} count={props.column.rows.length} />
        <Show when={props.column.rows.length === 0}>
          <div class="drophint">{dropHint(props.column.state)}</div>
        </Show>
        <For each={props.column.rows}>
          {(row) => {
            let card!: HTMLDivElement;
            const id = `task:${row.key}`;
            const menu = () => <TaskMenu row={row} />;
            return (
              <div
                ref={card}
                class="tcard"
                classList={{
                  sub: row.depth > 0,
                  done: !isLive(row.state),
                  dragging: isDragged(row.key),
                  "menu-open": isMenuOpen(id),
                }}
                style={{ "--depth": row.depth }}
                tabindex="0"
                onClick={() => openTask(row.key)}
                onKeyDown={(event) => event.key === "F2" && startRename("task", row.key)}
                onContextMenu={onRightClick(id, () => card, menu)}
                {...dragSource(row, props.section, props.column.state)}
              >
                <div class="tt">
                  <Glyph glyph={row.glyph} />
                  <Show when={isRenaming("task", row.key)} fallback={row.title}>
                    <RenameInput value={row.title} apply={(title) => backend.taskRename(row.at, title)} />
                  </Show>
                  <span class="acts">
                    <button class="mini more" title="task menu" {...menuTrigger(id, () => card, menu)}>
                      ⋯
                    </button>
                  </span>
                </div>
                <div class="meta">
                  <span>{serverLabel(row.at.server)}</span>
                  <span>{statusWord[row.glyph]}</span>
                  <Show when={row.state !== props.column.state && isLive(row.state)}>
                    <span>{row.state}</span>
                  </Show>
                </div>
              </div>
            );
          }}
        </For>
      </div>
    </Show>
  );
}

export function Board() {
  const section = () => model.lists.sections.find((candidate) => candidate.name === ui.prefs.board);
  const shown = () => ui.prefs.view === "tasks";
  return (
    <div class="pane">
      <Show when={section()} fallback={<div class="empty">Pick a section in the sidebar to see its tasks.</div>}>
        {(section) => (
          <div class="board">
            <For each={section().columns}>{(column) => <BoardColumn section={section().name} column={column} />}</For>
          </div>
        )}
      </Show>
      <Show when={shown()}>
        <MenuHost />
        <Toast />
      </Show>
    </div>
  );
}
