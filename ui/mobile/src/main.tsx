import { createEffect } from "solid-js";
import { render } from "solid-js/web";
import "virtual:design.css";
import "./app.css";
import { backend, events } from "@octopus/shared/backend.ts";
import { loadFonts } from "@octopus/shared/fonts.ts";
import type { Appearance } from "@octopus/shared/generated.ts";
import { Toast } from "@octopus/shared/SidebarControls.tsx";
import { applyBoot, applyModel, applyUi, report, setServers, ui } from "@octopus/shared/state.ts";
import { setWindowShown } from "@octopus/shared/terminalSession.ts";
import { ClientMenu } from "./ClientMenu";
import { setMicDenied } from "./screen";
import { Task, taskShown } from "./Task";
import { TaskSheet } from "./TaskSheet";
import { Tasks } from "./Tasks";
import { followViewport } from "./viewport";

const SCHEMES: Record<Appearance, string> = { system: "light dark", light: "light", dark: "dark" };

function App() {
  createEffect(() => document.documentElement.style.setProperty("color-scheme", SCHEMES[ui.prefs.appearance]));
  return (
    <>
      <div class="screen" style={{ display: taskShown() ? "none" : "" }}>
        <Tasks />
      </div>
      <Task />
      <TaskSheet />
      <ClientMenu />
      <Toast />
      <div class="toast problem" classList={{ on: ui.problem !== null }}>
        {ui.problem}
      </div>
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
  await loadFonts();
  await Promise.all([
    events.model(applyModel),
    events.servers(setServers),
    events.ui(applyUi),
    events.microphoneDenied(setMicDenied),
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
