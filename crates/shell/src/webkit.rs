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

/// What a page's view tells of the page it shows.
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
    /// The link under the pointer, or none.
    Hovered {
        link: Option<String>,
    },
    /// A drag from the screen's left edge the page's history leaves to its screen: how far right of where it began
    /// the finger is, and how fast it moves, in points and points a second.
    EdgeDragged {
        drag: Drag,
        x: f64,
        velocity: f64,
    },
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
    use super::Change;

    #[test]
    fn a_change_reads_as_the_android_plugin_writes_it() {
        let read = |json: &str| serde_json::from_str::<Change>(json).expect(json);
        let seen = read(r#"{"seen":{"url":"https://a.test/","loading":false,"back":true,"forward":false}}"#);
        assert!(
            matches!(seen, Change::Seen { url: Some(url), loading: false, back: true, forward: false } if url == "https://a.test/")
        );
        assert!(matches!(
            read(r#"{"seen":{"loading":true,"back":false,"forward":false}}"#),
            Change::Seen { url: None, .. }
        ));
        assert!(matches!(read(r#"{"titled":"A"}"#), Change::Titled(title) if title == "A"));
        assert!(matches!(read(r#"{"failed":{"url":"u","error":"e"}}"#), Change::Failed { .. }));
        assert!(matches!(read(r#"{"opened":{"url":"u","select":true}}"#), Change::Opened { select: true, .. }));
        assert!(matches!(read(r#""ended""#), Change::Ended));
        assert!(serde_json::from_str::<Change>(r#"{"failed":{"url":"u","error":"e","why":"x"}}"#).is_err());
    }
}
