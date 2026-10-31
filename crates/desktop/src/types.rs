//! The TypeScript the desktop UI compiles against beside the client app core's, rendered from the Rust types and
//! constants only the desktop app sends across the IPC boundary.

use octopus_shell::types::constant;
use serde_json::json;
use ts_rs::Config;

use crate::browser::{Go, NAVIGATED_EVENT, Navigated, OnScreen};
use crate::commands::Outside;
use crate::layers::{FLOAT, title_bar_height};

/// Where the rendered file lives, relative to the workspace root.
pub const PATH: &str = "ui/desktop/src/generated.ts";

pub fn render() -> String {
    let config = Config::default().with_large_int("number");
    let declarations = octopus_shell::declarations!(&config, OnScreen, Go, Navigated, Outside);
    let constants = [
        constant("FLOAT_LAYER", &FLOAT),
        constant("FLOAT_TOP", &title_bar_height()),
        constant("EVENTS", &json!({ "browserNavigated": NAVIGATED_EVENT })),
    ];
    let header = concat!(
        "// Rendered from the Rust types by `cargo run -p octopus-desktop --example types`. Edit those, not this.\n",
        "import type { BrowserId, Bounds, TaskRef } from \"@octopus/shared/generated.ts\";\n",
    );
    std::iter::once(header.to_string()).chain(declarations).chain(constants).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_ui_compiles_against_the_current_rust_types() {
        let committed = include_str!("../../../ui/desktop/src/generated.ts");
        assert!(
            committed == super::render(),
            "{} drifted from the Rust types; run `cargo run -p octopus-desktop --example types`",
            super::PATH
        );
    }
}
