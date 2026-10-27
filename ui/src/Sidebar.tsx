import { createEffect, For, on, onCleanup, Show } from "solid-js";
import { backend } from "./backend";
import { dragSource, dropTarget, isDragged, showsDropHint } from "./drag";
import { FLOAT_TOP, type Column, type Section } from "./generated";
import { Glyph } from "./Glyph";
import { dropHint, GroupHead } from "./Group";
import { swatchColor } from "./tokens";
import { commands, hint } from "./keys";
import { isMenuOpen, MenuHost, menuTrigger, onRightClick } from "./Menu";
import {
  AddWorkspace,
  holdsKeyboard,
  isRenaming,
  newTaskIn,
  RenameInput,
  SectionMenu,
  ServerMenu,
  startRename,
  TaskMenu,
  Toast,
} from "./SidebarControls";
import {
  isOpen,
  model,
  openTask,
  servers,
  serverLabel,
  serverView,
  showSidebar,
  showBoard,
  toggleCollapsed,
  togglePin,
  ui,
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
        <GroupHead state={props.column.state} />
        <For each={props.column.rows}>
          {(row) => {
            let element!: HTMLDivElement;
            const id = `task:${row.key}`;
            const menu = () => <TaskMenu row={row} />;
            return (
              <div
                ref={element}
                class="task"
                classList={{
                  sub: row.depth > 0,
                  active: isOpen(row.key),
                  dragging: isDragged(row.key),
                  "menu-open": isMenuOpen(id),
                }}
                style={{ "--depth": row.depth }}
                tabindex="0"
                onClick={() => openTask(row.key)}
                onContextMenu={onRightClick(id, () => element, menu)}
                onKeyDown={(event) => event.key === "F2" && startRename("task", row.key)}
                {...dragSource(row, props.section, props.column.state)}
              >
                <Glyph glyph={row.glyph} />
                <Show
                  when={isRenaming("task", row.key)}
                  fallback={
                    <span class="tt" onDblClick={() => startRename("task", row.key)}>
                      {row.title}
                    </span>
                  }
                >
                  <RenameInput value={row.title} apply={(title) => backend.taskRename(row.at, title)} />
                </Show>
                <Show when={row.state !== props.column.state}>
                  <span class="st">{row.state}</span>
                </Show>
                <span class="acts">
                  <button class="mini more" title="task menu" {...menuTrigger(id, () => element, menu)}>
                    ⋯
                  </button>
                </span>
              </div>
            );
          }}
        </For>
      </div>
    </Show>
  );
}

function SectionGroups(props: { section: Section }) {
  let head!: HTMLDivElement;
  const name = () => props.section.name;
  const id = () => `section:${name()}`;
  const menu = () => <SectionMenu name={name()} />;
  const collapsed = () => ui.prefs.collapsed.includes(name());
  return (
    <div class="ws" classList={{ closed: collapsed() }} style={{ "--ws": swatchColor(props.section.swatch) }}>
      <div
        ref={head}
        class="wsh"
        classList={{ "menu-open": isMenuOpen(id()) }}
        onClick={() => toggleCollapsed(name())}
        onContextMenu={(event) => onRightClick(id(), () => head, menu)(event)}
      >
        <span class="car">▼</span>
        <Show
          when={isRenaming("section", name())}
          fallback={
            <span
              class="name"
              onClick={(event) => {
                event.stopPropagation();
                showBoard(name());
              }}
              onDblClick={(event) => {
                event.stopPropagation();
                startRename("section", name());
              }}
            >
              {name()}
            </span>
          }
        >
          <RenameInput value={name()} apply={(renamed) => backend.sectionRename(name(), renamed)} />
        </Show>
        <span class="count">
          <Show when={props.section.waiting}>
            <span class="wait" title="waiting on you" />
          </Show>
          {props.section.live}
        </span>
        <span class="acts">
          <button
            class="mini"
            title="new task here"
            onClick={(event) => {
              event.stopPropagation();
              newTaskIn(name());
            }}
          >
            +
          </button>
          <button class="mini more" title="section menu" {...menuTrigger(id(), () => head, menu)}>
            ⋯
          </button>
        </span>
      </div>
      <div class="wsb">
        <For each={props.section.groups}>{(column) => <Group section={name()} column={column} />}</For>
        <Show when={props.section.live === 0}>
          <div class="empty-state">No live tasks.</div>
        </Show>
      </div>
    </div>
  );
}

/** Keyed by name, as every round trip refreshes each server's view. */
function ServerName(props: { name: string }) {
  let host!: HTMLSpanElement;
  const server = () => serverView(props.name);
  const id = `server:${props.name}`;
  const menu = () => <ServerMenu name={props.name} />;
  return (
    <span
      ref={host}
      class="host"
      classList={{ off: !server()?.reachable, "menu-open": isMenuOpen(id) }}
      title={server()?.problem ?? undefined}
      {...menuTrigger(id, () => host, menu)}
      onContextMenu={onRightClick(id, () => host, menu)}
    >
      <span class="dot" classList={{ off: !server()?.reachable }} />
      <Show when={isRenaming("server", props.name)} fallback={serverLabel(props.name)}>
        <RenameInput value={props.name} apply={(name) => backend.serverRename(props.name, name)} />
      </Show>
    </span>
  );
}

/**
 * The unpinned sidebar goes once the pointer leaves its column, which runs from the window's left edge to the
 * sidebar's right edge, so a pointer resting on the edge that revealed it keeps it open. A modal's scrim takes the
 * pointer without it leaving, so a pointer that was in the column when a modal opened leaves it when the modal closes
 * somewhere off the column. The sidebar stays while it holds the keyboard, and goes when it lets go if the pointer
 * left meanwhile. Hiding forgets the pointer, as a column that stops taking it under a still pointer never sees it
 * leave.
 */
function autohide(column: HTMLElement) {
  let hovered = false;
  let movedTo: PointerEvent | null = null;
  const track = (event: PointerEvent) => (movedTo = event);
  document.addEventListener("pointermove", track);
  onCleanup(() => document.removeEventListener("pointermove", track));
  const inColumn = ({ clientX, clientY }: PointerEvent) => {
    const [x, y] = [clientX, clientY + FLOAT_TOP];
    const { offsetLeft: left, offsetTop: top, offsetWidth: width, offsetHeight: height } = column;
    return x >= left && x < left + width && y >= top && y < top + height;
  };
  const leave = () => {
    hovered = false;
    if (!ui.prefs.pinned) showSidebar(false);
  };
  const settled = () => ui.modal === null && !holdsKeyboard();
  createEffect(() => {
    if (!ui.sidebar) hovered = false;
  });
  createEffect(
    on(
      settled,
      (free) => {
        if (free && hovered && movedTo !== null && !inColumn(movedTo)) leave();
        movedTo = null;
      },
      { defer: true },
    ),
  );
  column.addEventListener("pointerenter", () => (hovered = true));
  column.addEventListener("pointerleave", () => hovered && settled() && leave());
}

/** The sidebar takes the pointer in its whole column and draws in its own box and shadow. */
export function Sidebar(props: { floats: (aside: HTMLElement, column: HTMLElement) => void; shown: boolean }) {
  let column!: HTMLDivElement;
  return (
    <div
      ref={(element) => {
        column = element;
        autohide(element);
      }}
      class="sidebar-column"
      classList={{ hidden: !props.shown }}
    >
      <aside ref={(aside) => props.floats(aside, column)} class="sidebar" classList={{ hidden: !props.shown }}>
        <div class="sb-head">
          <div class="search" onClick={commands.search.run}>
            <span>Search</span>
            <kbd>{hint(commands.search)}</kbd>
          </div>
          <button class="icon" classList={{ on: ui.prefs.pinned }} title="pin" onClick={togglePin}>
            ⇤
          </button>
        </div>
        <div class="servers">
          <For each={servers().map((server) => server.name)}>{(name) => <ServerName name={name} />}</For>
          <button class="host add" title={commands.pairServer.label} onClick={commands.pairServer.run}>
            +
          </button>
        </div>
        <div class="groups">
          <For each={model.lists.sections}>{(section) => <SectionGroups section={section} />}</For>
          <AddWorkspace />
        </div>
        <MenuHost />
        <Toast />
      </aside>
    </div>
  );
}
