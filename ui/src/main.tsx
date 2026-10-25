import inter from "@fontsource-variable/inter/files/inter-latin-wght-normal.woff2?url";
import mono from "@fontsource-variable/jetbrains-mono/files/jetbrains-mono-latin-wght-normal.woff2?url";
import { render } from "solid-js/web";
import "virtual:design.css";
import "./app.css";
import { App } from "./App";
import { backend, events } from "./backend";
import { applyFilesChanged } from "./taskFiles";
import { bindKeys } from "./keys";
import { applyBoot, applyModel, setServers } from "./state";

/** The family names DESIGN.html's `--ui` and `--mono` tokens ask for, served from the bundle. */
const fonts = [
  new FontFace("Inter", `url(${inter}) format("woff2")`, { weight: "100 900" }),
  new FontFace("JetBrains Mono", `url(${mono}) format("woff2")`, { weight: "100 800" }),
];

async function start() {
  await Promise.all(fonts.map(async (font) => document.fonts.add(await font.load())));
  bindKeys();
  await Promise.all([events.model(applyModel), events.servers(setServers), events.filesChanged(applyFilesChanged)]);
  applyBoot(await backend.boot());
  render(() => <App />, document.body);
}

start()
  .catch((error) => {
    document.body.textContent = `octopus could not start its window: ${error}`;
  })
  .finally(() => requestAnimationFrame(() => backend.ready().catch(console.error)));
