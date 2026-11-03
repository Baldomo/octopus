import { createEffect, createSignal, For, Show } from "solid-js";
import type { Column, Row } from "@octopus/shared/generated.ts";
import { statusWord } from "@octopus/shared/Glyph.tsx";
import { model, serverLabel } from "@octopus/shared/state.ts";
import { keepInView } from "@octopus/shared/keepInView.ts";
import { swatchColor } from "@octopus/shared/tokens.ts";
import { feel } from "./feel";
import { animateList } from "@octopus/shared/flip.ts";
import { dropTarget, isAway, isLanding, isLifted, isNesting, lift } from "@octopus/shared/liftTasks.ts";
import { animate, FADE, give, play, PROJECT_S, reduced, type Running } from "@octopus/shared/motion.ts";
import { PUSH } from "./motion";
import { back, openSheet, openTask, screen } from "./nav";
import { Circle, Glyph, Gap, Id, Slot } from "./parts";
import { pressed } from "./press";
import { pullToRefresh } from "./pull";
import { liftOnPhone, refreshServers } from "./Tasks";
import { slidingThumb } from "./thumb";
import { owns } from "./touch";

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
        away: isAway(props.row.key),
        landing: isLanding(props.row.key),
        nest: isNesting(props.row.key),
      }}
      data-key={props.row.key}
      style={{ "--d": props.row.depth }}
      {...pressed(
        () => openTask(props.row.key, props.row.at),
        () => openSheet({ is: "task", key: props.row.key, open: null }),
        liftOnPhone(lift(props.row, props.section)),
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

/**
 * One section's four columns side by side, one in view under a scrolling bar of states and their counts. Choosing a
 * state slides its column in from the side of its tab, and a sideways swipe carries the columns with the finger.
 */
export function Board(props: { section: string }) {
  const section = () => model.lists.sections.find((shown) => shown.name === props.section);
  const [chosen, setChosen] = createSignal(0);
  let track!: HTMLDivElement;
  /** Which column stands in view, fractional while the columns move. */
  let page = 0;
  let running: Running | null = null;
  const pose = (at: number) => {
    page = at;
    track.style.translate = `${-at * 100}% 0`;
  };
  const choose = (nth: number, velocity = 0) => {
    if (nth !== chosen()) feel("select");
    setChosen(nth);
    running?.stop();
    if (!reduced()) return void (running = animate(page, nth, pose, PUSH, velocity));
    pose(nth);
    play(track, [{ opacity: 0 }, { opacity: 1 }], FADE);
  };
  /**
   * A sideways touch carries the columns from where it committed, giving way past the first and the `last`, and
   * letting go settles on the column the throw reaches, at most one away.
   */
  const swipe = (columns: HTMLElement, last: () => number) =>
    owns(columns, ({ axis, at: grabbed }) => {
      if (axis !== "x") return null;
      running?.stop();
      const from = page;
      const width = columns.getBoundingClientRect().width;
      return {
        move: (to) => {
          const at = from - (to.x - grabbed.x) / width;
          const end = Math.min(Math.max(at, 0), last());
          pose(end - give((end - at) * width) / width);
        },
        end: (thrown) => {
          const velocity = -(thrown?.x ?? 0) / width;
          const start = Math.round(from);
          const aimed = Math.round(page + velocity * PROJECT_S);
          choose(Math.min(Math.max(aimed, start - 1, 0), start + 1, last()), velocity);
        },
      };
    });

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
          <div class="tabs scroll" ref={slidingThumb}>
            <For each={shown().columns}>
              {(each, nth) => {
                const target = dropTarget(shown().name, each.state);
                return (
                  <span
                    ref={keepInView(() => nth() === chosen())}
                    class="tab"
                    classList={{ on: nth() === chosen(), over: target.over() }}
                    role="button"
                    onClick={() => nth() !== chosen() && choose(nth())}
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
          <div class="columns" ref={(columns) => swipe(columns, () => shown().columns.length - 1)}>
            <div class="track" ref={track}>
              <For each={shown().columns}>
                {(column) => (
                  <div class="column" data-state={column.state}>
                    <div
                      class="cards"
                      ref={(list) => {
                        animateList(".card, .empty")(list);
                        pullToRefresh(refreshServers)(list);
                      }}
                    >
                      <For each={column.rows} fallback={<div class="empty">no {column.state} tasks</div>}>
                        {(row) => <Card row={row} section={shown().name} column={column} />}
                      </For>
                    </div>
                  </div>
                )}
              </For>
            </div>
          </div>
        </>
      )}
    </Show>
  );
}
