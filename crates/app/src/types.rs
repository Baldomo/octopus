//! The TypeScript the UI compiles against, rendered from the Rust types and constants that cross the IPC boundary.

use octopus_proto::ops::{Entry, EntryKind, Match, Offer, PairedClient};
use octopus_proto::types::{
    AGENT_PANEL, Agent, AgentStatus, BrowserId, BrowserPanel, Checkout, Hue, NEW_BROWSER_PANEL, Panel, PtyId,
    SessionId, Swatch, Task, TaskId, TaskState, Tone, WorkspaceId,
};
use serde::Serialize;
use serde_json::json;
use ts_rs::{Config, TS};

use crate::browser::{Go, NAVIGATED_EVENT, Navigated, OnScreen};
use crate::client_menu::{Appearance, Zoom, ZoomStep};
use crate::commands::{Boot, NewTask};
use crate::hub::{FILES_CHANGED_EVENT, FilesChanged, MODEL_EVENT, Model, SERVERS_EVENT, ServerView};
use crate::layers::{Bounds, FLOAT, Floated, SHOWN_EVENT, Viewport, title_bar_height};
use crate::task_lists::{Column, Glyph, Lists, Row, Section, TaskRef, WorkspaceRef};
use crate::ui::{Modal, Prefs, Reveal, Surface, UI_EVENT, Ui, UiAct, View};
use crate::window::TaskView;

/// Where the rendered file lives, relative to the workspace root.
pub const PATH: &str = "ui/src/generated.ts";

macro_rules! declarations {
    ($config:expr, $($ty:ty),* $(,)?) => {
        [$(format!("export {}\n", <$ty as TS>::decl($config))),*]
    };
}

fn constant(name: &str, value: &impl Serialize) -> String {
    format!("export const {name} = {} as const;\n", serde_json::to_string(value).expect("constants serialize"))
}

pub fn render() -> String {
    let config = Config::default().with_large_int("number");
    let declarations = declarations!(
        &config,
        TaskId,
        WorkspaceId,
        PtyId,
        BrowserId,
        SessionId,
        TaskState,
        Checkout,
        AgentStatus,
        Hue,
        Tone,
        Swatch,
        Agent,
        Panel,
        BrowserPanel,
        Task,
        EntryKind,
        Entry,
        Match,
        Offer,
        PairedClient,
        TaskRef,
        Glyph,
        Row,
        Column,
        Section,
        WorkspaceRef,
        Lists,
        TaskView,
        ServerView,
        Model,
        FilesChanged,
        View,
        Surface,
        Modal,
        Appearance,
        Zoom,
        ZoomStep,
        Prefs,
        Reveal,
        Ui,
        UiAct,
        Viewport,
        Boot,
        NewTask,
        Bounds,
        Floated,
        OnScreen,
        Go,
        Navigated,
    );
    let constants = [
        constant("TASK_STATES", &TaskState::ALL),
        constant("LIVE_STATES", &TaskState::LIVE),
        constant("CHECKOUTS", &Checkout::ALL),
        constant("HUES", &Hue::ALL),
        constant("TONES", &Tone::ALL),
        constant("AGENT_PANEL", &AGENT_PANEL),
        constant("NEW_BROWSER_PANEL", &NEW_BROWSER_PANEL),
        constant("VERSION", &env!("CARGO_PKG_VERSION")),
        constant("FLOAT_LAYER", &FLOAT),
        constant("FLOAT_TOP", &title_bar_height()),
        constant(
            "EVENTS",
            &json!({
                "model": MODEL_EVENT,
                "servers": SERVERS_EVENT,
                "filesChanged": FILES_CHANGED_EVENT,
                "browserNavigated": NAVIGATED_EVENT,
                "ui": UI_EVENT,
                "windowShown": SHOWN_EVENT,
            }),
        ),
    ];
    let header =
        "// Rendered from the Rust types by `cargo run -p octopus-app --example types`. Edit those, not this.\n";
    std::iter::once(header.to_string()).chain(declarations).chain(constants).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_ui_compiles_against_the_current_rust_types() {
        let committed = include_str!("../../../ui/src/generated.ts");
        assert!(
            committed == super::render(),
            "{} drifted from the Rust types; run `cargo run -p octopus-app --example types`",
            super::PATH
        );
    }
}
