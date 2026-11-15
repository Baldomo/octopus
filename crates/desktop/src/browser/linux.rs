//! WebKitGTK tells what a page shows through the webview's properties and signals, which end with the webview.

use std::rc::Rc;

use tauri::Webview;
use webkit2gtk::{
    Error, FindControllerExt, FindOptions, HitTestResultExt, NetworkError, PluginError, PolicyError, WebViewExt,
};

use octopus_shell::webkit::{Change, DesktopChange, Go, Told};

/// Whether WebKit ended the load on purpose: a navigation another replaced, a load its policy stopped, such as a
/// download, or a media file it plays in a media document of its own.
fn on_purpose(error: &Error) -> bool {
    error.matches(NetworkError::Cancelled)
        || error.matches(PolicyError::FrameLoadInterruptedByPolicyChange)
        || error.matches(PluginError::WillHandleLoad)
}

pub fn watch(webview: &Webview, watcher: impl Fn(Told<DesktopChange>) + Send + 'static) -> tauri::Result<()> {
    webview.with_webview(move |platform| {
        let view = platform.inner();
        let watcher: Rc<dyn Fn(Told<DesktopChange>)> = Rc::new(watcher);
        let seen = |watcher: Rc<dyn Fn(Told<DesktopChange>)>| {
            move |view: &webkit2gtk::WebView| {
                watcher(Told::Page(Change::Seen {
                    url: view.uri().map(|uri| uri.to_string()),
                    loading: view.is_loading(),
                    back: view.can_go_back(),
                    forward: view.can_go_forward(),
                }));
            }
        };
        view.connect_uri_notify(seen(Rc::clone(&watcher)));
        view.connect_is_loading_notify(seen(Rc::clone(&watcher)));
        let titled = Rc::clone(&watcher);
        view.connect_title_notify(move |view| {
            titled(Told::Page(Change::Titled(view.title().map(|title| title.to_string()).unwrap_or_default())));
        });
        let hovering = Rc::clone(&watcher);
        view.connect_mouse_target_changed(move |_, hit, _| {
            hovering(Told::Device(DesktopChange::Hovered { link: hit.link_uri().map(|link| link.to_string()) }));
        });
        view.connect_load_failed(move |_, _, url, error| {
            if !on_purpose(error) {
                watcher(Told::Page(Change::Failed { url: url.to_string(), error: error.to_string() }));
            }
            true
        });
    })
}

pub(crate) fn unwatch(_: &Webview) -> tauri::Result<()> {
    Ok(())
}

pub fn go(webview: &Webview, go: Go) -> tauri::Result<()> {
    webview.with_webview(move |platform| {
        let view = platform.inner();
        match go {
            Go::Back => view.go_back(),
            Go::Forward => view.go_forward(),
            Go::Reload => view.reload(),
            Go::ReloadFromOrigin => view.reload_bypass_cache(),
        }
    })
}

pub fn find(webview: &Webview, text: String, backwards: bool) -> tauri::Result<()> {
    webview.with_webview(move |platform| {
        let backwards = if backwards { FindOptions::BACKWARDS } else { FindOptions::empty() };
        let options = FindOptions::WRAP_AROUND | FindOptions::CASE_INSENSITIVE | backwards;
        if let Some(controller) = platform.inner().find_controller() {
            controller.search(&text, options.bits(), u32::MAX);
        }
    })
}

pub(crate) fn has_focus(_: &tauri::Window) -> bool {
    false
}
