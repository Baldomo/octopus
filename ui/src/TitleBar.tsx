import { For, Show } from "solid-js";
import { backend } from "./backend";
import { commands, hint, mac } from "./keys";
import { act, me, openView, serverLabel, serverView, toggleClientMenu, ui } from "./state";

const Gear = () => (
  <svg
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.7"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
    <circle cx="12" cy="12" r="3" />
  </svg>
);

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
              {serverLabel(open().at.server)}
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
      <span class="divider" />
      <button
        class="gear"
        classList={{ "menu-open": ui.client_menu }}
        title={`${me()} · ${hint(commands.clientMenu)}`}
        aria-haspopup="menu"
        onClick={toggleClientMenu}
      >
        <Gear />
      </button>
    </header>
  );
}
