import { invoke as invokeAny, type InvokeArgs } from "@tauri-apps/api/core";
import { on } from "@octopus/shared/backend.ts";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { BrowserId, Go, OnScreen, TaskRef } from "@octopus/shared/generated.ts";
import {
  COMMANDS,
  EVENTS,
  type Hovered,
  type MenuGroup,
  type MenuKey,
  type Outside,
  type SetupView,
  type ShowDrive,
  type ShowSpot,
} from "./generated";

const invoke = <T>(command: (typeof COMMANDS)[number], args?: InvokeArgs) => invokeAny<T>(command, args);

export const desktop = {
  titleBarDoubleClick: () => invoke<void>("title_bar_double_click"),
  browserLoad: (at: TaskRef, panel: BrowserId, url: string) => invoke<void>("browser_load", { at, panel, url }),
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
  /** The install of this machine's server this launch runs, if any. */
  setup: () => invoke<SetupView | null>("setup"),
  /** Installs this machine's server again after a failed install. */
  setupRetry: () => invoke<void>("setup_retry"),
  /** Tells the show where a target of its tour sits. */
  showSpot: (spot: ShowSpot) => invoke<void>("show_spot", { spot }),
  /** Whether the first open's show is yet to end on this launch. */
  showPlaying: () => invoke<boolean>("show_playing"),
  /** Writes the octopus skill for the user's Claude Code. */
  skillInstall: () => invoke<void>("skill_install"),
  /** Opens an `http` or `https` URL in the system's browser. */
  openOutside: openUrl,
};

export const desktopEvents = {
  browserHovered: on<Hovered | null>(EVENTS.browserHovered),
  menuCommand: on<MenuKey>(EVENTS.menuCommand),
  setup: on<SetupView | null>(EVENTS.setup),
  showDrive: on<ShowDrive>(EVENTS.showDrive),
  showEnd: on<null>(EVENTS.showEnd),
};
