//! The TypeScript the desktop UI compiles against beside the client app core's, rendered from the Rust types and
//! constants only the desktop app sends across the IPC boundary.

use octopus_client::demo;
use octopus_shell::types::constant;
use serde_json::json;
use ts_rs::Config;

use crate::browser::{HOVERED_EVENT, Hovered};
use crate::commands::Outside;
use crate::layers::{FLOAT, title_bar_height};
use crate::menu::{COMMAND_EVENT, MenuCommand, MenuGroup, MenuKey};
use crate::setup::{Progress, SETUP_EVENT, SetupView};
use crate::show::{
    APP_EVENT, DRIVE_EVENT, END_EVENT, PLAY_EVENT, POINTER_EVENT, SPOT_EVENT, ShowApp, ShowDrive, ShowPoint, ShowRect,
    ShowScene, ShowSpot, ShowTarget,
};

/// Where the rendered file lives, relative to the workspace root.
pub const PATH: &str = "ui/desktop/src/generated.ts";

pub fn render() -> String {
    let config = Config::default().with_large_int("number");
    let declarations = octopus_shell::declarations!(
        &config,
        Hovered,
        Outside,
        MenuCommand,
        MenuGroup,
        MenuKey,
        Progress,
        SetupView,
        ShowApp,
        ShowDrive,
        ShowPoint,
        ShowRect,
        ShowScene,
        ShowSpot,
        ShowTarget
    );
    let events = json!({
        "browserHovered": HOVERED_EVENT,
        "menuCommand": COMMAND_EVENT,
        "setup": SETUP_EVENT,
        "showApp": APP_EVENT,
        "showDrive": DRIVE_EVENT,
        "showEnd": END_EVENT,
        "showPlay": PLAY_EVENT,
        "showPointer": POINTER_EVENT,
        "showSpot": SPOT_EVENT,
    });
    let constants = [
        constant("FLOAT_LAYER", &FLOAT),
        constant("TITLE_H", &title_bar_height()),
        constant("EVENTS", &events),
        constant("TOUR", &json!({ "server": demo::NAME, "project": demo::PROJECT, "task": demo::FIRST_TASK })),
        constant("COMMANDS", &crate::COMMANDS),
    ];
    let header = concat!(
        "// Rendered from the Rust types by `cargo run -p octopus-desktop --example types`. Edit those, not this.\n",
        "import type { Bounds } from \"@octopus/shared/generated.ts\";\n",
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
