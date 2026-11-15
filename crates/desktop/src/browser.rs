//! Each browser panel of a task renders as a native webview in the window's page layer, whose page
//! `octopus_shell::browsing` keeps. A link the user opens in a new tab, and a page's new window, go to the UI, which
//! opens them as browser panels in the task's order of tabs.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use octopus_proto::types::BrowserId;
use octopus_shell::browsing::{BrowserPages, OnScreen, panel_key};
use octopus_shell::served_files::ServedFiles;
use octopus_shell::task_lists::TaskRef;
use octopus_shell::ui::Bounds;
use octopus_shell::webkit::{Change, DesktopChange, Go, Told};
use octopus_shell::{emit, lock};
use serde::Serialize;
use tauri::async_runtime::block_on;
use tauri::webview::{DownloadEvent, NewWindowResponse, WebviewBuilder};
use tauri::{AppHandle, Manager, Url, Webview, WebviewUrl};
use ts_rs::TS;

use crate::layers::Layers;

pub(crate) const HOVERED_EVENT: &str = "browser_hovered";

/// The link under the pointer in the page on screen, and the page's rect in the UI's CSS pixels.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub(crate) struct Hovered {
    pub link: String,
    pub page: Bounds,
}

pub(crate) struct Browsers {
    app: AppHandle,
    layers: Arc<Layers>,
    next: AtomicU64,
    rendered: Mutex<HashMap<String, Webview>>,
}

fn failed(error: tauri::Error) -> String {
    error.to_string()
}

/// A download that ended at `path`, as its file's name in the folder that holds it.
fn downloaded(path: &Path, success: bool) -> Change {
    let file = path.file_name().map(|file| file.to_string_lossy().into_owned()).unwrap_or_default();
    let folder = path.parent().map(|folder| folder.display().to_string()).unwrap_or_default();
    Change::Downloaded { file, folder, success }
}

impl Browsers {
    pub fn new(app: AppHandle, layers: Arc<Layers>) -> Self {
        Self { app, layers, next: AtomicU64::new(0), rendered: Mutex::default() }
    }

    fn with<T>(
        &self,
        at: &TaskRef,
        panel: &BrowserId,
        act: impl FnOnce(&Webview) -> tauri::Result<T>,
    ) -> Result<T, String> {
        let key = panel_key(at, panel);
        let rendered = lock(&self.rendered);
        let webview = rendered.get(&key).ok_or_else(|| format!("no browser panel {key}"))?;
        act(webview).map_err(failed)
    }

    /// Loads `url` into the panel's webview, created hidden and blank on its first page so that it is watched before
    /// it loads; `show_page` puts it on screen.
    pub async fn load(&self, at: &TaskRef, panel: &BrowserId, url: &str) -> Result<(), String> {
        let (key, loaded) = self.app.state::<BrowserPages>().load(at, panel, url).await?;
        let mut rendered = lock(&self.rendered);
        let webview = match rendered.entry(key) {
            Entry::Occupied(existing) => existing.into_mut(),
            Entry::Vacant(first) => {
                let webview = self.create(first.key())?;
                first.insert(webview)
            }
        };
        webview.navigate(loaded).map_err(failed)
    }

    fn create(&self, key: &str) -> Result<Webview, String> {
        let label = format!("browser-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let blank = WebviewUrl::External(Url::parse("about:blank").expect("about:blank is a URL"));
        let (navigating, navigated) = (self.app.clone(), key.to_string());
        let (windowing, windowed) = (self.app.clone(), key.to_string());
        let (finishing, finished) = (self.app.clone(), key.to_string());
        let downloading = Mutex::new(HashMap::<Url, PathBuf>::new());
        let builder = WebviewBuilder::new(label, blank)
            .on_navigation(move |url| block_on(navigating.state::<BrowserPages>().heading(&navigated, url)))
            .on_new_window(move |url, _| {
                let opened = Change::Opened { url: url.to_string(), select: true };
                block_on(windowing.state::<BrowserPages>().changed(&windowed, opened));
                NewWindowResponse::Deny
            })
            .on_download(move |_, event| {
                match event {
                    DownloadEvent::Requested { url, destination } => {
                        lock(&downloading).insert(url, destination.clone());
                    }
                    DownloadEvent::Finished { url, path, success } => {
                        let path = path.or_else(|| lock(&downloading).remove(&url)).unwrap_or_default();
                        let ended = downloaded(&path, success);
                        block_on(finishing.state::<BrowserPages>().changed(&finished, ended));
                    }
                    _ => {}
                }
                true
            });
        let webview = self.layers.add_page(builder).map_err(failed)?;
        let (app, key, layers) = (self.app.clone(), key.to_string(), Arc::clone(&self.layers));
        native::watch(&webview, move |told| match told {
            Told::Device(DesktopChange::Hovered { link }) => {
                let link = link.as_deref().and_then(|link| Url::parse(link).ok());
                let link = link.map(|link| app.state::<Arc<ServedFiles>>().page_of(&link));
                let hovered = link.zip(layers.page_bounds()).map(|(link, page)| Hovered { link: link.into(), page });
                emit(&app, HOVERED_EVENT, hovered);
            }
            Told::Page(change) => block_on(app.state::<BrowserPages>().changed(&key, change)),
        })
        .map_err(failed)?;
        Ok(webview)
    }

    pub(crate) fn show_page(&self, on_screen: Option<OnScreen>) -> Result<(), String> {
        let next = on_screen
            .map(|OnScreen { at, panel, bounds }| self.with(&at, &panel, |webview| Ok((webview.clone(), bounds))))
            .transpose()?;
        emit(&self.app, HOVERED_EVENT, None::<Hovered>);
        self.layers.show_page(next).map_err(failed)
    }

    pub fn go(&self, at: &TaskRef, panel: &BrowserId, go: Go) -> Result<(), String> {
        self.with(at, panel, |webview| native::go(webview, go))
    }

    /// Selects the next match of `text` in the page, or the one before when `backwards`, wrapping around its ends.
    pub fn find(&self, at: &TaskRef, panel: &BrowserId, text: String, backwards: bool) -> Result<(), String> {
        self.with(at, panel, |webview| native::find(webview, text, backwards))
    }

    /// Closes the panel's webview on this client, and every tunnel only it held.
    pub async fn forget(&self, at: &TaskRef, panel: &BrowserId) -> Result<(), String> {
        let key = panel_key(at, panel);
        self.app.state::<BrowserPages>().forget(&key).await;
        let webview = lock(&self.rendered).remove(&key).ok_or_else(|| format!("no browser panel {key}"))?;
        native::unwatch(&webview).map_err(failed)?;
        self.layers.close_page(&webview).map_err(failed)
    }
}

#[cfg(target_os = "macos")]
use octopus_shell::webkit as native;
#[cfg(target_os = "linux")]
#[path = "browser/linux.rs"]
mod native;
pub(crate) use native::has_focus as page_has_focus;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use octopus_shell::webkit::Change;

    #[test]
    fn a_download_tells_its_file_in_the_folder_that_holds_it() {
        let ended = super::downloaded(Path::new("/Users/a/Downloads/report.pdf"), true);
        assert!(matches!(
            ended,
            Change::Downloaded { file, folder, success: true } if file == "report.pdf" && folder == "/Users/a/Downloads"
        ));
    }
}
