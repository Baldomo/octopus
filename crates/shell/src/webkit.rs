//! What a browser panel's page tells the client that draws it, whichever engine draws it, and the driver of WebKit's
//! `WKWebView`, which draws the macOS desktop's pages and the iOS phone's.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Go {
    Back,
    Forward,
    Reload,
    ReloadFromOrigin,
}

/// Where a finger dragging in from the screen's left edge stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Drag {
    Start,
    Move,
    End,
    Cancel,
}

/// What a page's view tells: a change of its page, which `browsing::BrowserPages` takes alike on every client, or one
/// only the device that draws it acts on.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum Told<Device> {
    Page(Change),
    Device(Device),
}

impl<Device> From<Change> for Told<Device> {
    fn from(change: Change) -> Self {
        Self::Page(change)
    }
}

/// What a page's view tells of the page it shows, on every client.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    Seen {
        url: Option<String>,
        loading: bool,
        back: bool,
        forward: bool,
    },
    Titled(String),
    Failed {
        url: String,
        error: String,
    },
    /// A page the page opens in a new tab, selected or behind the selected one.
    Opened {
        url: String,
        select: bool,
    },
    /// A link a finger holds, in an engine that shows no menu of its own for it.
    Held {
        link: String,
    },
    /// A download the page started ended, saved as `file` in `folder` when `success`, named as the user finds them.
    Downloaded {
        file: String,
        folder: String,
        success: bool,
    },
}

/// What only a desktop's page view tells.
pub enum DesktopChange {
    /// The link under the pointer, or none.
    Hovered { link: Option<String> },
}

/// What only a phone's page view tells.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PhoneChange {
    /// A drag from the screen's left edge the page's history leaves to its screen: how far right of where it began
    /// the finger is, and how fast it moves, in points and points a second.
    EdgeDragged { drag: Drag, x: f64, velocity: f64 },
    /// The OS ended the page to reclaim its memory.
    Ended,
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[allow(unsafe_code)]
mod apple;
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub use apple::*;

#[cfg(test)]
mod tests {
    use super::{Change, PhoneChange, Told};

    #[test]
    fn a_change_reads_as_the_android_plugin_writes_it() {
        let read = |json: &str| serde_json::from_str::<Told<PhoneChange>>(json).expect(json);
        let page = |json: &str| match read(json) {
            Told::Page(change) => change,
            Told::Device(_) => panic!("{json} reads as the phone's own"),
        };
        let seen = page(r#"{"seen":{"url":"https://a.test/","loading":false,"back":true,"forward":false}}"#);
        assert!(
            matches!(seen, Change::Seen { url: Some(url), loading: false, back: true, forward: false } if url == "https://a.test/")
        );
        assert!(matches!(
            page(r#"{"seen":{"loading":true,"back":false,"forward":false}}"#),
            Change::Seen { url: None, .. }
        ));
        assert!(matches!(page(r#"{"titled":"A"}"#), Change::Titled(title) if title == "A"));
        assert!(matches!(page(r#"{"failed":{"url":"u","error":"e"}}"#), Change::Failed { .. }));
        assert!(matches!(page(r#"{"opened":{"url":"u","select":true}}"#), Change::Opened { select: true, .. }));
        assert!(matches!(page(r#"{"held":{"link":"u"}}"#), Change::Held { link } if link == "u"));
        assert!(matches!(
            page(r#"{"downloaded":{"file":"a.zip","folder":"Downloads","success":true}}"#),
            Change::Downloaded { success: true, folder, .. } if folder == "Downloads"
        ));
        assert!(matches!(read(r#""ended""#), Told::Device(PhoneChange::Ended)));
        assert!(serde_json::from_str::<Told<PhoneChange>>(r#"{"failed":{"url":"u","error":"e","why":"x"}}"#).is_err());
    }
}
