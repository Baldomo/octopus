import { For, Show } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import { LIVE_STATES, type PtyId, TASK_STATES, type TaskView } from "@octopus/shared/generated.ts";
import { ticking } from "@octopus/shared/time.ts";
import { act, model, report, serverLabel, serverView } from "@octopus/shared/state.ts";
import { feel } from "./feel";
import { Icon } from "./Icon";
import { back, go } from "./nav";
import { Chev, Gap, Id, Mi, Para, Sep, ServerDot, Why } from "./parts";
import { RenameItem } from "./Rename";
import { Sheet, SheetHead } from "./Sheet";
import { reachOf } from "./reach";
import { notify } from "./Toast";

function Facts(props: { view: TaskView; panel: PtyId | null }) {
  const now = ticking();
  const panel = () => props.view.panels.find((each) => each.id === props.panel);
  const held = () => {
    const shown = panel();
    return shown?.running && shown.lock_holder !== null ? shown : null;
  };
  const reach = () => reachOf(props.view.at, panel(), now());
  const server = () => serverView(props.view.at.server);
  return (
    <div class="chips wrap">
      <span class="chip" classList={{ state: props.view.live }}>
        {props.view.task.state}
      </span>
      <span class="chip">
        <ServerDot server={server()} reconnecting={reach().is === "reconnecting"} />
        {serverLabel(props.view.at.server)}
        <Id>
          {
            { reachable: `${server()?.rtt_ms ?? "–"} ms`, reconnecting: "reconnecting", unreachable: "unreachable" }[
              reach().is
            ]
          }
        </Id>
      </span>
      <Show when={props.view.task.checkout.kind !== "root" && props.view.task.checkout}>
        {(branched) => (
          <span class="chip">
            <Icon name="branch" />
            <span class="mono">{branched().branch}</span>
          </span>
        )}
      </Show>
      <span class="chip">{props.view.task.harness}</span>
      <Show when={held()}>
        {(shown) => (
          <span class="chip">
            <span class="mono">
              {shown().lock_holder?.name} · {shown().cols}×{shown().rows}
            </span>
          </span>
        )}
      </Show>
    </div>
  );
}

export function TaskMenu(props: { taskKey: string; open: { panel: PtyId | null } | null }) {
  const view = () => model.tasks[props.taskKey];
  const copyId = (id: string) => navigator.clipboard.writeText(id).then(() => notify("Copied the task id"), report);
  return (
    <Sheet>
      <Show when={view()}>
        {(view) => (
          <>
            <SheetHead
              title={view().task.title}
              facts={[props.open ? view().task.id : `${view().task.id} · ${serverLabel(view().at.server)}`]}
            />
            <Show when={props.open}>
              {(open) => (
                <>
                  <Facts view={view()} panel={open().panel} />
                  <Mi slot={<Icon name="folder" />} end={<Chev />} tap={() => go({ at: "files", key: props.taskKey })}>
                    Files
                  </Mi>
                  <Mi slot={<Icon name="globe" />} end={<Chev />} tap={() => go({ at: "browser", key: props.taskKey })}>
                    Browser
                  </Mi>
                  <Sep />
                </>
              )}
            </Show>
            <RenameItem value={view().task.title} apply={(title) => backend.taskRename(view().at, title)} />
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
                  tap={() => {
                    if (view().task.state !== state) feel("select");
                    act(backend.taskMove(view().at, state));
                  }}
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
              tap={() => backend.taskArchive(view().at).then(() => feel("success"), report)}
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
                <Mi
                  slot={<Icon name="reload" />}
                  tap={() =>
                    backend.taskMove(view().at, state).then(() => {
                      feel("success");
                      back();
                    }, report)
                  }
                >
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
