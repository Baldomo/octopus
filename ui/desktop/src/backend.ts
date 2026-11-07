import { invoke as invokeAny, type InvokeArgs } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { BrowserId, TaskRef } from "@octopus/shared/generated.ts";
import { BACKGROUND_BROWSER_PANEL, NEW_BROWSER_PANEL } from "@octopus/shared/generated.ts";
import {
  COMMANDS,
  EVENTS,
  type Downloaded,
  type Failed,
  type Go,
  type Hovered,
  type MenuGroup,
  type MenuKey,
  type OnScreen,
  type Opened,
  type Outside,
  type Paged,
} from "./generated";

const invoke = <T>(command: (typeof COMMANDS)[number], args?: InvokeArgs) => invokeAny<T>(command, args);

export const desktop = {
  titleBarDoubleClick: () => invoke<void>("title_bar_double_click"),
  /** Resolves to the id of the panel that shows `url` on the task's server. */
  browserOpen: (
    at: TaskRef,
    panel: BrowserId | typeof NEW_BROWSER_PANEL | typeof BACKGROUND_BROWSER_PANEL,
    url: string,
  ) => invoke<BrowserId>("browser_open", { at, panel, url }),
  /** Removes the panel on the task's server, and so from every client. */
  browserClose: (at: TaskRef, panel: BrowserId) => invoke<void>("browser_close", { at, panel }),
  /** Moves the panel to index `to` among the task's panels on its server, and so on every client. */
  browserMove: (at: TaskRef, panel: BrowserId, to: number) => invoke<void>("browser_move", { at, panel, to }),
  /** Loads the panel's `url` on this client, unless its webview already loaded this `load`. */
  browserLoad: (at: TaskRef, panel: BrowserId, url: string, load: number) =>
    invoke<void>("browser_load", { at, panel, url, load }),
  browserShowPage: (onScreen: OnScreen | null) => invoke<void>("browser_show_page", { onScreen }),
  browserGo: (at: TaskRef, panel: BrowserId, go: Go) => invoke<void>("browser_go", { at, panel, go }),
  browserFind: (at: TaskRef, panel: BrowserId, text: string, backwards: boolean) =>
    invoke<void>("browser_find", { at, panel, text, backwards }),
  /** Closes the panel's webview on this client; the panel stays on its server. */
  browserForget: (at: TaskRef, panel: BrowserId) => invoke<void>("browser_forget", { at, panel }),
  /** Lists the commands in the macOS menu bar. */
  appMenu: (groups: MenuGroup[]) => invoke<void>("app_menu", { groups }),
  /** Reveals or opens an entry of a task whose server runs on this machine. */
  fileOutside: (at: TaskRef, path: string, how: Outside) => invoke<void>("file_outside", { at, path, how }),
  hapticAlignment: () => invoke<void>("haptic_alignment"),
  /** Opens an `http` or `https` URL in the system's browser. */
  openOutside: openUrl,
};

const on =
  <T>(event: string) =>
  (handle: (payload: T) => void) =>
    listen<T>(event, (heard) => handle(heard.payload));

export const desktopEvents = {
  browserPaged: on<Paged>(EVENTS.browserPaged),
  browserFailed: on<Failed>(EVENTS.browserFailed),
  browserOpened: on<Opened>(EVENTS.browserOpened),
  browserHovered: on<Hovered | null>(EVENTS.browserHovered),
  browserDownloaded: on<Downloaded>(EVENTS.browserDownloaded),
  menuCommand: on<MenuKey>(EVENTS.menuCommand),
};
