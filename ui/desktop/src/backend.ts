import { invoke as invokeAny, type InvokeArgs } from "@tauri-apps/api/core";
import { on } from "@octopus/shared/backend.ts";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { BrowserId, TaskRef } from "@octopus/shared/generated.ts";
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

export const desktopEvents = {
  browserPaged: on<Paged>(EVENTS.browserPaged),
  browserFailed: on<Failed>(EVENTS.browserFailed),
  browserOpened: on<Opened>(EVENTS.browserOpened),
  browserHovered: on<Hovered | null>(EVENTS.browserHovered),
  browserDownloaded: on<Downloaded>(EVENTS.browserDownloaded),
  menuCommand: on<MenuKey>(EVENTS.menuCommand),
};
