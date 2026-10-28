import { createSignal, For, Show } from "solid-js";
import { Gear } from "@octopus/shared/Gear.tsx";
import { Glyph } from "@octopus/shared/Glyph.tsx";
import type { Column, Row, Section } from "@octopus/shared/generated.ts";
import { model, servers } from "@octopus/shared/state.ts";
import { swatchColor } from "@octopus/shared/tokens.ts";
import { openMenu, openTask, openTaskSheet } from "./screen";

const LONG_PRESS_MS = 450;
/** A finger that moves further than this is scrolling, not pressing. */
const PRESS_SLOP_PX = 10;

const [collapsed, setCollapsed] = createSignal<ReadonlySet<string>>(new Set());
const toggle = (section: string) =>
  setCollapsed((shut) => {
    const next = new Set(shut);
    if (!next.delete(section)) next.add(section);
    return next;
  });

/** A tap runs `tap`; holding a finger still runs `hold`, and the tap that ends the hold does nothing more. */
function pressed(tap: () => void, hold: () => void) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let from: { x: number; y: number } | null = null;
  let held = false;
  const cancel = () => {
    clearTimeout(timer);
    from = null;
  };
  return {
    onTouchStart: (event: TouchEvent) => {
      const touch = event.touches[0];
      if (touch === undefined) return;
      held = false;
      from = { x: touch.clientX, y: touch.clientY };
      timer = setTimeout(() => {
        held = true;
        hold();
      }, LONG_PRESS_MS);
    },
    onClick: () => held || tap(),
    onTouchMove: (event: TouchEvent) => {
      const touch = event.touches[0];
      if (from === null || touch === undefined) return;
      if (Math.hypot(touch.clientX - from.x, touch.clientY - from.y) > PRESS_SLOP_PX) cancel();
    },
    onTouchEnd: cancel,
    onTouchCancel: cancel,
    onContextMenu: (event: MouseEvent) => event.preventDefault(),
  };
}

function TaskRow(props: { row: Row; group: Column }) {
  return (
    <div
      class="row"
      style={{ "--depth": props.row.depth }}
      {...pressed(
        () => openTask(props.row.key, props.row.at),
        () => openTaskSheet(props.row.key),
      )}
    >
      <Glyph glyph={props.row.glyph} />
      <span class="tt">{props.row.title}</span>
      <Show when={props.row.state !== props.group.state}>
        <span class="st">{props.row.state}</span>
      </Show>
      <span class="chev">›</span>
    </div>
  );
}

function SectionRows(props: { section: Section }) {
  const shut = () => collapsed().has(props.section.name);
  return (
    <div class="ws" classList={{ closed: shut() }} style={{ "--ws": swatchColor(props.section.swatch) }}>
      <button class="wsh" onClick={() => toggle(props.section.name)}>
        <span class="car">▾</span>
        <span class="name">{props.section.name}</span>
        <Show when={shut()}>
          <span class="count">
            <Show when={props.section.waiting}>
              <span class="wait" />
            </Show>
            {props.section.live}
          </span>
        </Show>
      </button>
      <Show when={!shut()}>
        <For each={props.section.groups.filter((group) => group.rows.length > 0)}>
          {(group) => (
            <div class="group">
              <div class="gh">{group.state}</div>
              <For each={group.rows}>{(row) => <TaskRow row={row} group={group} />}</For>
            </div>
          )}
        </For>
      </Show>
    </div>
  );
}

export function Tasks() {
  return (
    <main class="tasks">
      <header class="title">
        <h1>Tasks</h1>
        <button class="gear" aria-label="Client menu" onClick={() => openMenu({ show: "menu" })}>
          <Gear />
        </button>
      </header>
      <Show
        when={servers().length > 0}
        fallback={
          <button class="first-pair" onClick={() => openMenu({ show: "pair" })}>
            Pair a server
          </button>
        }
      >
        <div class="servers">
          <For each={servers()}>
            {(server) => (
              <button
                class="host"
                classList={{ off: !server.reachable }}
                onClick={() => openMenu({ show: "server", name: server.name })}
              >
                <span class="dot" classList={{ off: !server.reachable }} />
                <span class="name">{server.label}</span>
              </button>
            )}
          </For>
        </div>
        <div class="list">
          <For each={model.lists.sections}>{(section) => <SectionRows section={section} />}</For>
        </div>
      </Show>
    </main>
  );
}
