//! The TypeScript the desktop UI compiles against beside the client app core's, rendered from the Rust types and
//! constants only the desktop app sends across the IPC boundary.

use octopus_shell::types::constant;
use serde_json::json;
use ts_rs::Config;

use crate::browser::{
    DOWNLOADED_EVENT, Downloaded, FAILED_EVENT, Failed, Go, HOVERED_EVENT, Hovered, OPENED_EVENT, OnScreen, Opened,
    PAGED_EVENT, Paged,
};
use crate::commands::Outside;
use crate::layers::{FLOAT, title_bar_height};
use crate::menu::{COMMAND_EVENT, MenuCommand, MenuGroup, MenuKey};

/// Where the rendered file lives, relative to the workspace root.
pub const PATH: &str = "ui/desktop/src/generated.ts";

pub fn render() -> String {
    let config = Config::default().with_large_int("number");
    let declarations = octopus_shell::declarations!(
        &config,
        OnScreen,
        Go,
        Paged,
        Failed,
        Opened,
        Hovered,
        Downloaded,
        Outside,
        MenuCommand,
        MenuGroup,
        MenuKey
    );
    let events = json!({
        "browserPaged": PAGED_EVENT,
        "browserFailed": FAILED_EVENT,
        "browserOpened": OPENED_EVENT,
        "browserHovered": HOVERED_EVENT,
        "browserDownloaded": DOWNLOADED_EVENT,
        "menuCommand": COMMAND_EVENT,
    });
    let constants =
        [constant("FLOAT_LAYER", &FLOAT), constant("FLOAT_TOP", &title_bar_height()), constant("EVENTS", &events)];
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
