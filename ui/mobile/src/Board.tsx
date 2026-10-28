import { createEffect, createSignal, For, Show } from "solid-js";
import type { Column, Row } from "@octopus/shared/generated.ts";
import { statusWord } from "@octopus/shared/Glyph.tsx";
import { model, serverLabel } from "@octopus/shared/state.ts";
import { keepInView } from "@octopus/shared/keepInView.ts";
import { swatchColor } from "@octopus/shared/tokens.ts";
import { dropTarget, isLifted, lift } from "./lift";
import { back, openSheet, openTask, screen } from "./nav";
import { Circle, Glyph, Gap, Id, Slot } from "./parts";
import { pressed } from "./press";

function Card(props: { row: Row; section: string; column: Column }) {
  const meta = () =>
    [serverLabel(props.row.at.server), statusWord[props.row.glyph]]
      .concat(props.row.state === props.column.state ? [] : [props.row.state])
      .join(" · ");
  return (
    <div
      class="card"
      classList={{
        ended: props.row.glyph === "completed" || props.row.glyph === "cancelled",
        lifting: isLifted(props.row.key),
      }}
      style={{ "--d": props.row.depth }}
      {...pressed(
        () => openTask(props.row.key, props.row.at),
        () => openSheet({ is: "task", key: props.row.key, open: false }),
        (element, from) => lift(props.row, props.section, element, from),
      )}
    >
      <Slot>
        <Glyph status={props.row.glyph} />
      </Slot>
      <div class="txt">
        <span class="name">{props.row.title}</span>
        <Id>{meta()}</Id>
      </div>
    </div>
  );
}

/** One section's four columns, one at a time under a scrolling bar of states and their counts. */
export function Board(props: { section: string }) {
  const section = () => model.lists.sections.find((shown) => shown.name === props.section);
  const [chosen, setChosen] = createSignal(0);
  const column = () => section()?.columns[chosen()];
  /** A section renamed or removed from its own board leaves nothing to show there. */
  createEffect(() => {
    const top = screen();
    if (section() === undefined && top.at === "board" && top.section === props.section) back();
  });
  return (
    <Show when={section()}>
      {(shown) => (
        <>
          <div class="bar">
            <Circle icon="back" label="Tasks" tap={back} />
            <div class="ttl" style={{ "--ws": swatchColor(shown().swatch) }}>
              <span class="crumb">
                <i />
                {shown().name}
              </span>
              <b class="name">All tasks</b>
            </div>
            <Circle icon="more" label="Section menu" tap={() => openSheet({ is: "section", name: shown().name })} />
          </div>
          <div class="tabs scroll">
            <For each={shown().columns}>
              {(each, nth) => {
                const target = dropTarget(shown().name, each.state);
                return (
                  <span
                    ref={keepInView(() => nth() === chosen())}
                    class="tab"
                    classList={{ on: nth() === chosen(), over: target.over() }}
                    role="button"
                    onClick={() => setChosen(nth())}
                    {...target.attributes}
                  >
                    {each.state}
                    <Id>{each.rows.length}</Id>
                  </span>
                );
              }}
            </For>
          </div>
          <Gap n={2} />
          <Show when={column()}>
            {(shownColumn) => (
              <div class="cards">
                <For each={shownColumn().rows} fallback={<div class="empty">no {shownColumn().state} tasks</div>}>
                  {(row) => <Card row={row} section={shown().name} column={shownColumn()} />}
                </For>
              </div>
            )}
          </Show>
        </>
      )}
    </Show>
  );
}
