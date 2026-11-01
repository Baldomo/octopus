import { For, Show } from "solid-js";
import type { Column, Row, Section } from "@octopus/shared/generated.ts";
import { isOpen, model, report, servers, toggleCollapsed, ui } from "@octopus/shared/state.ts";
import { backend } from "@octopus/shared/backend.ts";
import { dropHint } from "@octopus/shared/drop.ts";
import { swatchColor } from "@octopus/shared/tokens.ts";
import { Icon } from "./Icon";
import { animateList } from "./flip";
import { dropTarget, isAway, isLanding, isLifted, lift, liftsIn } from "./lift";
import { go, openSheet, openTask } from "./nav";
import { Circle, Glyph, Id, ServerDot, Slot } from "./parts";
import { pressed } from "./press";
import { pullToRefresh } from "./pull";

const ITEMS = ".sech, .gl, .row, .empty, .addws";

export const refreshServers = () => backend.serversRefresh().catch(report);

function TaskRow(props: { row: Row; section: string; group: Column }) {
  return (
    <div
      class="row"
      data-key={props.row.key}
      classList={{
        open: isOpen(props.row.key),
        lifting: isLifted(props.row.key),
        away: isAway(props.row.key),
        landing: isLanding(props.row.key),
      }}
      style={{ "--d": props.row.depth }}
      {...pressed(
        () => openTask(props.row.key, props.row.at),
        () => openSheet({ is: "task", key: props.row.key, open: null }),
        (element, from) => lift(props.row, props.section, element, from),
      )}
    >
      <Slot>
        <Glyph status={props.row.glyph} />
      </Slot>
      <span class="name">{props.row.title}</span>
      <Show when={props.row.state !== props.group.state}>
        <Id>{props.row.state}</Id>
      </Show>
    </div>
  );
}

/** A state's group of a section, which shows while empty only to take a task lifted in its section. */
function Group(props: { section: string; group: Column }) {
  const target = dropTarget(props.section, props.group.state);
  return (
    <div class="grp" classList={{ over: target.over() }} data-state={props.group.state} {...target.attributes}>
      <div class="gl" data-key={`${props.section}/${props.group.state}`}>
        {props.group.state}
      </div>
      <For
        each={props.group.rows}
        fallback={
          <div class="empty" data-key={`${props.section}/${props.group.state}/empty`}>
            {dropHint(props.group.state)}
          </div>
        }
      >
        {(row) => <TaskRow row={row} section={props.section} group={props.group} />}
      </For>
    </div>
  );
}

function SectionRows(props: { section: Section }) {
  const closed = () => ui.prefs.collapsed.includes(props.section.name);
  const groups = () => props.section.groups.filter((group) => group.rows.length > 0 || liftsIn(props.section.name));
  const empty = () => !closed() && groups().length === 0;
  return (
    <div
      class="sec"
      classList={{ closed: closed(), void: empty() }}
      style={{ "--ws": swatchColor(props.section.swatch) }}
    >
      <div
        class="sech"
        data-key={`section/${props.section.name}`}
        {...pressed(
          () => {},
          () => openSheet({ is: "section", name: props.section.name }),
        )}
      >
        <span
          class="slot"
          role="button"
          onClick={(event) => {
            event.stopPropagation();
            toggleCollapsed(props.section.name);
          }}
        >
          <Icon name="car" class="car" />
        </span>
        <span class="name" role="button" onClick={() => go({ at: "board", section: props.section.name })}>
          {props.section.name}
        </span>
        <Show when={closed()}>
          <span class="count">
            <Show when={props.section.waiting}>
              <i class="dot needs" />
            </Show>
            <Id>{props.section.live}</Id>
          </span>
        </Show>
        <span
          class="slot new"
          role="button"
          aria-label={`New task in ${props.section.name}`}
          onClick={(event) => {
            event.stopPropagation();
            openSheet({ is: "newtask", section: props.section.name });
          }}
        >
          <Icon name="plus" />
        </span>
      </div>
      <Show when={!closed()}>
        <For each={groups()}>{(group) => <Group section={props.section.name} group={group} />}</For>
        <Show when={empty()}>
          <div class="empty" data-key={`${props.section.name}/none`}>
            no live tasks
          </div>
        </Show>
      </Show>
    </div>
  );
}

/** The root screen: every section's live tasks under their state, on one scroll. */
export function Tasks() {
  return (
    <>
      <div class="bar">
        <h1>Tasks</h1>
        <div class="right">
          <div class="capsule">
            <Circle icon="search" label="Search" tap={() => go({ at: "search" })} />
            <Circle icon="gear" label="Client menu" tap={() => openSheet({ is: "client" })} />
          </div>
        </div>
      </div>
      <div class="servers">
        <For each={servers()}>
          {(server) => (
            <span
              classList={{ off: !server.reachable }}
              role="button"
              onClick={() => openSheet({ is: "server", name: server.name })}
            >
              <ServerDot server={server} />
              <span class="name">{server.label}</span>
            </span>
          )}
        </For>
      </div>
      <div
        class="list"
        ref={(list) => {
          animateList(ITEMS)(list);
          pullToRefresh(refreshServers)(list);
        }}
      >
        <For each={model.lists.sections}>{(section) => <SectionRows section={section} />}</For>
        <div class="addws" role="button" onClick={() => openSheet({ is: "addws" })}>
          <Slot>
            <Icon name="plus" />
          </Slot>
          <span class="name">workspace</span>
        </div>
      </div>
    </>
  );
}
