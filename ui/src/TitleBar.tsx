import { For, Show } from "solid-js";
import { backend } from "./backend";
import { commands, hint, mac } from "./keys";
import { act, openView, serverView, ui } from "./state";

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
    if (ui.prefs.view === "tasks") return ui.prefs.board === null ? [] : [ui.prefs.board, "all tasks"];
    return openView()?.crumbs ?? [];
  };
  return (
    <header
      class="chrome"
      data-tauri-drag-region
      onDblClick={(event) => {
        if ((event.target as HTMLElement).hasAttribute("data-tauri-drag-region")) act(backend.titleBarDoubleClick());
      }}
    >
      <Show when={mac}>
        <div class="lights" data-tauri-drag-region />
      </Show>
      <Crumbs crumbs={crumbs()} />
      <div class="spacer" data-tauri-drag-region />
      <Show when={ui.prefs.view === "task" && openView()}>
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
        <button classList={{ on: ui.prefs.view === "task" }} onClick={commands.taskView.run}>
          Task<kbd>{hint(commands.taskView)}</kbd>
        </button>
        <button classList={{ on: ui.prefs.view === "tasks" }} onClick={commands.tasksView.run}>
          Tasks<kbd>{hint(commands.tasksView)}</kbd>
        </button>
      </div>
    </header>
  );
}
