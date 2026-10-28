import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { Floated } from "@octopus/shared/generated.ts";
import { scale, ui, uiAct } from "@octopus/shared/state.ts";
import { FLOAT_LAYER, FLOAT_TOP } from "./generated";

/** Which layer of the window this webview renders: base beneath the pages, or float above them. */
export const layer = getCurrentWebview().label === FLOAT_LAYER ? "float" : "base";

export const floatSurface = (surface: string, at: Floated, modal: boolean) =>
  uiAct({ do: "float", surface, at, modal });
export const sinkSurface = (surface: string) => uiAct({ do: "sink", surface });

/** The title bar's height in this layer's CSS pixels: it keeps its height in points at every zoom. */
export const floatTop = () => FLOAT_TOP / scale(ui.prefs);
