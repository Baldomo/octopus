//! What UIKit is told about the iOS webview: the page runs under the status bar and the home indicator, clearing them
//! by the safe area's insets, and the window takes the appearance the client menu picks, so the status bar reads
//! against the page. The page follows the edge swipe itself, so the swipe moves its live screens. The page hears where
//! the keyboard is going as it starts to move, as `octopus-keyboard` with the height it leaves the page and when, in
//! milliseconds since the epoch, it started, since the webview reports the keyboard to the page only once it stops.
//! The keyboard rises bare, without the bar of previous, next and Done WebKit puts over it.
#![allow(unsafe_code, reason = "a WKWebView reaches UIKit only through its raw pointer")]

use std::ptr::NonNull;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{Message, ffi, msg_send, sel};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSNotification, NSNotificationCenter, NSOperationQueue, NSString, NSValue};
use objc2_ui_kit::{
    NSValueUIGeometryExtensions, UIKeyboardFrameEndUserInfoKey, UIKeyboardWillChangeFrameNotification, UIScrollView,
    UIScrollViewContentInsetAdjustmentBehavior, UIUserInterfaceStyle, UIView,
};
use octopus_shell::client_menu::Appearance;
use octopus_shell::ui::UiStore;
use tauri::{Manager, WebviewWindow};

pub(crate) fn set_up(app: &tauri::App) -> anyhow::Result<()> {
    let window = app.get_webview_window("main").ok_or_else(|| anyhow::anyhow!("no main window"))?;
    bare_keyboard()?;
    on_view(&window, |view| {
        // SAFETY: a WKWebView's `scrollView` is its UIScrollView.
        let scroll: Retained<UIScrollView> = unsafe { msg_send![view, scrollView] };
        scroll.setContentInsetAdjustmentBehavior(UIScrollViewContentInsetAdjustmentBehavior::Never);
        follow_keyboard(view.retain());
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

/// Tells the page, each time the keyboard starts to move, the height of the webview it will leave uncovered.
fn follow_keyboard(view: Retained<UIView>) {
    let moving = RcBlock::new(move |notification: NonNull<NSNotification>| {
        // SAFETY: the center hands the block a live notification.
        let info = unsafe { notification.as_ref() }.userInfo();
        // SAFETY: UIKit's keys are static NSStrings.
        let end = info.and_then(|info| info.objectForKey(unsafe { UIKeyboardFrameEndUserInfoKey }));
        let Some(end) = end.and_then(|end| end.downcast::<NSValue>().ok()) else { return };
        let Some(window) = view.window() else { return };
        let screen = window.screen().coordinateSpace();
        // SAFETY: the keyboard's end frame is an NSValue holding a CGRect in the screen's coordinates, and a UIView is
        // a UICoordinateSpace that converts from another.
        let frame: CGRect = unsafe { msg_send![&*view, convertRect: end.CGRectValue(), fromCoordinateSpace: &*screen] };
        let height = frame.origin.y.clamp(0.0, view.bounds().size.height);
        let at = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(since) => since.as_secs_f64() * 1000.0,
            Err(error) => return eprintln!("octopus app: read the clock the keyboard's move is timed by: {error}"),
        };
        let moved = format!(
            "dispatchEvent(new CustomEvent('octopus-keyboard', {{ detail: {{ height: {height}, at: {at} }} }}))"
        );
        let unanswered: Option<&DynBlock<dyn Fn(*mut AnyObject, *mut AnyObject)>> = None;
        // SAFETY: the view is a WKWebView, which runs the script in its page; a nil handler leaves the result unread.
        let () = unsafe {
            msg_send![&*view, evaluateJavaScript: &*NSString::from_str(&moved), completionHandler: unanswered]
        };
    });
    // SAFETY: the main queue runs the block on the main thread, which owns the view.
    let observer = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(UIKeyboardWillChangeFrameNotification),
            None,
            Some(&NSOperationQueue::mainQueue()),
            &moving,
        )
    };
    // The page follows the keyboard for as long as the app runs.
    std::mem::forget(observer);
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

/// WebKit's content view, the responder that raises the keyboard, offers no view to show above it.
fn bare_keyboard() -> anyhow::Result<()> {
    extern "C-unwind" fn no_view(_: *mut AnyObject, _: Sel) -> *mut AnyObject {
        std::ptr::null_mut()
    }
    let content = AnyClass::get(c"WKContentView").ok_or_else(|| anyhow::anyhow!("WebKit has no WKContentView"))?;
    type Getter = extern "C-unwind" fn(*mut AnyObject, Sel) -> *mut AnyObject;
    // SAFETY: `inputAccessoryView` takes no argument and returns a nullable UIView, as `no_view` does; the runtime calls
    // an IMP with the signature its type encoding names.
    unsafe {
        let imp = std::mem::transmute::<Getter, Imp>(no_view);
        ffi::class_replaceMethod(
            std::ptr::from_ref(content).cast_mut(),
            sel!(inputAccessoryView),
            imp,
            c"@@:".as_ptr(),
        );
    }
    Ok(())
}

/// Runs `act` on the main thread with the window's WKWebView.
fn on_view(window: &WebviewWindow, act: impl FnOnce(&UIView) + Send + 'static) -> tauri::Result<()> {
    window.with_webview(move |webview| {
        // SAFETY: on iOS `inner` is the window's live WKWebView, a UIView, for as long as this closure runs.
        act(unsafe { &*webview.inner().cast::<UIView>() });
    })
}
