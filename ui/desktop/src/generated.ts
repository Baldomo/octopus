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

export type Progress = { "state": "installing" } | { "state": "failed", error: string, } | { "state": "installed" };

export type SetupView = { server: string, log: string, progress: Progress, };

export type ShowApp = { frame: ShowRect, picture: string, };

export type ShowDrive = { "do": "start" } | { "do": "press", target: ShowTarget, } | { "do": "type", target: ShowTarget, text: string, } | { "do": "sidebar", shown: boolean, } | { "do": "end" };

export type ShowPoint = { x: number, y: number, };

export type ShowRect = { x: number, y: number, width: number, height: number, };

export type ShowScene = { display: ShowRect, bounds: ShowRect, wallpaper: string | null, drives: boolean, };

export type ShowSpot = { target: ShowTarget, rect: ShowRect, };

export type ShowTarget = "edge" | "sidebar" | "add_workspace" | "host" | "folder" | "choose" | "new_task" | "form" | "title" | "create" | "terminal";

export const FLOAT_LAYER = "float" as const;

export const TITLE_H = 38.0 as const;

export const EVENTS = {"browserHovered":"browser_hovered","menuCommand":"menu_command","setup":"setup","showApp":"show_app","showDrive":"show_drive","showEnd":"show_end","showPlay":"show_play","showPointer":"show_pointer","showSpot":"show_spot"} as const;

export const TOUR = {"project":"tidewatch","server":"tidepool","task":"Add a weekly tide chart"} as const;

export const COMMANDS = ["title_bar_double_click","browser_load","browser_show_page","browser_go","browser_find","browser_forget","app_menu","file_outside","haptic_alignment","setup","setup_retry","show_playing","show_scene","show_app","show_drive","show_spot","show_ready","show_cursor","show_alert","show_skip","skill_install"] as const;
