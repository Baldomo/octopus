import { For, Show } from "solid-js";
import type { Column } from "@octopus/shared/generated.ts";
import { Glyph, statusWord } from "@octopus/shared/Glyph.tsx";
import { animateList } from "@octopus/shared/flip.ts";
import { isAway, isLanding, isLifted, isNesting, lift } from "@octopus/shared/liftTasks.ts";
import { type Within } from "@octopus/shared/lift.ts";
import { liftOnDrag } from "./lift";
import { Group } from "./Group";
import { isMenuOpen, MenuHost, menuTrigger, onRightClick } from "@octopus/shared/Menu.tsx";
import { backend } from "@octopus/shared/backend.ts";
import { isRenaming, RenameInput, startRename } from "@octopus/shared/SidebarControls.tsx";
import { isLive, TaskMenu } from "@octopus/shared/TaskMenus.tsx";
import { Toast } from "@octopus/shared/Toast.tsx";
import { model, openTask, serverLabel, ui } from "@octopus/shared/state.ts";

const ON_BOARD: Within = { stage: ".tasks-view", list: ".pane", free: true };

function BoardColumn(props: { section: string; column: Column }) {
  return (
    <Group class="col" section={props.section} column={props.column} counted>
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
              lifting: isLifted(row.key),
              away: isAway(row.key),
              landing: isLanding(row.key),
              nest: isNesting(row.key),
              "menu-open": isMenuOpen(id),
            }}
            data-key={row.key}
            style={{ "--depth": row.depth }}
            tabindex="0"
            onClick={() => openTask(row.key)}
            onKeyDown={(event) => event.key === "F2" && startRename("task", row.key)}
            onContextMenu={onRightClick(id, () => card, menu)}
            {...liftOnDrag(ON_BOARD, lift(row, props.section))}
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
    </Group>
  );
}

export function Board() {
  const section = () => model.lists.sections.find((candidate) => candidate.name === ui.prefs.board);
  return (
    <div class="pane" ref={animateList(".gh, .tcard, .drophint")}>
      <Show when={section()} fallback={<div class="empty">Pick a section in the sidebar to see its tasks.</div>}>
        {(section) => (
          <div class="board">
            <For each={section().columns}>{(column) => <BoardColumn section={section().name} column={column} />}</For>
          </div>
        )}
      </Show>
      <Show when={ui.prefs.view === "tasks"}>
        <MenuHost />
        <Toast />
      </Show>
    </div>
  );
}
