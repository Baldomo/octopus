//! What UIKit is told about the iOS webview: the page runs under the status bar and the home indicator, clearing them
//! by the safe area's insets, the edge swipe steps back through the page's history, and the window takes the
//! appearance the client menu picks, so the status bar reads against the page.
#![allow(unsafe_code, reason = "a WKWebView reaches UIKit only through its raw pointer")]

use std::sync::Arc;

use objc2::msg_send;
use objc2::rc::Retained;
use objc2_ui_kit::{UIScrollView, UIScrollViewContentInsetAdjustmentBehavior, UIUserInterfaceStyle, UIView};
use octopus_shell::client_menu::Appearance;
use octopus_shell::ui::UiStore;
use tauri::{Manager, WebviewWindow};

pub fn set_up(app: &tauri::App) -> anyhow::Result<()> {
    let window = app.get_webview_window("main").ok_or_else(|| anyhow::anyhow!("no main window"))?;
    on_view(&window, |view| {
        // SAFETY: a WKWebView's `scrollView` is its UIScrollView.
        let scroll: Retained<UIScrollView> = unsafe { msg_send![view, scrollView] };
        scroll.setContentInsetAdjustmentBehavior(UIScrollViewContentInsetAdjustmentBehavior::Never);
        // SAFETY: a WKWebView takes this property; the edge swipe then steps back through the page's history.
        let () = unsafe { msg_send![view, setAllowsBackForwardNavigationGestures: true] };
    })?;
    let ui = Arc::clone(app.state::<Arc<UiStore>>().inner());
    show(&window, ui.native().appearance)?;
    ui.follow(move |before, after| {
        if before.appearance == after.appearance {
            return Ok(());
        }
        show(&window, after.appearance).map_err(|error| format!("show the {:?} appearance: {error}", after.appearance))
    });
    Ok(())
}

/// The whole window takes the appearance, so the status bar reads against the page.
fn show(window: &WebviewWindow, appearance: Appearance) -> tauri::Result<()> {
    let style = match appearance {
        Appearance::System => UIUserInterfaceStyle::Unspecified,
        Appearance::Light => UIUserInterfaceStyle::Light,
        Appearance::Dark => UIUserInterfaceStyle::Dark,
    };
    on_view(window, move |view| {
        view.window().expect("the webview shows in a window").setOverrideUserInterfaceStyle(style);
    })
}

/// Runs `act` on the main thread with the window's WKWebView.
fn on_view(window: &WebviewWindow, act: impl FnOnce(&UIView) + Send + 'static) -> tauri::Result<()> {
    window.with_webview(move |webview| {
        // SAFETY: on iOS `inner` is the window's live WKWebView, a UIView, for as long as this closure runs.
        act(unsafe { &*webview.inner().cast::<UIView>() });
    })
}
