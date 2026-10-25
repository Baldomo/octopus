import inter from "@fontsource-variable/inter/files/inter-latin-wght-normal.woff2?url";
import mono from "@fontsource-variable/jetbrains-mono/files/jetbrains-mono-latin-wght-normal.woff2?url";
import terminalBold from "./fonts/MesloLGSNF-Bold.ttf?url";
import terminalBoldItalic from "./fonts/MesloLGSNF-BoldItalic.ttf?url";
import terminalItalic from "./fonts/MesloLGSNF-Italic.ttf?url";
import terminalRegular from "./fonts/MesloLGSNF-Regular.ttf?url";
import { render } from "solid-js/web";
import "virtual:design.css";
import "./app.css";
import { backend, events } from "./backend";
import { applyFilesChanged } from "./taskFiles";
import { bindKeys } from "./keys";
import { Docked, Floating } from "./layers";
import { applyBoot, applyModel, applyUi, layer, report, setServers } from "./state";

/** The family names DESIGN.html's `--ui`, `--mono` and `--mono-terminal` tokens ask for, served from the bundle. */
const fonts = [
  new FontFace("Inter", `url(${inter}) format("woff2")`, { weight: "100 900" }),
  new FontFace("JetBrains Mono", `url(${mono}) format("woff2")`, { weight: "100 800" }),
  new FontFace("MesloLGS NF", `url(${terminalRegular}) format("truetype")`),
  new FontFace("MesloLGS NF", `url(${terminalBold}) format("truetype")`, { weight: "bold" }),
  new FontFace("MesloLGS NF", `url(${terminalItalic}) format("truetype")`, { style: "italic" }),
  new FontFace("MesloLGS NF", `url(${terminalBoldItalic}) format("truetype")`, { weight: "bold", style: "italic" }),
];

async function start() {
  document.documentElement.dataset.layer = layer;
  await Promise.all(fonts.map(async (font) => document.fonts.add(await font.load())));
  bindKeys();
  await Promise.all([
    events.model(applyModel),
    events.servers(setServers),
    events.ui(applyUi),
    events.filesChanged(applyFilesChanged),
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
