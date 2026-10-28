import { createSignal, For, Show } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import { LIVE_STATES, TASK_STATES } from "@octopus/shared/generated.ts";
import { act, model, report, serverLabel } from "@octopus/shared/state.ts";
import { Icon } from "./Icon";
import { back, go } from "./nav";
import { Chev, Gap, Mi, Para, Sep, Why } from "./parts";
import { Rename } from "./Rename";
import { Sheet, SheetHead } from "./Sheet";
import { notify } from "./Toast";

/** A task's menu: the open task's leads with its Files and Browser, then rename, its state, its id and archive. */
export function TaskMenu(props: { taskKey: string; open: boolean }) {
  const view = () => model.tasks[props.taskKey];
  const [renaming, setRenaming] = createSignal(false);
  const copyId = (id: string) => navigator.clipboard.writeText(id).then(() => notify("Copied the task id"), report);
  return (
    <Sheet>
      <Show when={view()}>
        {(view) => (
          <>
            <SheetHead title={view().task.title} facts={[`${view().task.id} · ${serverLabel(view().at.server)}`]} />
            <Show when={props.open}>
              <Mi slot={<Icon name="folder" />} end={<Chev />} tap={() => go({ at: "files", key: props.taskKey })}>
                Files
              </Mi>
              <Mi slot={<Icon name="globe" />} end={<Chev />} tap={() => go({ at: "browser", key: props.taskKey })}>
                Browser
              </Mi>
              <Sep />
            </Show>
            <Show
              when={renaming()}
              fallback={
                <Mi slot={<Icon name="pen" />} tap={() => setRenaming(true)}>
                  Rename…
                </Mi>
              }
            >
              <Rename
                value={view().task.title}
                apply={(title) => backend.taskRename(view().at, title)}
                done={() => setRenaming(false)}
              />
            </Show>
            <Sep />
            <For each={TASK_STATES}>
              {(state) => (
                <Mi
                  slot={
                    <Show when={view().task.state === state}>
                      <span class="ck">
                        <Icon name="check" />
                      </span>
                    </Show>
                  }
                  tap={() => act(backend.taskMove(view().at, state))}
                >
                  {state}
                </Mi>
              )}
            </For>
            <Sep />
            <Mi slot={<Icon name="copy" />} tap={() => copyId(view().task.id)}>
              Copy task id
            </Mi>
            <Mi
              slot={<Icon name="archive" />}
              off={view().live}
              end={<Show when={view().live}>{<Why>end it first</Why>}</Show>}
              tap={() => act(backend.taskArchive(view().at))}
            >
              Archive
            </Mi>
            <Gap n={2} />
          </>
        )}
      </Show>
    </Sheet>
  );
}

/** Reopening an ended task moves it back to a live state, which ends its agent terminal. */
export function Reopen(props: { taskKey: string }) {
  const view = () => model.tasks[props.taskKey];
  return (
    <Sheet>
      <Show when={view()}>
        {(view) => (
          <>
            <SheetHead
              title={`Reopen ${view().task.title}`}
              facts={[`${view().task.id} · ${serverLabel(view().at.server)}`]}
            />
            <Para>Moving it back to a live state ends its agent terminal.</Para>
            <Gap n={2} />
            <For each={LIVE_STATES}>
              {(state) => (
                <Mi slot={<Icon name="reload" />} tap={() => backend.taskMove(view().at, state).then(back, report)}>
                  {state}
                </Mi>
              )}
            </For>
            <Gap n={2} />
          </>
        )}
      </Show>
    </Sheet>
  );
}
