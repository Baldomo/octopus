import { For, Show } from "solid-js";
import { backend } from "./backend";
import { commands, hint, mac } from "./keys";
import { act, board, openView, serverView, view } from "./state";

function Crumbs(props: { crumbs: string[] }) {
  return (
    <div class="crumbs">
      <For each={props.crumbs}>
        {(crumb, index) => (
          <Show when={index() < props.crumbs.length - 1} fallback={<b>{crumb}</b>}>
            <span>{crumb}</span>
            <span class="sep">/</span>
          </Show>
        )}
      </For>
    </div>
  );
}

export function TitleBar() {
  const crumbs = () => {
    if (view() === "tasks") return board() === null ? [] : [board()!, "all tasks"];
    return openView()?.crumbs ?? [];
  };
  return (
    <header class="chrome" data-tauri-drag-region>
      <Show when={mac}>
        <div class="lights" data-tauri-drag-region />
      </Show>
      <Crumbs crumbs={crumbs()} />
      <div class="spacer" data-tauri-drag-region />
      <Show when={view() === "task" && openView()}>
        {(open) => (
          <>
            <span class="server">
              <span class="dot" classList={{ off: !serverView(open().at.server)?.reachable }} />
              {open().at.server}
            </span>
            <span
              class="pill"
              classList={{ ended: !open().live }}
              title={`move to ${open().next}`}
              onClick={() => act(backend.taskMove(open().at, open().next))}
            >
              {open().task.state}
            </span>
          </>
        )}
      </Show>
      <div class="seg">
        <button classList={{ on: view() === "task" }} onClick={commands.taskView.run}>
          Task<kbd>{hint(commands.taskView)}</kbd>
        </button>
        <button classList={{ on: view() === "tasks" }} onClick={commands.tasksView.run}>
          Tasks<kbd>{hint(commands.tasksView)}</kbd>
        </button>
      </div>
    </header>
  );
}
