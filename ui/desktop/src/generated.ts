// Rendered from the Rust types by `cargo run -p octopus-desktop --example types`. Edit those, not this.
import type { Bounds } from "@octopus/shared/generated.ts";

export type Hovered = { link: string, page: Bounds, };

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

export const EVENTS = {"browserHovered":"browser_hovered","menuCommand":"menu_command"} as const;

export const COMMANDS = ["title_bar_double_click","browser_load","browser_show_page","browser_go","browser_find","browser_forget","app_menu","file_outside","haptic_alignment"] as const;
