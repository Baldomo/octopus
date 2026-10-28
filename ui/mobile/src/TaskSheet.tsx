import { createSignal, For, Show } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import { TASK_STATES } from "@octopus/shared/generated.ts";
import { toast } from "@octopus/shared/SidebarControls.tsx";
import { act, model, report, serverLabel } from "@octopus/shared/state.ts";
import { Rename } from "./Rename";
import { closeTaskSheet, sheetTask } from "./screen";
import { Sheet, SheetHead } from "./Sheet";

function TaskActions(props: { taskKey: string }) {
  const view = () => model.tasks[props.taskKey];
  const [renaming, setRenaming] = createSignal(false);
  const copyId = (id: string) => navigator.clipboard.writeText(id).then(() => toast("Copied the task id"), report);
  return (
    <Show when={view()}>
      {(view) => (
        <>
          <SheetHead
            title={
              <Show when={renaming()} fallback={view().task.title}>
                <Rename
                  value={view().task.title}
                  apply={(title) => backend.taskRename(view().at, title)}
                  done={() => setRenaming(false)}
                />
              </Show>
            }
            lines={[view().task.id, serverLabel(view().at.server)]}
          />
          <button class="si" onClick={() => setRenaming(true)}>
            Rename…
          </button>
          <div class="sep" />
          <For each={TASK_STATES}>
            {(state) => (
              <button class="si" onClick={() => act(backend.taskMove(view().at, state))}>
                <span class="ck">{view().task.state === state ? "✓" : ""}</span>
                {state}
              </button>
            )}
          </For>
          <div class="sep" />
          <button class="si" onClick={() => copyId(view().task.id)}>
            Copy task id
          </button>
        </>
      )}
    </Show>
  );
}

export function TaskSheet() {
  return (
    <Sheet shown={sheetTask() !== null} close={closeTaskSheet}>
      <Show when={sheetTask()} keyed>
        {(key) => <TaskActions taskKey={key} />}
      </Show>
    </Sheet>
  );
}
