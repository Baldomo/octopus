//! The phone's Browser screen draws each browser panel's page with the OS's own browser engine, in a native view of
//! its own that the screen places over its page area: a `WKWebView` on iOS, an `android.webkit.WebView` on Android.
//! `octopus_shell::browsing` keeps each page, and a file page loads through `octopus_shell::served_files`.

use std::sync::Arc;

use octopus_proto::types::BrowserId;
use octopus_shell::browsing::{BrowserPages, OnScreen, panel_key};
use octopus_shell::emit;
use octopus_shell::hub::Hub;
use octopus_shell::served_files::{SCHEME, ServedFiles};
use octopus_shell::task_lists::TaskRef;
use octopus_shell::ui::Bounds;
use octopus_shell::webkit::{Change, Drag, Go, PhoneChange, Told};
use serde::Serialize;
use tauri::http::{Method, Response, StatusCode};
use tauri::{AppHandle, Manager, Url};

#[cfg(target_os = "ios")]
#[path = "browser/ios.rs"]
mod engine;
#[cfg(target_os = "android")]
#[path = "browser/android.rs"]
mod engine;

#[cfg(target_os = "android")]
pub(crate) use engine::{plugin, set_up};

const ENDED_EVENT: &str = "browser_ended";
const EDGE_EVENT: &str = "browser_edge";

/// The label a page's view asks for files under, which is none of the UI's webviews.
const PAGE_VIEW: &str = "browser";

/// A browser panel whose page the OS ended, and whose view is gone with it.
#[derive(Clone, Serialize)]
struct Ended {
    task: String,
    panel: BrowserId,
}

/// A finger dragging in from the left edge over a page whose history has nowhere to go back to.
#[derive(Clone, Serialize)]
struct EdgeDrag {
    phase: Drag,
    /// How far right of where it began the finger is, and how fast it moves, in CSS pixels and pixels a second.
    x: f64,
    velocity: f64,
}

#[derive(Serialize)]
struct Placed {
    key: String,
    bounds: Bounds,
    covered: bool,
}

/// Whether the page of the panel `key` may navigate to `url`, which only a web page or a file page may.
async fn heading(app: &AppHandle, key: &str, url: &str) -> bool {
    let pages = app.state::<BrowserPages>();
    match Url::parse(url) {
        Ok(page) if matches!(page.scheme(), "http" | "https" | "about" | SCHEME) => pages.heading(key, &page).await,
        _ => {
            let error = "this phone opens only web pages and files".to_string();
            pages.changed(key, Change::Failed { url: url.to_string(), error }).await;
            false
        }
    }
}

async fn changed(app: &AppHandle, key: &str, told: Told<PhoneChange>) {
    match told {
        Told::Page(change) => app.state::<BrowserPages>().changed(key, change).await,
        Told::Device(PhoneChange::EdgeDragged { drag, x, velocity }) => {
            emit(app, EDGE_EVENT, EdgeDrag { phase: drag, x, velocity });
        }
        Told::Device(PhoneChange::Ended) => {
            let Some((at, panel)) = app.state::<BrowserPages>().panel_of(key) else { return };
            if let Err(error) = forget(app, key).await {
                eprintln!("octopus app: let go of the ended page of {key}: {error}");
            }
            emit(app, ENDED_EVENT, Ended { task: at.key(), panel });
        }
    }
}

/// Answers a file page's request for `url`.
async fn serve(app: &AppHandle, url: &str, method: &str, range: Option<&str>) -> Response<Vec<u8>> {
    match (Url::parse(url), Method::from_bytes(method.as_bytes())) {
        (Ok(loaded), Ok(method)) => {
            let (hub, served) = (app.state::<Arc<Hub>>(), app.state::<Arc<ServedFiles>>());
            served.answer(&hub, PAGE_VIEW, &loaded, &method, range).await
        }
        _ => Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Vec::new())
            .expect("a status and a body make a response"),
    }
}

async fn forget(app: &AppHandle, key: &str) -> Result<(), String> {
    app.state::<BrowserPages>().forget(key).await;
    engine::forget(app, key).await
}

/// Whether this phone's browser engine draws pages, which an Android System WebView without `NavigationListener`
/// does not.
#[tauri::command]
pub(crate) async fn phone_browser_engine(app: AppHandle) -> Result<bool, String> {
    engine::draws(&app).await
}

/// Loads `url` in the panel's view, made on its first page; `phone_browser_place` puts it on screen.
#[tauri::command]
pub(crate) async fn phone_browser_load(
    app: AppHandle,
    at: TaskRef,
    panel: BrowserId,
    url: String,
) -> Result<(), String> {
    let (key, loaded) = app.state::<BrowserPages>().load(&at, &panel, &url).await?;
    engine::load(&app, &key, loaded.as_str()).await
}

/// Shows the one view on screen at its rect, and hides every other.
#[tauri::command]
pub(crate) async fn phone_browser_place(
    app: AppHandle,
    on_screen: Option<OnScreen>,
    covered: bool,
) -> Result<(), String> {
    let placed =
        on_screen.map(|OnScreen { at, panel, bounds }| Placed { key: panel_key(&at, &panel), bounds, covered });
    engine::place(&app, placed).await
}

#[tauri::command]
pub(crate) async fn phone_browser_go(app: AppHandle, at: TaskRef, panel: BrowserId, go: Go) -> Result<(), String> {
    engine::go(&app, &panel_key(&at, &panel), go).await
}

/// Selects the next match of `text` in the page, or the one before when `backwards`, wrapping around its ends; no
/// text unmarks the matches the engine marks.
#[tauri::command]
pub(crate) async fn phone_browser_find(
    app: AppHandle,
    at: TaskRef,
    panel: BrowserId,
    text: String,
    backwards: bool,
) -> Result<(), String> {
    engine::find(&app, &panel_key(&at, &panel), text, backwards).await
}

#[tauri::command]
pub(crate) async fn phone_browser_forget(app: AppHandle, at: TaskRef, panel: BrowserId) -> Result<(), String> {
    forget(&app, &panel_key(&at, &panel)).await
}
