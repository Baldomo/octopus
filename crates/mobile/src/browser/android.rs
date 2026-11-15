//! Android draws each page in an `android.webkit.WebView` that `BrowserPlugin.kt` makes and places beside the app's
//! own webview. The plugin tells of each page through one channel: every change of the page, each navigation of its
//! main frame, which waits until it is allowed, and each request a file page makes, which waits for the bytes its
//! server reads.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use anyhow::Context;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use octopus_shell::webkit::{self, Go, PhoneChange};
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use tauri::http::header;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::plugin::{PluginHandle, TauriPlugin};
use tauri::{AppHandle, Manager, Wry};

use super::Placed;

/// `BrowserPlugin.kt`, which owns every page's `WebView`.
struct Pages(PluginHandle<Wry>);

/// Whether the System WebView draws pages, as the plugin said when it began to watch them.
static DRAWS: OnceLock<bool> = OnceLock::new();

pub(crate) fn plugin() -> TauriPlugin<Wry> {
    tauri::plugin::Builder::new("browser")
        .setup(|app, api| {
            app.manage(Pages(api.register_android_plugin("com.noi.octopus", "BrowserPlugin")?));
            Ok(())
        })
        .build()
}

/// What the plugin tells of a page, named by its browser panel's key, or asks for it.
#[derive(Deserialize)]
#[serde(tag = "told", rename_all = "snake_case", deny_unknown_fields)]
enum Told {
    Changed {
        key: String,
        change: webkit::Told<PhoneChange>,
    },
    /// The page's main frame heads to another page, and waits for `headed`.
    Heading {
        id: u64,
        key: String,
        url: String,
    },
    /// A file page asks for a file, and waits for `answer`.
    Asked {
        id: u64,
        url: String,
        method: String,
        range: Option<String>,
    },
}

#[derive(Serialize)]
struct Watch {
    told: Channel,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Watching {
    draws: bool,
}

#[derive(Serialize)]
struct Headed {
    id: u64,
    allowed: bool,
}

/// A file's bytes in base64, as the plugin's commands carry JSON.
#[derive(Serialize)]
struct Answer {
    id: u64,
    status: u16,
    reason: &'static str,
    kind: String,
    headers: BTreeMap<String, String>,
    body: String,
}

#[derive(Serialize)]
struct Load<'a> {
    key: &'a str,
    url: &'a str,
}

#[derive(Serialize)]
struct Place {
    page: Option<Placed>,
}

#[derive(Serialize)]
struct Went<'a> {
    key: &'a str,
    go: Go,
}

#[derive(Serialize)]
struct Find<'a> {
    key: &'a str,
    text: String,
    backwards: bool,
}

#[derive(Serialize)]
struct Forget<'a> {
    key: &'a str,
}

async fn done(app: &AppHandle, command: &str, args: impl Serialize) -> Result<(), String> {
    let pages = &app.state::<Pages>().0;
    pages
        .run_mobile_plugin_async::<IgnoredAny>(command, args)
        .await
        .map(drop)
        .map_err(|error| format!("{command}: {error}"))
}

/// Hands the plugin its channel and hears it for as long as the app runs, each page's changes in their order.
pub(crate) fn set_up(app: &tauri::App) -> anyhow::Result<()> {
    let (tell, mut told) = tokio::sync::mpsc::unbounded_channel();
    let channel = Channel::new(move |body| {
        if let InvokeResponseBody::Json(json) = body {
            match serde_json::from_str::<Told>(&json) {
                Ok(told) => drop(tell.send(told)),
                Err(error) => eprintln!("octopus app: the browser plugin told {json}: {error}"),
            }
        }
        Ok(())
    });
    let Watching { draws } =
        app.state::<Pages>().0.run_mobile_plugin("watch", Watch { told: channel }).context("watch the pages")?;
    DRAWS.set(draws).expect("the app sets up once");
    let app = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        while let Some(told) = told.recv().await {
            hear(&app, told).await;
        }
    });
    Ok(())
}

async fn hear(app: &AppHandle, told: Told) {
    match told {
        Told::Changed { key, change } => super::changed(app, &key, change).await,
        Told::Heading { id, key, url } => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let allowed = super::heading(&app, &key, &url).await;
                reply(&app, "headed", Headed { id, allowed }).await;
            });
        }
        Told::Asked { id, url, method, range } => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let answer = answer(&app, id, &url, &method, range.as_deref()).await;
                reply(&app, "answer", answer).await;
            });
        }
    }
}

async fn reply(app: &AppHandle, command: &str, args: impl Serialize) {
    if let Err(error) = done(app, command, args).await {
        eprintln!("octopus app: {error}");
    }
}

async fn answer(app: &AppHandle, id: u64, url: &str, method: &str, range: Option<&str>) -> Answer {
    let reply = super::serve(app, url, method, range).await;
    let text = |value: &header::HeaderValue| value.to_str().map(str::to_owned).ok();
    let kind = reply.headers().get(header::CONTENT_TYPE).and_then(text).unwrap_or_default();
    let headers = reply
        .headers()
        .iter()
        .filter(|(name, _)| *name != header::CONTENT_TYPE)
        .filter_map(|(name, value)| Some((name.to_string(), text(value)?)))
        .collect();
    let status = reply.status();
    let reason = status.canonical_reason().expect("a file page is answered with a standard status");
    Answer { id, status: status.as_u16(), reason, kind, headers, body: STANDARD.encode(reply.body()) }
}

pub(super) async fn draws(_: &AppHandle) -> Result<bool, String> {
    DRAWS.get().copied().ok_or_else(|| "the browser plugin is not watching yet".to_string())
}

pub(super) async fn load(app: &AppHandle, key: &str, url: &str) -> Result<(), String> {
    done(app, "load", Load { key, url }).await
}

pub(super) async fn place(app: &AppHandle, placed: Option<Placed>) -> Result<(), String> {
    done(app, "place", Place { page: placed }).await
}

pub(super) async fn go(app: &AppHandle, key: &str, go: Go) -> Result<(), String> {
    done(app, "go", Went { key, go }).await
}

pub(super) async fn find(app: &AppHandle, key: &str, text: String, backwards: bool) -> Result<(), String> {
    done(app, "find", Find { key, text, backwards }).await
}

pub(super) async fn forget(app: &AppHandle, key: &str) -> Result<(), String> {
    done(app, "forget", Forget { key }).await
}
