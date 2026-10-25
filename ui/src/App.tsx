import { createEffect, createMemo, Match, on, Show, Switch } from "solid-js";
import { AddServer } from "./AddServer";
import { backend } from "./backend";
import { Board } from "./Board";
import { Browser, forgetBrowsersExcept } from "./Browser";
import { Files } from "./Files";
import { forgetFilesExcept } from "./taskFiles";
import { commands, hint } from "./keys";
import { NewTask } from "./NewTask";
import { Palette } from "./Palette";
import { Sidebar } from "./Sidebar";
import {
  act,
  open,
  openView,
  overlay,
  persistPrefs,
  recent,
  serverView,
  setOverlay,
  setSidebar,
  surface,
  view,
} from "./state";
import { StatusBar } from "./StatusBar";
import { TerminalStacks } from "./Terminals";
import { TitleBar } from "./TitleBar";

export function App() {
  persistPrefs();
  createEffect(
    on(recent, (keys) => {
      forgetFilesExcept(keys);
      forgetBrowsersExcept(keys);
    }),
  );
  /** The open task is focused whenever it is in the Task view with its server reachable, so a restart refocuses it. */
  const shownInTaskView = createMemo(() => {
    const shown = view() === "task" ? openView() : undefined;
    return shown !== undefined && serverView(shown.at.server)?.reachable ? open() : null;
  });
  createEffect(
    on(shownInTaskView, (key) => {
      const shown = key === null ? undefined : openView();
      if (shown !== undefined) act(backend.taskFocus(shown.at));
    }),
  );

  return (
    <div class="app">
      <TitleBar />
      <div class="halves" style={{ display: view() === "task" ? "" : "none" }}>
        <div class="pane">
          <div class="tabs">
            <button class="tab" classList={{ on: surface() === "files" }} onClick={commands.files.run}>
              Files<kbd>{hint(commands.files)}</kbd>
            </button>
            <button class="tab" classList={{ on: surface() === "browser" }} onClick={commands.browser.run}>
              Browser<kbd>{hint(commands.browser)}</kbd>
            </button>
          </div>
          <Show when={openView()} fallback={<div class="empty">No task is open.</div>}>
            <div class="surface" style={{ display: surface() === "files" ? "" : "none" }}>
              <Files />
            </div>
            <div class="surface" style={{ display: surface() === "browser" ? "" : "none" }}>
              <Browser visible={view() === "task" && surface() === "browser"} />
            </div>
          </Show>
        </div>
        <TerminalStacks />
      </div>
      <div class="tasks-view" style={{ display: view() === "tasks" ? "" : "none" }}>
        <Board />
      </div>
      <StatusBar />
      <div class="edge" onPointerEnter={() => setSidebar(true)} />
      <Sidebar />
      <Show when={overlay() !== null}>
        <div class="scrim" onClick={() => setOverlay(null)} />
      </Show>
      <Switch>
        <Match when={overlay() === "palette"}>
          <Palette />
        </Match>
        <Match when={overlay() === "new-task"}>
          <NewTask />
        </Match>
        <Match when={overlay() === "add-server"}>
          <AddServer />
        </Match>
      </Switch>
    </div>
  );
}
