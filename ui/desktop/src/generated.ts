// Rendered from the Rust types by `cargo run -p octopus-desktop --example types`. Edit those, not this.
import type { BrowserId, Bounds, TaskRef } from "@octopus/shared/generated.ts";

export type OnScreen = { at: TaskRef, panel: BrowserId, bounds: Bounds, };

export type Go = "back" | "forward" | "reload";

export type Navigated = { 
/**
 * The task's key.
 */
task: string, panel: BrowserId, url: string, };

export const FLOAT_LAYER = "float" as const;

export const FLOAT_TOP = 38.0 as const;

export const EVENTS = {"browserNavigated":"browser_navigated"} as const;
