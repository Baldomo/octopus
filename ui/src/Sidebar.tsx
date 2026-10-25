import { For, Show } from "solid-js";
import { dragSource, dropTarget, isDragged, showsDropHint } from "./drag";
import type { Column, Section } from "./generated";
import { Glyph } from "./Glyph";
import { dropHint, GroupHead } from "./Group";
import { commands, hint } from "./keys";
import {
  collapsed,
  isOpen,
  model,
  openTask,
  pinned,
  servers,
  setPinned,
  setSidebar,
  showBoard,
  sidebar,
  toggleCollapsed,
} from "./state";

function Group(props: { section: string; column: Column }) {
  const target = dropTarget(props.section, props.column.state);
  return (
    <Show
      when={props.column.rows.length > 0}
      fallback={
        <Show when={showsDropHint(props.section)}>
          <div class="drophint" classList={{ over: target.over() }} {...target.attributes}>
            {dropHint(props.column.state)}
          </div>
        </Show>
      }
    >
      <div class="group" classList={{ over: target.over() }} {...target.attributes}>
        <GroupHead column={props.column} />
        <For each={props.column.rows}>
          {(row) => (
            <div
              class="task"
              classList={{ active: isOpen(row.key), dragging: isDragged(row.key) }}
              style={{ "--depth": row.depth }}
              onClick={() => openTask(row.key)}
              {...dragSource(row, props.section, props.column.state)}
            >
              <Glyph glyph={row.glyph} />
              <span class="tt">{row.title}</span>
              <Show when={row.via}>{(via) => <span class="via">↳ {via()}</span>}</Show>
              <span class="srv">{row.at.server}</span>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}

function SectionGroups(props: { section: Section }) {
  return (
    <div class="ws" classList={{ closed: collapsed().includes(props.section.name) }}>
      <div class="wsh" onClick={() => toggleCollapsed(props.section.name)}>
        <span class="car">▼</span>
        <span
          class="name"
          onClick={(event) => {
            event.stopPropagation();
            showBoard(props.section.name);
          }}
        >
          {props.section.name}
        </span>
        <span class="count">{props.section.live} live</span>
      </div>
      <div class="wsb">
        <For each={props.section.columns.filter((column) => column.live)}>
          {(column) => <Group section={props.section.name} column={column} />}
        </For>
      </div>
    </div>
  );
}

export function Sidebar() {
  return (
    <aside
      class="sidebar"
      classList={{ hidden: !sidebar() }}
      onPointerLeave={() => {
        if (!pinned()) setSidebar(false);
      }}
    >
      <div class="sb-head">
        <div class="search" onClick={commands.search.run}>
          <span>{commands.search.label}</span>
          <kbd>{hint(commands.search)}</kbd>
        </div>
        <button class="icon" classList={{ on: pinned() }} title="pin" onClick={() => setPinned(!pinned())}>
          ⇤
        </button>
      </div>
      <div class="servers">
        <For each={servers()}>
          {(server) => (
            <span class="chip" title={server.problem ?? undefined}>
              <span class="dot" classList={{ off: !server.reachable }} />
              {server.name}
            </span>
          )}
        </For>
        <span class="chip add" onClick={commands.pairServer.run}>
          + server
        </span>
      </div>
      <div class="groups">
        <For each={model.lists.sections}>{(section) => <SectionGroups section={section} />}</For>
      </div>
      <div class="sb-foot">
        <span>{hint(commands.newTask)} new task</span>
        <span>{hint(commands.sidebar)} sidebar</span>
      </div>
    </aside>
  );
}
