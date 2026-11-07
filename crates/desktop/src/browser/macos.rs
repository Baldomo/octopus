//! WebKit tells its embedder what a page shows through key-value observing of the webview, and a failed load through
//! its navigation delegate, whose class wry declares without the failure methods; octopus adds them, and makes the
//! delegate the webview's observer. A link clicked with ⌘ or the middle button reaches the navigation delegate's
//! policy, which octopus takes over from wry for watched webviews. The link under the pointer and the context menu
//! reach the UI delegate through WebKit's private methods, which octopus adds too. WebKit notes which methods a
//! delegate answers when it is set, so a webview set its delegates before the methods existed has them set again.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_void};
use std::sync::{Once, OnceLock};

use block2::{Block, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::{msg_send, sel};
use objc2_foundation::NSString;
use tauri::Webview;

use super::{Change, Go};

type Watcher = Box<dyn Fn(Change)>;

thread_local! {
    /// Each watched webview's watcher, by the address of its `WKWebView`.
    static WATCHERS: RefCell<HashMap<usize, Watcher>> = RefCell::new(HashMap::new());
    /// The link under the pointer in each watched webview that has one.
    static HOVERED: RefCell<HashMap<usize, String>> = RefCell::new(HashMap::new());
}

const OBSERVED: [&str; 4] = ["URL", "loading", "canGoBack", "canGoForward"];
/// A navigation another replaced, and a response WebKit hands to a download instead of showing.
const NOT_A_FAILURE: [isize; 2] = [-999, 102];
const LINK_ACTIVATED: isize = 0;
const CANCEL: isize = 0;
const SHIFT: usize = 1 << 17;
const COMMAND: usize = 1 << 20;
/// WebKit names a navigation's mouse button by its bit among the pressed buttons.
const MIDDLE_BUTTON: isize = 1 << 2;
const OPEN_LINK_IN_NEW_WINDOW: &str = "WKMenuItemIdentifierOpenLinkInNewWindow";

/// # Safety
/// `string` is nil or an `NSString`.
unsafe fn text(string: *mut AnyObject) -> Option<String> {
    let string = unsafe { string.as_ref() }?;
    let utf8: *const c_char = unsafe { msg_send![string, UTF8String] };
    Some(unsafe { CStr::from_ptr(utf8) }.to_string_lossy().into_owned())
}

/// # Safety
/// `url` is nil or an `NSURL`.
unsafe fn url_text(url: *mut AnyObject) -> Option<String> {
    let url = unsafe { url.as_ref() }?;
    unsafe { text(msg_send![url, absoluteString]) }
}

fn key(webview: *const AnyObject) -> usize {
    webview as usize
}

fn watched(webview: *const AnyObject) -> bool {
    WATCHERS.with_borrow(|watchers| watchers.contains_key(&key(webview)))
}

fn tell(webview: *const AnyObject, change: Change) {
    WATCHERS.with_borrow(|watchers| {
        if let Some(watcher) = watchers.get(&key(webview)) {
            watcher(change);
        }
    });
}

type Observed =
    unsafe extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject, *mut AnyObject, *mut AnyObject, *mut c_void);
type FailedLoad = unsafe extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject, *mut AnyObject, *mut AnyObject);
type Decide = unsafe extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject, *mut AnyObject, &Block<dyn Fn(isize)>);
type MouseOver = unsafe extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject, *mut AnyObject, usize, *mut AnyObject);
type ContextMenu =
    unsafe extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject, *mut AnyObject, *mut AnyObject) -> *mut AnyObject;
type MenuAction = unsafe extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject);

static WRY_DECIDE: OnceLock<Decide> = OnceLock::new();

unsafe extern "C-unwind" fn observed(
    _: &AnyObject,
    _: Sel,
    _: *mut AnyObject,
    webview: *mut AnyObject,
    _: *mut AnyObject,
    _: *mut c_void,
) {
    // SAFETY: the observed object is the WKWebView `watch` observes.
    let Some(view) = (unsafe { webview.as_ref() }) else { return };
    let (url, loading, back, forward) = unsafe {
        (
            url_text(msg_send![view, URL]),
            msg_send![view, isLoading],
            msg_send![view, canGoBack],
            msg_send![view, canGoForward],
        )
    };
    tell(webview, Change::Seen { url, loading, back, forward });
}

unsafe extern "C-unwind" fn failed_load(
    _: &AnyObject,
    _: Sel,
    webview: *mut AnyObject,
    _: *mut AnyObject,
    error: *mut AnyObject,
) {
    // SAFETY: WebKit passes the NSError the load failed with.
    let Some(error) = (unsafe { error.as_ref() }) else { return };
    let (code, url, description) = unsafe {
        let code: isize = msg_send![error, code];
        let info: *mut AnyObject = msg_send![error, userInfo];
        let key = NSString::from_str("NSErrorFailingURLStringKey");
        let url = info.as_ref().and_then(|info| text(msg_send![info, objectForKey: &*key]));
        (code, url, text(msg_send![error, localizedDescription]))
    };
    if !NOT_A_FAILURE.contains(&code) {
        let error = description.unwrap_or_else(|| format!("error {code}"));
        tell(webview, Change::Failed { url: url.unwrap_or_default(), error });
    }
}

/// The new tab a link clicked with ⌘ or the middle button opens, selected when ⇧ is held too.
///
/// # Safety
/// `action` is a `WKNavigationAction`.
unsafe fn new_tab(action: &AnyObject) -> Option<Change> {
    let (kind, flags, button): (isize, usize, isize) = unsafe {
        (msg_send![action, navigationType], msg_send![action, modifierFlags], msg_send![action, buttonNumber])
    };
    if kind != LINK_ACTIVATED || (flags & COMMAND == 0 && button != MIDDLE_BUTTON) {
        return None;
    }
    let url = unsafe {
        let request: *mut AnyObject = msg_send![action, request];
        url_text(msg_send![&*request, URL])
    }?;
    Some(Change::Opened { url, select: flags & SHIFT != 0 })
}

unsafe extern "C-unwind" fn decide(
    delegate: &AnyObject,
    selector: Sel,
    webview: *mut AnyObject,
    action: *mut AnyObject,
    decided: &Block<dyn Fn(isize)>,
) {
    // SAFETY: WebKit passes the navigation action it asks about.
    let opened = unsafe { action.as_ref() }.filter(|_| watched(webview)).and_then(|action| unsafe { new_tab(action) });
    match opened {
        Some(opened) => {
            decided.call((CANCEL,));
            tell(webview, opened);
        }
        None => {
            let wry = WRY_DECIDE.get().expect("octopus takes wry's policy over after saving it");
            // SAFETY: wry's own implementation, called with the arguments WebKit passed.
            unsafe { wry(delegate, selector, webview, action, decided) }
        }
    }
}

unsafe extern "C-unwind" fn mouse_over(
    _: &AnyObject,
    _: Sel,
    webview: *mut AnyObject,
    hit: *mut AnyObject,
    _: usize,
    _: *mut AnyObject,
) {
    if !watched(webview) {
        return;
    }
    // SAFETY: WebKit passes the `_WKHitTestResult` under the pointer.
    let link = unsafe { hit.as_ref().and_then(|hit| url_text(msg_send![hit, absoluteLinkURL])) };
    let before = HOVERED.with_borrow_mut(|hovered| match &link {
        Some(link) => hovered.insert(key(webview), link.clone()),
        None => hovered.remove(&key(webview)),
    });
    if before != link {
        tell(webview, Change::Hovered { link });
    }
}

/// WebKit's Open Link in New Window becomes Open Link in New Tab, which opens the link under the pointer behind.
unsafe extern "C-unwind" fn context_menu(
    _: &AnyObject,
    _: Sel,
    webview: *mut AnyObject,
    menu: *mut AnyObject,
    _: *mut AnyObject,
) -> *mut AnyObject {
    let link = HOVERED.with_borrow(|hovered| hovered.get(&key(webview)).cloned());
    // SAFETY: WebKit passes its webview and the menu it proposes, whose items it made.
    if let (Some(link), Some(view), Some(proposed)) = (link, unsafe { webview.as_ref() }, unsafe { menu.as_ref() }) {
        unsafe {
            let count: isize = msg_send![proposed, numberOfItems];
            for index in 0..count {
                let item: *mut AnyObject = msg_send![proposed, itemAtIndex: index];
                let item = &*item;
                if text(msg_send![item, identifier]).as_deref() == Some(OPEN_LINK_IN_NEW_WINDOW) {
                    let title = NSString::from_str("Open Link in New Tab");
                    let link = NSString::from_str(&link);
                    let _: () = msg_send![item, setTitle: &*title];
                    let _: () = msg_send![item, setTarget: view];
                    let _: () = msg_send![item, setAction: sel!(octopusOpenLinkInNewTab:)];
                    let _: () = msg_send![item, setRepresentedObject: &*link];
                }
            }
        }
    }
    menu
}

unsafe extern "C-unwind" fn open_link_in_new_tab(webview: &AnyObject, _: Sel, item: *mut AnyObject) {
    // SAFETY: the item `context_menu` set up, which holds its link.
    let link = unsafe { item.as_ref().and_then(|item| text(msg_send![item, representedObject])) };
    if let Some(url) = link {
        tell(webview, Change::Opened { url, select: false });
    }
}

fn add(class: &AnyClass, methods: &[(Sel, Imp, &CStr)]) {
    let class = std::ptr::from_ref::<AnyClass>(class).cast_mut();
    for &(selector, method, types) in methods {
        // SAFETY: the class is one of wry's live classes, and the types encode the signature.
        let added: Bool = unsafe { objc2::ffi::class_addMethod(class, selector, method, types.as_ptr()) };
        assert!(added.as_bool(), "wry leaves {selector} to octopus");
    }
}

/// Gives wry's webview, navigation delegate and UI delegate classes octopus's methods, once.
fn teach(view: &AnyObject, navigation: &AnyObject, ui: &AnyObject) {
    static TAUGHT: Once = Once::new();
    TAUGHT.call_once(|| {
        let (observe, fail, over, menu, open): (Observed, FailedLoad, MouseOver, ContextMenu, MenuAction) =
            (observed, failed_load, mouse_over, context_menu, open_link_in_new_tab);
        // SAFETY: an `Imp` is any function pointer, and each signature matches its selector's type encoding.
        let (observe, fail, over, menu, open) = unsafe {
            (
                std::mem::transmute::<Observed, Imp>(observe),
                std::mem::transmute::<FailedLoad, Imp>(fail),
                std::mem::transmute::<MouseOver, Imp>(over),
                std::mem::transmute::<ContextMenu, Imp>(menu),
                std::mem::transmute::<MenuAction, Imp>(open),
            )
        };
        add(
            navigation.class(),
            &[
                (sel!(observeValueForKeyPath:ofObject:change:context:), observe, c"v@:@@@^v"),
                (sel!(webView:didFailProvisionalNavigation:withError:), fail, c"v@:@@@"),
                (sel!(webView:didFailNavigation:withError:), fail, c"v@:@@@"),
            ],
        );
        add(
            ui.class(),
            &[
                (sel!(_webView:mouseDidMoveOverElement:withFlags:userInfo:), over, c"v@:@@Q@"),
                (sel!(_webView:contextMenu:forElement:), menu, c"@@:@@@"),
            ],
        );
        add(view.class(), &[(sel!(octopusOpenLinkInNewTab:), open, c"v@:@")]);
        let policy = navigation
            .class()
            .instance_method(sel!(webView:decidePolicyForNavigationAction:decisionHandler:))
            .expect("wry's navigation delegate decides each navigation's policy");
        let decide: Decide = decide;
        // SAFETY: `decide` has the signature of wry's implementation it takes the place of, which it calls.
        let wry = unsafe { policy.set_implementation(std::mem::transmute::<Decide, Imp>(decide)) };
        // SAFETY: wry's implementation of the same selector.
        let saved = WRY_DECIDE.set(unsafe { std::mem::transmute::<Imp, Decide>(wry) });
        assert!(saved.is_ok(), "octopus takes wry's policy over once");
    });
}

/// Runs `act` on the main thread with the webview's `WKWebView`.
fn with_view(webview: &Webview, act: impl FnOnce(&AnyObject) + Send + 'static) -> tauri::Result<()> {
    webview.with_webview(move |platform| {
        // SAFETY: tauri hands over its live WKWebView.
        act(unsafe { &*platform.inner().cast::<AnyObject>() });
    })
}

/// The webview's navigation delegate and UI delegate, which wry set for its lifetime.
///
/// # Safety
/// `view` is a `WKWebView` wry made.
unsafe fn delegates(view: &AnyObject) -> (&AnyObject, &AnyObject) {
    unsafe {
        let navigation: *mut AnyObject = msg_send![view, navigationDelegate];
        let ui: *mut AnyObject = msg_send![view, UIDelegate];
        (&*navigation, &*ui)
    }
}

/// Tells `watcher` each change of the page the webview shows, on the main thread.
pub fn watch(webview: &Webview, watcher: impl Fn(Change) + Send + 'static) -> tauri::Result<()> {
    with_view(webview, move |view| {
        // SAFETY: wry made the webview.
        let (navigation, ui) = unsafe { delegates(view) };
        teach(view, navigation, ui);
        // SAFETY: setting the same delegates again has WebKit ask anew which methods they answer.
        let () = unsafe { msg_send![view, setNavigationDelegate: navigation] };
        let () = unsafe { msg_send![view, setUIDelegate: ui] };
        let () = unsafe { msg_send![view, setAllowsBackForwardNavigationGestures: true] };
        let () = unsafe { msg_send![view, setAllowsMagnification: true] };
        WATCHERS.with_borrow_mut(|watchers| watchers.insert(key(view), Box::new(watcher)));
        for observed in OBSERVED {
            let observed = NSString::from_str(observed);
            let context = std::ptr::null_mut::<c_void>();
            // SAFETY: the delegate now answers the observation, and `unwatch` removes it before the webview goes.
            let () = unsafe {
                msg_send![view, addObserver: navigation, forKeyPath: &*observed, options: 0_usize, context: context]
            };
        }
    })
}

pub(crate) fn unwatch(webview: &Webview) -> tauri::Result<()> {
    with_view(webview, |view| {
        // SAFETY: wry made the webview.
        let (navigation, _) = unsafe { delegates(view) };
        for observed in OBSERVED {
            let observed = NSString::from_str(observed);
            // SAFETY: `watch` added this observation.
            let () = unsafe { msg_send![view, removeObserver: navigation, forKeyPath: &*observed] };
        }
        WATCHERS.with_borrow_mut(|watchers| watchers.remove(&key(view)));
        HOVERED.with_borrow_mut(|hovered| hovered.remove(&key(view)));
    })
}

pub fn go(webview: &Webview, go: Go) -> tauri::Result<()> {
    with_view(webview, move |view| {
        // SAFETY: WKWebView's own history moves and reloads, which do nothing at either end of the history.
        let _: *mut AnyObject = unsafe {
            match go {
                Go::Back => msg_send![view, goBack],
                Go::Forward => msg_send![view, goForward],
                Go::Reload => msg_send![view, reload],
                Go::ReloadFromOrigin => msg_send![view, reloadFromOrigin],
            }
        };
    })
}

pub fn find(webview: &Webview, text: String, backwards: bool) -> tauri::Result<()> {
    with_view(webview, move |view| {
        let configuration = AnyClass::get(c"WKFindConfiguration").expect("WebKit has WKFindConfiguration");
        let text = NSString::from_str(&text);
        let found = RcBlock::new(|_: *mut AnyObject| {});
        // SAFETY: WKWebView's own find, which selects the match and scrolls to it.
        unsafe {
            let configuration: Retained<AnyObject> = msg_send![configuration, new];
            let () = msg_send![&*configuration, setBackwards: backwards];
            let () = msg_send![&*configuration, setWraps: true];
            let () = msg_send![&*configuration, setCaseSensitive: false];
            let () =
                msg_send![view, findString: &*text, withConfiguration: &*configuration, completionHandler: &*found];
        }
    })
}

/// Runs on the main thread, where AppKit tells which view holds the keyboard.
pub(crate) fn has_focus(window: &tauri::Window) -> bool {
    window.ns_window().is_ok_and(|window| {
        // SAFETY: tauri hands over its live NSWindow.
        let responder: *mut AnyObject = unsafe { msg_send![&*window.cast::<AnyObject>(), firstResponder] };
        watched(responder)
    })
}
