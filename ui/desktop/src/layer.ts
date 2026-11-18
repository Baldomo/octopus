import { getCurrentWebview } from "@tauri-apps/api/webview";
import { scale, ui } from "@octopus/shared/state.ts";
import { FLOAT_LAYER, TITLE_H } from "./generated";

/** Which layer of the window this webview renders: base beneath the pages, or float above them. */
export const layer = getCurrentWebview().label === FLOAT_LAYER ? "float" : "base";

/** The title bar's height in this layer's CSS pixels: it keeps its height in points at every zoom. */
export const titleHeight = () => TITLE_H / scale(ui.prefs);
