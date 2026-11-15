import { For, Show } from "solid-js";
import { animateList } from "./flip";
import { Glyph } from "./Glyph";
import { keepInView } from "./keepInView";
import type { LiftOnDrag, Within } from "./lift";
import { createArrangement, liftTab } from "./liftTab";

export type FolderTab = {
  key: string;
  name: string;
  where: string | undefined;
  title: string;
  loading: boolean;
  dirty: boolean;
};

type AddTab = { title: string; selected: boolean; run: () => void };

const IN_TABS: Within = { stage: ".ftabs", list: ".etabs", free: false };

export function FolderTabs(props: {
  tabs: FolderTab[];
  selected: string | null;
  visible: boolean;
  drag: LiftOnDrag;
  choose: (key: string) => void;
  close: (key: string) => void;
  move: (key: string, to: number) => Promise<unknown>;
  add?: AddTab;
}) {
  let bar!: HTMLDivElement;
  const keys = () => props.tabs.map((tab) => tab.key);
  const { shown, arrange } = createArrangement(keys);
  const scrollSideways = (event: WheelEvent) => {
    if (Math.abs(event.deltaY) <= Math.abs(event.deltaX)) return;
    bar.scrollLeft += event.deltaY;
    event.preventDefault();
  };
  return (
    <div class="ftabs">
      <div
        class="etabs"
        ref={(element) => {
          bar = element;
          animateList(".etab", { enterFrom: "below" })(element);
        }}
        onWheel={scrollSideways}
      >
        <For each={shown()}>
          {(key) => (
            <Show when={props.tabs.find((tab) => tab.key === key)}>
              {(tab) => (
                <span
                  ref={keepInView(() => props.visible && props.selected === key)}
                  class="etab"
                  classList={{ on: props.selected === key, mod: tab().dirty, loading: tab().loading }}
                  data-key={key}
                  title={tab().title}
                  onClick={() => props.choose(key)}
                  onAuxClick={(event) => event.button === 1 && props.close(key)}
                  {...props.drag(
                    IN_TABS,
                    liftTab(key, keys, arrange, (to) => props.move(key, to)),
                  )}
                >
                  <span class="name">{tab().name}</span>
                  <Show when={tab().where}>{(where) => <span class="where">{where()}</span>}</Show>
                  <span
                    class="close"
                    onClick={(event) => {
                      event.stopPropagation();
                      props.close(key);
                    }}
                  >
                    <Show when={tab().loading}>
                      <Glyph glyph="working" />
                    </Show>
                  </span>
                </span>
              )}
            </Show>
          )}
        </For>
        <Show when={props.add}>
          {(add) => (
            <span
              ref={keepInView(() => props.visible && add().selected)}
              class="etab plus"
              classList={{ on: add().selected }}
              title={add().title}
              onClick={() => add().run()}
            >
              +
            </span>
          )}
        </Show>
      </div>
    </div>
  );
}
