import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { BrowserId, TaskRef } from "@octopus/shared/generated.ts";
import { NEW_BROWSER_PANEL } from "@octopus/shared/generated.ts";
import { EVENTS, type Go, type Navigated, type OnScreen } from "./generated";

/** The desktop app's own commands, beside the client app core's. */
export const desktop = {
  titleBarDoubleClick: () => invoke<void>("title_bar_double_click"),
  /** Resolves to the id of the panel that shows `url` on the task's server. */
  browserOpen: (at: TaskRef, panel: BrowserId | typeof NEW_BROWSER_PANEL, url: string) =>
    invoke<BrowserId>("browser_open", { at, panel, url }),
  /** Resolves to the port the page reaches through a tunnel, if it does. */
  browserLoad: (at: TaskRef, panel: BrowserId, url: string) =>
    invoke<number | null>("browser_load", { at, panel, url }),
  browserShowPage: (onScreen: OnScreen | null) => invoke<void>("browser_show_page", { onScreen }),
  browserGo: (at: TaskRef, panel: BrowserId, go: Go) => invoke<void>("browser_go", { at, panel, go }),
  /** Closes the panel's webview on this client; the panel stays on its server. */
  browserClose: (at: TaskRef, panel: BrowserId) => invoke<void>("browser_close", { at, panel }),
};

export const desktopEvents = {
  browserNavigated: (handle: (navigated: Navigated) => void) =>
    listen<Navigated>(EVENTS.browserNavigated, (event) => handle(event.payload)),
};
