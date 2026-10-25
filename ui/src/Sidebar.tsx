import { createEffect, For, on, onCleanup, Show } from "solid-js";
import { dragSource, dropTarget, isDragged, showsDropHint } from "./drag";
import { FLOAT_TOP, type Column, type Section } from "./generated";
import { Glyph } from "./Glyph";
import { dropHint, GroupHead } from "./Group";
import { commands, hint } from "./keys";
import { isOpen, model, openTask, servers, showSidebar, showBoard, toggleCollapsed, togglePin, ui } from "./state";

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
    <div class="ws" classList={{ closed: ui.prefs.collapsed.includes(props.section.name) }}>
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

/**
 * The unpinned sidebar goes once the pointer leaves the place it settles in. Entering and leaving count only there:
 * the sidebar sliding in sweeps under a pointer resting at the window's edge, and a modal's scrim takes the pointer
 * without it leaving, so a pointer that was on the sidebar when a modal opened leaves it when the modal closes
 * somewhere off the sidebar.
 */
function autohide(aside: HTMLElement) {
  let hovered = false;
  let movedTo: PointerEvent | null = null;
  const track = (event: PointerEvent) => (movedTo = event);
  document.addEventListener("pointermove", track);
  onCleanup(() => document.removeEventListener("pointermove", track));
  const onAside = ({ clientX, clientY }: PointerEvent) => {
    const [x, y] = [clientX, clientY + FLOAT_TOP];
    const { offsetLeft: left, offsetTop: top, offsetWidth: width, offsetHeight: height } = aside;
    return x >= left && x < left + width && y >= top && y < top + height;
  };
  const leave = () => {
    hovered = false;
    if (!ui.prefs.pinned) showSidebar(false);
  };
  createEffect(
    on(
      () => ui.modal === null,
      (closed) => {
        if (closed && hovered && movedTo !== null && !onAside(movedTo)) leave();
        movedTo = null;
      },
      { defer: true },
    ),
  );
  aside.addEventListener("pointerenter", (event) => (hovered ||= onAside(event)));
  aside.addEventListener("pointerleave", () => hovered && ui.modal === null && leave());
}

export function Sidebar(props: { ref: (aside: HTMLElement) => void; shown: boolean }) {
  return (
    <aside
      ref={(aside) => {
        props.ref(aside);
        autohide(aside);
      }}
      class="sidebar"
      classList={{ hidden: !props.shown }}
    >
      <div class="sb-head">
        <div class="search" onClick={commands.search.run}>
          <span>{commands.search.label}</span>
          <kbd>{hint(commands.search)}</kbd>
        </div>
        <button class="icon" classList={{ on: ui.prefs.pinned }} title="pin" onClick={togglePin}>
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
