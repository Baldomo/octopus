import inter from "@fontsource-variable/inter/files/inter-latin-opsz-normal.woff2?url";
import { createEffect, For, Match, on, Switch } from "solid-js";
import { render } from "solid-js/web";
import "virtual:design.css";
import "./app.css";
import { backend, events } from "@octopus/shared/backend.ts";
import { loadFonts } from "@octopus/shared/fonts.ts";
import type { Appearance } from "@octopus/shared/generated.ts";
import { applyBoot, applyModel, applyUi, report, servers, setServers, ui } from "@octopus/shared/state.ts";
import { setWindowShown } from "@octopus/shared/terminalSession.ts";
import { applyFilesChanged, forgetFilesExcept } from "@octopus/shared/taskFiles.ts";
import { Board } from "./Board";
import { Browser } from "./Browser";
import { ClientMenu, ServerMenu } from "./ClientMenu";
import { EditorScreen, Files } from "./Files";
import { Sprite } from "./Icon";
import { loadSymbols } from "./symbols";
import { screen, screens, setMicDenied, sheet, type Screen } from "./nav";
import { NewTask } from "./NewTask";
import { PairSheet, Scan, scanning, Welcome } from "./Pairing";
import { Search } from "./Search";
import { Task } from "./Task";
import { Reopen, TaskMenu } from "./TaskMenu";
import { Tasks } from "./Tasks";
import { Toast } from "./Toast";
import { followViewport } from "./viewport";
import { AddWorkspace, SectionMenu } from "./Workspaces";

const SCHEMES: Record<Appearance, string> = { system: "light dark", light: "light", dark: "dark" };

function Shown(props: { screen: Screen }) {
  return (
    <Switch>
      <Match when={props.screen.at === "tasks"}>
        <Switch fallback={<Tasks />}>
          <Match when={servers().length === 0}>
            <Welcome />
          </Match>
        </Switch>
      </Match>
      <Match when={props.screen.at === "scan"}>
        <Scan />
      </Match>
      <Match when={props.screen.at === "task" && props.screen.key} keyed>
        {(key) => <Task taskKey={key} />}
      </Match>
      <Match when={props.screen.at === "board" && props.screen.section} keyed>
        {(section) => <Board section={section} />}
      </Match>
      <Match when={props.screen.at === "files" && props.screen.key} keyed>
        {(key) => <Files taskKey={key} />}
      </Match>
      <Match when={props.screen.at === "editor" && props.screen.key} keyed>
        {(key) => <EditorScreen taskKey={key} />}
      </Match>
      <Match when={props.screen.at === "browser" && props.screen.key} keyed>
        {(key) => <Browser taskKey={key} />}
      </Match>
      <Match when={props.screen.at === "search"}>
        <Search />
      </Match>
    </Switch>
  );
}

function SheetShown() {
  return (
    <Switch>
      <Match when={sheet()?.is === "client"}>
        <ClientMenu />
      </Match>
      <Match when={sheet()} keyed>
        {(shown) => (
          <Switch>
            <Match when={shown.is === "server" && shown.name} keyed>
              {(name) => <ServerMenu name={name} />}
            </Match>
            <Match when={shown.is === "pair" && shown.invite} keyed>
              {(invite) => <PairSheet invite={invite} />}
            </Match>
            <Match when={shown.is === "task" && shown} keyed>
              {(menu) => <TaskMenu taskKey={menu.key} open={menu.open} />}
            </Match>
            <Match when={shown.is === "reopen" && shown.key} keyed>
              {(key) => <Reopen taskKey={key} />}
            </Match>
            <Match when={shown.is === "section" && shown.name} keyed>
              {(name) => <SectionMenu name={name} />}
            </Match>
            <Match when={shown.is === "addws"}>
              <AddWorkspace />
            </Match>
            <Match when={shown.is === "newtask" && shown} keyed>
              {(form) => <NewTask section={form.section} />}
            </Match>
          </Switch>
        )}
      </Match>
    </Switch>
  );
}

/** Every pushed screen stays mounted under the one on top, so going back finds it as it was left. */
function App() {
  createEffect(() => document.documentElement.style.setProperty("color-scheme", SCHEMES[ui.prefs.appearance]));
  createEffect(() => document.documentElement.toggleAttribute("data-scanning", scanning()));
  createEffect(on(() => [...ui.recent], forgetFilesExcept));
  return (
    <>
      <Sprite />
      <For each={screens()}>
        {(shown) => (
          <div
            class="app"
            classList={{ cam: shown.at === "scan" }}
            style={{ display: shown === screen() ? "" : "none" }}
          >
            <div class="content">
              <Shown screen={shown} />
            </div>
          </div>
        )}
      </For>
      <SheetShown />
      <Toast />
    </>
  );
}

/**
 * The app is in front while its window is visible, overlays and its own permission prompts included. On iOS the
 * page's visibility follows the scene; Android hides the page for a prompt too, so the app's activity says instead.
 */
const followFront = () => {
  if (/Android/.test(navigator.userAgent)) {
    window.addEventListener("octopus-front", (event) => setWindowShown((event as CustomEvent<boolean>).detail));
    return;
  }
  const shown = () => setWindowShown(document.visibilityState === "visible");
  document.addEventListener("visibilitychange", shown);
  shown();
};

async function start() {
  followViewport();
  followFront();
  await Promise.all([loadFonts(inter), loadSymbols()]);
  await Promise.all([
    events.model(applyModel),
    events.servers(setServers),
    events.ui(applyUi),
    events.microphoneDenied(setMicDenied),
    events.filesChanged(applyFilesChanged),
  ]);
  applyBoot(await backend.boot());
  render(() => <App />, document.body);
}

start()
  .catch((error) => {
    const problem = `octopus could not start: ${error}`;
    document.body.textContent = problem;
    report(problem);
  })
  .finally(() => requestAnimationFrame(() => backend.ready().catch(console.error)));
