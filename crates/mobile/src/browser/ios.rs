//! iOS draws each page in a `WKWebView` that `octopus_shell::webkit` drives, a sibling of the app's own webview in its
//! window. Every view lives on the main thread.
#![allow(unsafe_code, reason = "a WKWebView reaches UIKit only through its raw pointer")]

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use objc2::msg_send;
use objc2::runtime::AnyObject;
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_ui_kit::{UIColor, UIScrollView, UIView};
use octopus_shell::webkit::{self, Asked, Embedder, Go, Page};
use tauri::AppHandle;
use tauri::async_runtime::block_on;

use super::Placed;

thread_local! {
    /// Each kept browser panel's page, by its key.
    static PAGES: RefCell<HashMap<String, Rc<Page>>> = RefCell::new(HashMap::new());
}

fn kept(key: &str) -> Option<Rc<Page>> {
    PAGES.with_borrow(|pages| pages.get(key).cloned())
}

fn page(key: &str) -> Result<Rc<Page>, String> {
    kept(key).ok_or_else(|| format!("no browser panel {key}"))
}

/// Runs `act` on the main thread with the app's webview, and waits for what it returns.
fn on_main<T: Send + 'static>(
    app: &AppHandle,
    act: impl FnOnce(&UIView) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (done, result) = std::sync::mpsc::channel();
    crate::webview::on_view(app, move |app_view| {
        let _asker_gone = done.send(act(app_view));
    })
    .map_err(|error| format!("{error:#}"))?;
    result.recv().map_err(|error| error.to_string())?
}

/// The page's view, a `UIView` the screen places.
fn native(page: &Page) -> &UIView {
    // SAFETY: a WKWebView is a UIView.
    unsafe { &*std::ptr::from_ref::<AnyObject>(page.native()).cast::<UIView>() }
}

pub(super) async fn draws(_: &AppHandle) -> Result<bool, String> {
    Ok(true)
}

/// A page with nothing loaded, hidden among the app's webview's siblings.
fn make(app: &AppHandle, app_view: &UIView, key: &str) -> Result<Page, String> {
    let (heading, asking, changing) = (app.clone(), app.clone(), app.clone());
    let (headed, changed) = (key.to_string(), key.to_string());
    let embedder = Embedder {
        allows: Box::new(move |url| block_on(super::heading(&heading, &headed, url))),
        serve: Box::new(move |asked| serve(&asking, asked)),
    };
    let page = Page::new(embedder, move |change| block_on(super::changed(&changing, &changed, change)));
    let parent = app_view.superview().ok_or("the app's webview is in no window")?;
    native(&page).setHidden(true);
    parent.addSubview(native(&page));
    Ok(page)
}

/// Answers a file page's request off the main thread, which WebKit hears the answer on.
fn serve(app: &AppHandle, asked: Asked) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Asked { id, url, method, range } = asked;
        let reply = super::serve(&app, &url, &method, range.as_deref()).await;
        if let Err(error) = app.run_on_main_thread(move || webkit::answer(id, reply)) {
            eprintln!("octopus app: answer {url}: {error}");
        }
    });
}

pub(super) async fn load(app: &AppHandle, key: &str, url: &str) -> Result<(), String> {
    let (handle, key, url) = (app.clone(), key.to_string(), url.to_string());
    on_main(app, move |app_view| {
        let page = match kept(&key) {
            Some(page) => page,
            None => {
                let page = Rc::new(make(&handle, app_view, &key)?);
                PAGES.with_borrow_mut(|pages| pages.insert(key, Rc::clone(&page)));
                page
            }
        };
        page.load(&url);
        Ok(())
    })
}

pub(super) async fn place(app: &AppHandle, placed: Option<Placed>) -> Result<(), String> {
    on_main(app, move |app_view| {
        let shown = placed.as_ref().map(|placed| placed.key.as_str());
        PAGES.with_borrow(|pages| {
            for (key, page) in pages {
                if Some(key.as_str()) != shown {
                    native(page).setHidden(true);
                }
            }
        });
        let Some(Placed { key, bounds, covered }) = placed else { return Ok(()) };
        let page = page(&key)?;
        let view = native(&page);
        let around = app_view.frame();
        view.setFrame(CGRect::new(
            CGPoint::new(around.origin.x + bounds.x, around.origin.y + bounds.y),
            CGSize::new(bounds.width, bounds.height),
        ));
        let parent = app_view.superview().ok_or("the app's webview is in no window")?;
        if covered {
            parent.insertSubview_belowSubview(view, app_view);
        } else {
            parent.insertSubview_aboveSubview(view, app_view);
        }
        view.setHidden(false);
        // The camera's scanner leaves the app's webview opaque when it ends, so each placing clears it.
        let clear = UIColor::clearColor();
        // SAFETY: a WKWebView's `scrollView` is its UIScrollView.
        let scroll: objc2::rc::Retained<UIScrollView> = unsafe { msg_send![app_view, scrollView] };
        app_view.setOpaque(false);
        app_view.setBackgroundColor(Some(&clear));
        scroll.setBackgroundColor(Some(&clear));
        Ok(())
    })
}

pub(super) async fn go(app: &AppHandle, key: &str, go: Go) -> Result<(), String> {
    let key = key.to_string();
    on_main(app, move |_| {
        page(&key)?.view().go(go);
        Ok(())
    })
}

pub(super) async fn find(app: &AppHandle, key: &str, text: String, backwards: bool) -> Result<(), String> {
    let key = key.to_string();
    on_main(app, move |_| {
        // WebKit marks no match but the one it selects.
        if !text.is_empty() {
            page(&key)?.view().find(&text, backwards);
        }
        Ok(())
    })
}

pub(super) async fn forget(app: &AppHandle, key: &str) -> Result<(), String> {
    let key = key.to_string();
    on_main(app, move |_| {
        if let Some(page) = PAGES.with_borrow_mut(|pages| pages.remove(&key)) {
            native(&page).removeFromSuperview();
        }
        Ok(())
    })
}
