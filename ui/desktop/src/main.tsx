import { render } from "solid-js/web";
import "virtual:design.css";
import "./app.css";
import { backend, events } from "@octopus/shared/backend.ts";
import { loadFonts } from "@octopus/shared/fonts.ts";
import { applyBoot, applyModel, applyUi, report, setServers } from "@octopus/shared/state.ts";
import { applyFilesChanged } from "@octopus/shared/taskFiles.ts";
import { setWindowShown } from "@octopus/shared/terminalSession.ts";
import { bindKeys } from "./keys";
import { layer } from "./layer";
import { Docked, Floating } from "./layers";

async function start() {
  document.documentElement.dataset.layer = layer;
  await loadFonts();
  bindKeys();
  await Promise.all([
    events.model(applyModel),
    events.servers(setServers),
    events.ui(applyUi),
    events.filesChanged(applyFilesChanged),
    events.windowShown(setWindowShown),
  ]);
  applyBoot(await backend.boot());
  render(() => (layer === "float" ? <Floating /> : <Docked />), document.body);
}

/** The float layer shows nothing until something floats, so its failure shows in the base layer's status bar. */
start()
  .catch((error) => {
    const problem = `octopus could not start its ${layer} layer: ${error}`;
    document.body.textContent = problem;
    report(problem);
  })
  .finally(() => requestAnimationFrame(() => backend.ready().catch(console.error)));
