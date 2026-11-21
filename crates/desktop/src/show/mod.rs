//! The first open's show: on the launch that installs this machine's first server, a transparent window above
//! everything on every display plays show.html in sync to the song, then closes on octopus's own window beneath. It
//! asks for no permission, so it sees no other app's pixels: each display's page gets the display's wallpaper, the
//! pointer, and octopus's window, its frame and a snapshot of it, which the show dives into and then uncovers live.
//!
//! In the middle the show makes a first task in octopus's own window: it lists the blank tidepool, a demo computer that
//! starts empty, in the account until the tour ends, and drives the window's UI a step at a time, each layer telling
//! where the places the tour works sit whenever they move.
//!
//! Each page asks for its scene, prepares, and says it is ready; once every page is, the windows take the pointer,
//! octopus's window comes to the front beneath them, the song starts and every page gets the same start instant. Every
//! page draws; the driving page alone also asks octopus for what the show does at its moments: the window's snapshot,
//! the tour's steps, the cursor and the alert sound. The show ends when the song does, at the first key or click, or
//! when a page is not ready in time.
//!
//! The run and the sound are the same everywhere; what each desktop does differently sits in its platform module,
//! `appkit` on macOS and `gtk` on Linux, which both offer the same functions: `displays`, `pointer`, `cover`,
//! `snapshot`, `keep_drawing`, `cursor` and `alert_sound`.

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod appkit;
#[cfg(target_os = "linux")]
mod gtk;
mod run;
mod sound;

#[cfg(target_os = "macos")]
use appkit as platform;
#[cfg(target_os = "linux")]
use gtk as platform;

use std::sync::Arc;

use base64::Engine;
use octopus_client::demo::{self, Computer};
use octopus_shell::hub::Hub;
use octopus_shell::ui::UiStore;
use serde::{Deserialize, Serialize};
use tauri::Manager;
use ts_rs::TS;

pub(crate) use run::Show;

/// How finely the wallpapers and the snapshot of octopus's window keep their detail, from 0 to 1.
const JPEG_QUALITY: f64 = 0.88;

/// A display as the platform finds it: where it sits, in global points, and its wallpaper drawn to cover it, as an
/// image URL.
struct Display {
    rect: ShowRect,
    wallpaper: Option<String>,
}

/// Where an image of `natural` size lands when drawn to cover `size` about its middle.
fn covering(natural: (f64, f64), size: (f64, f64)) -> ShowRect {
    let k = (size.0 / natural.0).max(size.1 / natural.1);
    let (width, height) = (natural.0 * k, natural.1 * k);
    ShowRect { x: (size.0 - width) / 2.0, y: (size.1 - height) / 2.0, width, height }
}

fn jpeg_url(jpeg: &[u8]) -> String {
    format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(jpeg))
}

pub(crate) const PLAY_EVENT: &str = "show_play";
pub(crate) const POINTER_EVENT: &str = "show_pointer";
pub(crate) const END_EVENT: &str = "show_end";
pub(crate) const DRIVE_EVENT: &str = "show_drive";
pub(crate) const SPOT_EVENT: &str = "show_spot";
pub(crate) const APP_EVENT: &str = "show_app";

/// A rect in points: global ones from the main display's top-left corner, or a window's from its own.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShowRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
pub(crate) struct ShowPoint {
    pub x: f64,
    pub y: f64,
}

/// What one display's page draws from: its display, the rect around every display, and its wallpaper as an image URL
/// covering the display; and whether it is the page that drives the show.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub(crate) struct ShowScene {
    pub display: ShowRect,
    pub bounds: ShowRect,
    pub wallpaper: Option<String>,
    pub drives: bool,
}

/// octopus's window as it stands: its frame and a picture of its UI as an image URL.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub(crate) struct ShowApp {
    pub frame: ShowRect,
    pub picture: String,
}

/// A place in octopus's window the tour works, in the order it makes a first task: the window's left edge, where
/// the sidebar comes out, the sidebar, "+ workspace", the folder picker's blank tidepool, its project folder and its
/// Choose, the new workspace's "+", the new-task form, its title and Create, and the task's terminals.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ShowTarget {
    Edge,
    Sidebar,
    AddWorkspace,
    Host,
    Folder,
    Choose,
    NewTask,
    Form,
    Title,
    Create,
    Terminal,
}

/// A step of the tour for octopus's window: hand the window over to it, press a target, type `text` into it, show or
/// hide the sidebar, or put the window back as the tour found it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "do", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ShowDrive {
    Start,
    Press { target: ShowTarget },
    Type { target: ShowTarget, text: String },
    Sidebar { shown: bool },
    End,
}

/// Where a target sits, in the window's points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShowSpot {
    pub target: ShowTarget,
    pub rect: ShowRect,
}

/// Plays the show when `plays`, and first forgets a blank tidepool a show cut short left in the account. A playing show
/// is the app's state until the launch ends; a show that cannot open its windows is reported and plays nothing, so the
/// launch goes on without it.
pub(crate) fn start(app: &tauri::App, plays: bool, ui: &Arc<UiStore>) {
    if let Err(error) = demo::forget(&app.state::<Arc<Hub>>().me, Computer::Blank) {
        ui.report(format!("forget the show's tidepool: {error:#}"));
    }
    if !plays {
        return;
    }
    match Show::start(app) {
        Ok(Some(show)) => {
            app.manage(show);
        }
        Ok(None) => {}
        Err(error) => ui.report(format!("play the first open's show: {error:#}")),
    }
}
