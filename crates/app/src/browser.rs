use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tauri::webview::{PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewUrl};
use ts_rs::TS;

use crate::{UI, emit, lock};

pub const NAVIGATED_EVENT: &str = "browser_navigated";

/// Where a browser panel's page sits, in the UI's CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    fn position(self) -> LogicalPosition<f64> {
        LogicalPosition::new(self.x, self.y)
    }

    fn size(self) -> LogicalSize<f64> {
        LogicalSize::new(self.width, self.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Go {
    Back,
    Forward,
    Reload,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Navigated {
    pub panel: String,
    pub url: String,
}

/// Each browser panel is a native webview laid over the left half of the window; `place` alone shows one.
#[derive(Default)]
pub struct Browsers {
    next: AtomicU64,
    webviews: Mutex<HashMap<String, Webview>>,
}

fn failed(error: tauri::Error) -> String {
    error.to_string()
}

fn page(url: &str) -> Result<Url, String> {
    let parsed = Url::parse(url).map_err(|error| format!("{url}: {error}"))?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed),
        scheme => Err(format!("a browser panel opens http and https pages, not {scheme}:")),
    }
}

impl Browsers {
    fn with<T>(&self, panel: &str, act: impl FnOnce(&Webview) -> tauri::Result<T>) -> Result<T, String> {
        let webviews = lock(&self.webviews);
        let webview = webviews.get(panel).ok_or_else(|| format!("no browser panel {panel}"))?;
        act(webview).map_err(failed)
    }

    /// Creates the panel's webview hidden, or points the existing one at `url`.
    pub fn open(&self, app: &AppHandle, panel: &str, url: &str) -> Result<(), String> {
        let url = page(url)?;
        if let Some(webview) = lock(&self.webviews).get(panel) {
            return webview.navigate(url).map_err(failed);
        }
        let window = app.get_window(UI).ok_or("the window is gone")?;
        let label = format!("browser-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let (events, name) = (app.clone(), panel.to_string());
        let builder = WebviewBuilder::new(label, WebviewUrl::External(url)).on_page_load(move |_, payload| {
            if payload.event() == PageLoadEvent::Finished {
                emit(&events, NAVIGATED_EVENT, Navigated { panel: name.clone(), url: payload.url().to_string() });
            }
        });
        let webview =
            window.add_child(builder, LogicalPosition::new(0.0, 0.0), LogicalSize::new(0.0, 0.0)).map_err(failed)?;
        webview.hide().map_err(failed)?;
        lock(&self.webviews).insert(panel.to_string(), webview);
        Ok(())
    }

    /// `None` hides the page: a native webview draws above every element of the UI.
    pub fn place(&self, panel: &str, bounds: Option<Bounds>) -> Result<(), String> {
        self.with(panel, |webview| match bounds {
            Some(bounds) => {
                webview.set_position(bounds.position())?;
                webview.set_size(bounds.size())?;
                webview.show()
            }
            None => webview.hide(),
        })
    }

    pub fn go(&self, panel: &str, go: Go) -> Result<(), String> {
        self.with(panel, |webview| match go {
            Go::Back => webview.eval("history.back()"),
            Go::Forward => webview.eval("history.forward()"),
            Go::Reload => webview.reload(),
        })
    }

    pub fn close(&self, panel: &str) -> Result<(), String> {
        let webview = lock(&self.webviews).remove(panel).ok_or_else(|| format!("no browser panel {panel}"))?;
        webview.close().map_err(failed)
    }
}
