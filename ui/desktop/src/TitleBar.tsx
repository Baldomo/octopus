import { For, Show } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import { commands, hint, mac } from "@octopus/shared/commands.ts";
import { act, me, openView, serverLabel, serverView, toggleClientMenu, ui } from "@octopus/shared/state.ts";
import { Gear } from "@octopus/shared/Gear.tsx";
import { desktop } from "./backend";

function Crumbs(props: { crumbs: string[] }) {
  return (
    <div class="crumbs">
      <For each={props.crumbs}>
        {(crumb, index) => (
          <Show when={index() < props.crumbs.length - 1} fallback={<b class="name">{crumb}</b>}>
            <span class="name">{crumb}</span>
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
        if ((event.target as HTMLElement).hasAttribute("data-tauri-drag-region")) act(desktop.titleBarDoubleClick());
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
              <span class="name">{serverLabel(open().at.server)}</span>
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
