import "../../design/desktop/tokens.css";
import "../../design/desktop/components.css";
import "./app.css";
import { backend, events } from "@octopus/shared/backend.ts";
import inter from "@fontsource-variable/inter/files/inter-latin-wght-normal.woff2?url";
import { fileDragged } from "@octopus/shared/fileDrag.ts";
import { loadFonts } from "@octopus/shared/fonts.ts";
import { mount } from "@octopus/shared/mount.ts";
import { applyBoot, applyModel, applyUi, report, setAccount, setServers } from "@octopus/shared/state.ts";
import { applyFilesChanged } from "@octopus/shared/taskFiles.ts";
import { setWindowShown, windowFocused } from "@octopus/shared/terminalSession.ts";
import { followSetup } from "./FirstOpen";
import { bindKeys } from "./keys";
import { followShow } from "./tour";
import { layer } from "./layer";
import { Docked, Floating } from "./layers";

async function start() {
  document.documentElement.dataset.layer = layer;
  await loadFonts(inter);
  bindKeys();
  await Promise.all([
    events.model(applyModel),
    events.servers(setServers),
    events.account(setAccount),
    events.ui(applyUi),
    events.filesChanged(applyFilesChanged),
    events.windowShown(setWindowShown),
    events.windowFocused(windowFocused),
    layer === "base" && events.fileDrag(fileDragged),
    followSetup(),
    followShow(layer === "base"),
  ]);
  applyBoot(await backend.boot());
  mount(() => (layer === "float" ? <Floating /> : <Docked />), document.body);
}

/** The float layer shows nothing until something floats, so its failure shows in the base layer's status bar. */
start()
  .catch((error) => {
    const problem = `octopus could not start its ${layer} layer: ${error}`;
    document.body.textContent = problem;
    report(problem);
  })
  .finally(() => requestAnimationFrame(() => backend.ready().catch(console.error)));
