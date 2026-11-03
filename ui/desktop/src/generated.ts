// Rendered from the Rust types by `cargo run -p octopus-desktop --example types`. Edit those, not this.
import type { BrowserId, Bounds, TaskRef } from "@octopus/shared/generated.ts";

export type OnScreen = { at: TaskRef, panel: BrowserId, bounds: Bounds, };

export type Go = "back" | "forward" | "reload" | "reload_from_origin";

export type Paged = { 
/**
 * The task's key.
 */
task: string, panel: BrowserId, url: string, loading: boolean, 
/**
 * Whether the panel's history goes back, and forward.
 */
back: boolean, forward: boolean, 
/**
 * The port of the server's machine the page reaches through a tunnel, when it does.
 */
tunnel: number | null, };

export type Failed = { 
/**
 * The task's key.
 */
task: string, panel: BrowserId, url: string, error: string, };

export type Opened = { 
/**
 * The task's key.
 */
task: string, 
/**
 * The panel whose page opened it.
 */
panel: BrowserId, url: string, 
/**
 * Whether the new tab takes the selection, else it opens behind the selected one.
 */
select: boolean, };

export type Hovered = { link: string, page: Bounds, };

export type Downloaded = { path: string, success: boolean, };

export type Outside = "reveal" | "open";

export type MenuCommand = { 
/**
 * The key as the UI writes it, e.g. `⌘⇧V`.
 */
key: string, label: string, 
/**
 * The key in Tauri's accelerator syntax, e.g. `Cmd+Shift+KeyV`.
 */
accelerator: string, };

export type MenuGroup = { title: string, commands: Array<MenuCommand>, };

export type MenuKey = { key: string, inPage: boolean, };

export const FLOAT_LAYER = "float" as const;

export const FLOAT_TOP = 38.0 as const;

export const EVENTS = {"browserDownloaded":"browser_downloaded","browserFailed":"browser_failed","browserHovered":"browser_hovered","browserOpened":"browser_opened","browserPaged":"browser_paged","menuCommand":"menu_command"} as const;

export const COMMANDS = ["title_bar_double_click","browser_open","browser_close","browser_move","browser_load","browser_show_page","browser_go","browser_find","browser_forget","app_menu","file_outside","haptic_alignment"] as const;
