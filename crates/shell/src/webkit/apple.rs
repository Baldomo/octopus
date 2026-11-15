//! WebKit tells its embedder what a page shows through key-value observing of the `WKWebView`, and a failed load
//! through its navigation delegate. On macOS tauri makes the view, whose delegate classes wry declares without the
//! failure methods; octopus adds them, makes the navigation delegate the view's observer, takes the navigation policy
//! over from wry so a link clicked with ⌘ or the middle button opens a tab, and adds WebKit's private methods for the
//! link under the pointer and the context menu to the UI delegate. WebKit notes which methods a delegate answers when
//! it is set, so a view set its delegates before the methods existed has them set again. On iOS octopus makes the
//! view, with one delegate of its own for every page, which also leads a link's menu with Open in New Tab and saves
//! each download to the embedder's folder.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_void};
#[cfg(any(target_os = "ios", test))]
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[cfg(target_os = "macos")]
use block2::RcBlock;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2_foundation::NSString;

#[cfg(target_os = "macos")]
use super::DesktopChange as DeviceChange;
#[cfg(target_os = "ios")]
use super::PhoneChange as DeviceChange;
use super::{Change, Go, Told};

type Watcher = Rc<dyn Fn(Told<DeviceChange>)>;

thread_local! {
    /// Each watched view's watcher, by the address of its `WKWebView`.
    static WATCHERS: RefCell<HashMap<usize, Watcher>> = RefCell::new(HashMap::new());
}

const OBSERVED: [&str; 5] = ["URL", "loading", "canGoBack", "canGoForward", "title"];

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

fn key(view: *const AnyObject) -> usize {
    view as usize
}

/// The watcher runs with no view borrowed, so it may watch or stop watching one.
fn tell(view: *const AnyObject, told: impl Into<Told<DeviceChange>>) {
    let watcher = WATCHERS.with_borrow(|watchers| watchers.get(&key(view)).cloned());
    if let Some(watcher) = watcher {
        watcher(told.into());
    }
}

unsafe extern "C-unwind" fn observed(
    _: &AnyObject,
    _: Sel,
    path: *mut AnyObject,
    view: *mut AnyObject,
    _: *mut AnyObject,
    _: *mut c_void,
) {
    // SAFETY: the observed object is the WKWebView `watch` observes, at one of the key paths it names.
    let Some(shown) = (unsafe { view.as_ref() }) else { return };
    let change = unsafe {
        if text(path).as_deref() == Some("title") {
            Change::Titled(text(msg_send![shown, title]).unwrap_or_default())
        } else {
            Change::Seen {
                url: url_text(msg_send![shown, URL]),
                loading: msg_send![shown, isLoading],
                back: msg_send![shown, canGoBack],
                forward: msg_send![shown, canGoForward],
            }
        }
    };
    tell(view, change);
}

unsafe extern "C-unwind" fn failed_load(
    _: &AnyObject,
    _: Sel,
    view: *mut AnyObject,
    _: *mut AnyObject,
    error: *mut AnyObject,
) {
    // SAFETY: WebKit passes the NSError the load failed with.
    let Some(error) = (unsafe { error.as_ref() }) else { return };
    let (domain, code, url, description) = unsafe {
        let domain = text(msg_send![error, domain]).unwrap_or_default();
        let code: isize = msg_send![error, code];
        let info: *mut AnyObject = msg_send![error, userInfo];
        let key = NSString::from_str("NSErrorFailingURLStringKey");
        let url = info.as_ref().and_then(|info| text(msg_send![info, objectForKey: &*key]));
        (domain, code, url, text(msg_send![error, localizedDescription]))
    };
    if !on_purpose(&domain, code) {
        let error = description.unwrap_or_else(|| format!("error {code}"));
        tell(view, Change::Failed { url: url.unwrap_or_default(), error });
    }
}

/// Whether WebKit ended the load on purpose: a navigation another replaced, a load its policy stopped, such as a
/// download, or a media file it plays in a media document of its own.
fn on_purpose(domain: &str, code: isize) -> bool {
    matches!((domain, code), ("NSURLErrorDomain", -999) | ("WebKitErrorDomain", 102 | 204))
}

/// `name` in `folder`, or, while `taken` says a file has it, the name with the lowest number from 2 after its stem.
#[cfg(any(target_os = "ios", test))]
fn unclaimed(folder: &Path, name: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let named = Path::new(name);
    let stem = named.file_stem().map_or_else(|| name.into(), |stem| stem.to_string_lossy());
    let extension = named.extension().map(|extension| format!(".{}", extension.to_string_lossy()));
    std::iter::once(name.to_string())
        .chain((2..).map(|nth| format!("{stem} {nth}{}", extension.as_deref().unwrap_or_default())))
        .map(|name| folder.join(name))
        .find(|path| !taken(path))
        .expect("some number is free")
}

/// A live `WKWebView`, on the main thread, which owns every view.
pub struct View<'a>(&'a AnyObject);

impl View<'_> {
    /// Tells `watcher` each change of the page the view shows.
    pub fn watch(self, watcher: impl Fn(Told<DeviceChange>) + 'static) {
        let view = self.0;
        // SAFETY: a WKWebView's navigation delegate, which outlives the watch: wry's for the view's lifetime on macOS,
        // and octopus's own for the app's on iOS.
        let navigation: *mut AnyObject = unsafe { msg_send![view, navigationDelegate] };
        let navigation = unsafe { navigation.as_ref() }.expect("a watched view has its navigation delegate");
        #[cfg(target_os = "macos")]
        mac::teach(view, navigation);
        WATCHERS.with_borrow_mut(|watchers| watchers.insert(key(view), Rc::new(watcher)));
        for observed in OBSERVED {
            let observed = NSString::from_str(observed);
            let context = std::ptr::null_mut::<c_void>();
            // SAFETY: the delegate answers the observation, and `unwatch` removes it before the view goes.
            let () = unsafe {
                msg_send![view, addObserver: navigation, forKeyPath: &*observed, options: 0_usize, context: context]
            };
        }
    }

    pub fn unwatch(self) {
        let view = self.0;
        WATCHERS.with_borrow_mut(|watchers| watchers.remove(&key(view)));
        // SAFETY: the delegate `watch` made the view's observer.
        let navigation: *mut AnyObject = unsafe { msg_send![view, navigationDelegate] };
        for observed in OBSERVED {
            let observed = NSString::from_str(observed);
            // SAFETY: `watch` added this observation.
            let () = unsafe { msg_send![view, removeObserver: navigation, forKeyPath: &*observed] };
        }
        #[cfg(target_os = "macos")]
        mac::forget(view);
    }

    pub fn go(self, go: Go) {
        let view = self.0;
        // SAFETY: WKWebView's own history moves and reloads, which do nothing at either end of the history.
        let _: *mut AnyObject = unsafe {
            match go {
                Go::Back => msg_send![view, goBack],
                Go::Forward => msg_send![view, goForward],
                Go::Reload => msg_send![view, reload],
                Go::ReloadFromOrigin => msg_send![view, reloadFromOrigin],
            }
        };
    }

    /// Selects the next match of `text` in the page, or the one before when `backwards`, wrapping around its ends.
    pub fn find(self, text: &str, backwards: bool) {
        let configuration = objc2::class!(WKFindConfiguration);
        let text = NSString::from_str(text);
        #[cfg(target_os = "macos")]
        let found = RcBlock::new(|_: *mut AnyObject| {});
        #[cfg(target_os = "ios")]
        let found = phone::reveal_match(self.0);
        // SAFETY: WKWebView's own find, which selects the match and scrolls to it.
        unsafe {
            let configuration: Retained<AnyObject> = msg_send![configuration, new];
            let () = msg_send![&*configuration, setBackwards: backwards];
            let () = msg_send![&*configuration, setWraps: true];
            let () = msg_send![&*configuration, setCaseSensitive: false];
            let () =
                msg_send![self.0, findString: &*text, withConfiguration: &*configuration, completionHandler: &*found];
        }
    }
}

#[cfg(target_os = "macos")]
pub use mac::{find, go, has_focus, unwatch, watch};

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::CStr;
    use std::sync::{Once, OnceLock};

    use block2::Block;
    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
    use objc2::{msg_send, sel};
    use objc2_foundation::NSString;
    use tauri::Webview;

    use super::{Change, DeviceChange, Go, Told, View, WATCHERS, failed_load, key, observed, tell, text, url_text};

    thread_local! {
        /// The link under the pointer in each watched view that has one.
        static HOVERED: RefCell<HashMap<usize, String>> = RefCell::new(HashMap::new());
    }

    fn watched(view: *const AnyObject) -> bool {
        WATCHERS.with_borrow(|watchers| watchers.contains_key(&key(view)))
    }

    const LINK_ACTIVATED: isize = 0;
    const CANCEL: isize = 0;
    const SHIFT: usize = 1 << 17;
    const COMMAND: usize = 1 << 20;
    /// WebKit names a navigation's mouse button by its bit among the pressed buttons.
    const MIDDLE_BUTTON: isize = 1 << 2;
    const OPEN_LINK_IN_NEW_WINDOW: &str = "WKMenuItemIdentifierOpenLinkInNewWindow";

    type Decide = unsafe extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject, *mut AnyObject, &Block<dyn Fn(isize)>);

    static WRY_DECIDE: OnceLock<Decide> = OnceLock::new();

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
        view: *mut AnyObject,
        action: *mut AnyObject,
        decided: &Block<dyn Fn(isize)>,
    ) {
        // SAFETY: WebKit passes the navigation action it asks about.
        let opened = unsafe { action.as_ref() }.filter(|_| watched(view)).and_then(|action| unsafe { new_tab(action) });
        match opened {
            Some(opened) => {
                decided.call((CANCEL,));
                tell(view, opened);
            }
            None => {
                let wry = WRY_DECIDE.get().expect("octopus takes wry's policy over after saving it");
                // SAFETY: wry's own implementation, called with the arguments WebKit passed.
                unsafe { wry(delegate, selector, view, action, decided) }
            }
        }
    }

    unsafe extern "C-unwind" fn mouse_over(
        _: &AnyObject,
        _: Sel,
        view: *mut AnyObject,
        hit: *mut AnyObject,
        _: usize,
        _: *mut AnyObject,
    ) {
        if !watched(view) {
            return;
        }
        // SAFETY: WebKit passes the `_WKHitTestResult` under the pointer.
        let link = unsafe { hit.as_ref().and_then(|hit| url_text(msg_send![hit, absoluteLinkURL])) };
        let before = HOVERED.with_borrow_mut(|hovered| match &link {
            Some(link) => hovered.insert(key(view), link.clone()),
            None => hovered.remove(&key(view)),
        });
        if before != link {
            tell(view, Told::Device(DeviceChange::Hovered { link }));
        }
    }

    /// WebKit's Open Link in New Window becomes Open Link in New Tab, which opens the link under the pointer behind.
    unsafe extern "C-unwind" fn context_menu(
        _: &AnyObject,
        _: Sel,
        view: *mut AnyObject,
        menu: *mut AnyObject,
        _: *mut AnyObject,
    ) -> *mut AnyObject {
        let link = HOVERED.with_borrow(|hovered| hovered.get(&key(view)).cloned());
        // SAFETY: WebKit passes its view and the menu it proposes, whose items it made.
        if let (Some(link), Some(shown), Some(proposed)) = (link, unsafe { view.as_ref() }, unsafe { menu.as_ref() }) {
            unsafe {
                let count: isize = msg_send![proposed, numberOfItems];
                for index in 0..count {
                    let item: *mut AnyObject = msg_send![proposed, itemAtIndex: index];
                    let item = &*item;
                    if text(msg_send![item, identifier]).as_deref() == Some(OPEN_LINK_IN_NEW_WINDOW) {
                        let title = NSString::from_str("Open Link in New Tab");
                        let link = NSString::from_str(&link);
                        let _: () = msg_send![item, setTitle: &*title];
                        let _: () = msg_send![item, setTarget: shown];
                        let _: () = msg_send![item, setAction: sel!(octopusOpenLinkInNewTab:)];
                        let _: () = msg_send![item, setRepresentedObject: &*link];
                    }
                }
            }
        }
        menu
    }

    unsafe extern "C-unwind" fn open_link_in_new_tab(view: &AnyObject, _: Sel, item: *mut AnyObject) {
        // SAFETY: the item `context_menu` set up, which holds its link.
        let link = unsafe { item.as_ref().and_then(|item| text(msg_send![item, representedObject])) };
        if let Some(url) = link {
            tell(view, Change::Opened { url, select: false });
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

    /// Gives wry's view, navigation delegate and UI delegate classes octopus's methods, once, and has WebKit ask this
    /// view's delegates anew which methods they answer.
    pub(super) fn teach(view: &AnyObject, navigation: &AnyObject) {
        // SAFETY: wry set the view's UI delegate for its lifetime.
        let ui: *mut AnyObject = unsafe { msg_send![view, UIDelegate] };
        let ui = unsafe { ui.as_ref() }.expect("wry gives a view its UI delegate");
        static TAUGHT: Once = Once::new();
        TAUGHT.call_once(|| {
            // SAFETY: an `Imp` is any function pointer, and each signature matches its selector's type encoding.
            let [observe, fail, over, menu, open] = unsafe {
                [
                    std::mem::transmute::<unsafe extern "C-unwind" fn(_, _, _, _, _, _), Imp>(observed),
                    std::mem::transmute::<unsafe extern "C-unwind" fn(_, _, _, _, _), Imp>(failed_load),
                    std::mem::transmute::<unsafe extern "C-unwind" fn(_, _, _, _, _, _), Imp>(mouse_over),
                    std::mem::transmute::<unsafe extern "C-unwind" fn(_, _, _, _, _) -> _, Imp>(context_menu),
                    std::mem::transmute::<unsafe extern "C-unwind" fn(_, _, _), Imp>(open_link_in_new_tab),
                ]
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
        // SAFETY: setting the same delegates again has WebKit ask anew which methods they answer.
        let () = unsafe { msg_send![view, setNavigationDelegate: navigation] };
        let () = unsafe { msg_send![view, setUIDelegate: ui] };
        let () = unsafe { msg_send![view, setAllowsBackForwardNavigationGestures: true] };
        let () = unsafe { msg_send![view, setAllowsMagnification: true] };
    }

    pub(super) fn forget(view: &AnyObject) {
        HOVERED.with_borrow_mut(|hovered| hovered.remove(&key(view)));
    }

    /// Runs `act` on the main thread with the webview's `WKWebView`.
    fn with_view(webview: &Webview, act: impl FnOnce(View<'_>) + Send + 'static) -> tauri::Result<()> {
        webview.with_webview(move |platform| {
            // SAFETY: tauri hands over its live WKWebView.
            act(View(unsafe { &*platform.inner().cast::<AnyObject>() }));
        })
    }

    pub fn watch(webview: &Webview, watcher: impl Fn(Told<DeviceChange>) + Send + 'static) -> tauri::Result<()> {
        with_view(webview, move |view| view.watch(watcher))
    }

    pub fn unwatch(webview: &Webview) -> tauri::Result<()> {
        with_view(webview, |view| view.unwatch())
    }

    pub fn go(webview: &Webview, go: Go) -> tauri::Result<()> {
        with_view(webview, move |view| view.go(go))
    }

    pub fn find(webview: &Webview, text: String, backwards: bool) -> tauri::Result<()> {
        with_view(webview, move |view| view.find(&text, backwards))
    }

    /// Whether a watched view holds the keyboard. Runs on the main thread, where AppKit tells which view does.
    pub fn has_focus(window: &tauri::Window) -> bool {
        window.ns_window().is_ok_and(|window| {
            // SAFETY: tauri hands over its live NSWindow.
            let responder: *mut AnyObject = unsafe { msg_send![&*window.cast::<AnyObject>(), firstResponder] };
            watched(responder)
        })
    }
}

#[cfg(target_os = "ios")]
pub use phone::{Asked, Embedder, Page, answer};

#[cfg(target_os = "ios")]
mod phone {
    use std::cell::{Cell, OnceCell, RefCell};
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use block2::{Block, DynBlock, RcBlock};
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyObject, Bool, ClassBuilder, NSObject, Sel};
    use objc2::{ClassType, class, msg_send, sel};
    use objc2_core_foundation::{CGPoint, CGRect};
    use objc2_foundation::NSString;
    use tauri::http::Response;

    use super::{Change, View, WATCHERS, Watcher, failed_load, key, observed, tell, text, url_text};
    use crate::webkit::{Drag, PhoneChange, Told};

    const CANCEL: isize = 0;
    const ALLOW: isize = 1;
    const DOWNLOAD: isize = 2;
    const LEFT_EDGE: usize = 1 << 1;
    /// The states of a `UIGestureRecognizer` that recognizes a drag.
    const BEGAN: isize = 1;
    const CHANGED: isize = 2;
    const ENDED: isize = 3;
    const CANCELLED: isize = 4;
    const FAILED: isize = 5;

    /// A request a page makes for a file of `served_files`, answered through `answer`.
    pub struct Asked {
        pub id: u64,
        pub url: String,
        pub method: String,
        pub range: Option<String>,
    }

    /// What the app that embeds a page decides for it.
    pub struct Embedder {
        /// Whether the page may navigate to a URL, asked before each navigation of its main frame goes on.
        pub allows: Box<dyn Fn(&str) -> bool>,
        pub serve: Box<dyn Fn(Asked)>,
        /// The folder a download lands in, under a name no other file there has.
        pub folder: PathBuf,
        /// The folder's name as the user finds it.
        pub folder_label: String,
    }

    /// A download under way: the embedder and the watcher of the page that started it, held so its end is told once
    /// the page is gone, and the name of its file, its URL's last segment until WebKit asks where it goes.
    struct Downloading {
        _download: Retained<AnyObject>,
        embedder: Rc<Embedder>,
        watcher: Watcher,
        file: String,
    }

    thread_local! {
        /// Each page's embedder, by the address of its `WKWebView`.
        static EMBEDDERS: RefCell<HashMap<usize, Rc<Embedder>>> = RefCell::new(HashMap::new());
        /// The scheme tasks WebKit waits on, by the id their request was asked under.
        static TASKS: RefCell<HashMap<u64, Retained<AnyObject>>> = RefCell::new(HashMap::new());
        static NEXT_TASK: Cell<u64> = const { Cell::new(0) };
        /// Each download under way, by the address of its `WKDownload`.
        static IN_FLIGHT: RefCell<HashMap<usize, Downloading>> = RefCell::new(HashMap::new());
        /// The one object that is every page's navigation delegate, UI delegate, download delegate, scheme handler
        /// and edge drag target.
        static DELEGATE: OnceCell<Retained<AnyObject>> = const { OnceCell::new() };
    }

    fn embedder(view: *const AnyObject) -> Option<Rc<Embedder>> {
        EMBEDDERS.with_borrow(|embedders| embedders.get(&key(view)).cloned())
    }

    unsafe extern "C-unwind" fn decide(
        _: &AnyObject,
        _: Sel,
        view: *mut AnyObject,
        action: *mut AnyObject,
        decided: &Block<dyn Fn(isize)>,
    ) {
        // SAFETY: WebKit passes the navigation action it asks about, whose request names its URL.
        let (url, main, download) = unsafe {
            let request: *mut AnyObject = msg_send![action, request];
            let frame: *mut AnyObject = msg_send![action, targetFrame];
            (
                request.as_ref().and_then(|request| url_text(msg_send![request, URL])),
                frame.as_ref().is_some_and(|frame| msg_send![frame, isMainFrame]),
                msg_send![action, shouldPerformDownload],
            )
        };
        // A download leaves the page where it is, so it goes on whatever its scheme, as a `blob:` export does.
        let policy = match (download, main, url, embedder(view)) {
            (true, ..) => DOWNLOAD,
            (false, false, ..) => ALLOW,
            (false, true, Some(url), Some(embedder)) if (embedder.allows)(&url) => ALLOW,
            _ => CANCEL,
        };
        decided.call((policy,));
    }

    /// A response WebKit cannot show, or one its server sends as an attachment, downloads.
    unsafe extern "C-unwind" fn decide_response(
        _: &AnyObject,
        _: Sel,
        _: *mut AnyObject,
        navigation: *mut AnyObject,
        decided: &Block<dyn Fn(isize)>,
    ) {
        // SAFETY: WebKit passes the navigation response it asks about, whose response is an NSURLResponse.
        let download = unsafe {
            let shows: bool = msg_send![navigation, canShowMIMEType];
            let response: *mut AnyObject = msg_send![navigation, response];
            let http: bool = msg_send![response, isKindOfClass: class!(NSHTTPURLResponse)];
            let field = NSString::from_str("Content-Disposition");
            let disposition = if http { text(msg_send![response, valueForHTTPHeaderField: &*field]) } else { None };
            !shows || disposition.is_some_and(|disposition| disposition.trim().to_lowercase().starts_with("attachment"))
        };
        decided.call((if download { DOWNLOAD } else { ALLOW },));
    }

    unsafe extern "C-unwind" fn became_download(
        delegate: &AnyObject,
        _: Sel,
        view: *mut AnyObject,
        _: *mut AnyObject,
        download: *mut AnyObject,
    ) {
        // SAFETY: WebKit passes the live download a navigation became, whose request names its URL.
        let Some(retained) = (unsafe { Retained::retain(download) }) else { return };
        let file = unsafe {
            let () = msg_send![&*retained, setDelegate: delegate];
            let request: *mut AnyObject = msg_send![&*retained, originalRequest];
            let url: *mut AnyObject = msg_send![request, URL];
            text(msg_send![url, lastPathComponent]).unwrap_or_default()
        };
        let watcher = WATCHERS.with_borrow(|watchers| watchers.get(&key(view)).cloned());
        let (Some(embedder), Some(watcher)) = (embedder(view), watcher) else { return };
        let downloading = Downloading { _download: retained, embedder, watcher, file };
        IN_FLIGHT.with_borrow_mut(|in_flight| in_flight.insert(key(download), downloading));
    }

    /// WebKit saves a download to the embedder's folder, under the name it suggests unless a file there has it.
    unsafe extern "C-unwind" fn download_destination(
        _: &AnyObject,
        _: Sel,
        download: *mut AnyObject,
        _: *mut AnyObject,
        suggested: *mut AnyObject,
        decided: &Block<dyn Fn(*mut AnyObject)>,
    ) {
        // SAFETY: WebKit passes the name it suggests as an NSString.
        let suggested = unsafe { text(suggested) }.unwrap_or_default();
        let path = IN_FLIGHT.with_borrow_mut(|in_flight| {
            let downloading = in_flight.get_mut(&key(download))?;
            let path = super::unclaimed(&downloading.embedder.folder, &suggested, Path::exists);
            downloading.file = path.file_name()?.to_string_lossy().into_owned();
            Some(path)
        });
        let Some(path) = path else { return decided.call((std::ptr::null_mut(),)) };
        // SAFETY: a file URL for an absolute path, which the handler takes as the download's destination.
        unsafe {
            let path = NSString::from_str(&path.to_string_lossy());
            let url: Retained<AnyObject> = msg_send![class!(NSURL), fileURLWithPath: &*path];
            decided.call((Retained::as_ptr(&url).cast_mut(),));
        }
    }

    unsafe extern "C-unwind" fn download_finished(_: &AnyObject, _: Sel, download: *mut AnyObject) {
        download_ended(download, true);
    }

    unsafe extern "C-unwind" fn download_failed(
        _: &AnyObject,
        _: Sel,
        download: *mut AnyObject,
        _: *mut AnyObject,
        _: *mut AnyObject,
    ) {
        download_ended(download, false);
    }

    fn download_ended(download: *mut AnyObject, success: bool) {
        let ended = IN_FLIGHT.with_borrow_mut(|in_flight| in_flight.remove(&key(download)));
        let Some(Downloading { embedder, watcher, file, .. }) = ended else { return };
        watcher(Change::Downloaded { file, folder: embedder.folder_label.clone(), success }.into());
    }

    /// A held element's menu holds WebKit's own items, a link's led by Open in New Tab, which opens it behind the
    /// selected tab.
    unsafe extern "C-unwind" fn held_menu(
        _: &AnyObject,
        _: Sel,
        view: *mut AnyObject,
        element: *mut AnyObject,
        decided: &Block<dyn Fn(*mut AnyObject)>,
    ) {
        // SAFETY: WebKit passes the `WKContextMenuElementInfo` the finger holds.
        let link = unsafe { element.as_ref().and_then(|element| url_text(msg_send![element, linkURL])) };
        let view = view.cast_const();
        let items = RcBlock::new(move |suggested: *mut AnyObject| -> *mut AnyObject {
            // SAFETY: UIKit's own menu of UIKit's actions, which the menu's caller takes autoreleased.
            unsafe {
                let children: Retained<AnyObject> = msg_send![class!(NSMutableArray), arrayWithArray: suggested];
                if let Some(link) = link.clone() {
                    let open = RcBlock::new(move |_: *mut AnyObject| {
                        tell(view, Change::Opened { url: link.clone(), select: false })
                    });
                    let title = NSString::from_str("Open in New Tab");
                    let symbol = NSString::from_str("plus.square.on.square");
                    let image: *mut AnyObject = msg_send![class!(UIImage), systemImageNamed: &*symbol];
                    let unnamed = std::ptr::null_mut::<AnyObject>();
                    let action: *mut AnyObject = msg_send![class!(UIAction), actionWithTitle: &*title, image: image, identifier: unnamed, handler: &*open];
                    let () = msg_send![&*children, insertObject: action, atIndex: 0_usize];
                }
                let untitled = NSString::from_str("");
                let menu: Retained<AnyObject> =
                    msg_send![class!(UIMenu), menuWithTitle: &*untitled, children: &*children];
                Retained::autorelease_return(menu)
            }
        });
        // SAFETY: a configuration whose preview is the held element itself, and whose menu `items` makes.
        unsafe {
            let unnamed = std::ptr::null_mut::<AnyObject>();
            let preview: Option<&DynBlock<dyn Fn() -> *mut AnyObject>> = None;
            let configuration: *mut AnyObject = msg_send![
                class!(UIContextMenuConfiguration),
                configurationWithIdentifier: unnamed,
                previewProvider: preview,
                actionProvider: &*items
            ];
            decided.call((configuration,));
        }
    }

    /// A link that targets a new window, or `window.open`, opens a tab in place of the window.
    unsafe extern "C-unwind" fn create_window(
        _: &AnyObject,
        _: Sel,
        view: *mut AnyObject,
        _: *mut AnyObject,
        action: *mut AnyObject,
        _: *mut AnyObject,
    ) -> *mut AnyObject {
        // SAFETY: WebKit passes the navigation action that asks for the window.
        let url = unsafe {
            let request: *mut AnyObject = msg_send![action, request];
            request.as_ref().and_then(|request| url_text(msg_send![request, URL]))
        };
        if let Some(url) = url {
            tell(view, Change::Opened { url, select: true });
        }
        std::ptr::null_mut()
    }

    unsafe extern "C-unwind" fn process_ended(_: &AnyObject, _: Sel, view: *mut AnyObject) {
        tell(view, Told::Device(PhoneChange::Ended));
    }

    unsafe extern "C-unwind" fn start_task(_: &AnyObject, _: Sel, view: *mut AnyObject, task: *mut AnyObject) {
        // SAFETY: WebKit passes the live task of a request for the embedder's scheme.
        let Some(task) = (unsafe { Retained::retain(task) }) else { return };
        let (url, method, range) = unsafe {
            let request: *mut AnyObject = msg_send![&*task, request];
            let range = NSString::from_str("Range");
            (
                url_text(msg_send![request, URL]).unwrap_or_default(),
                text(msg_send![request, HTTPMethod]).unwrap_or_default(),
                text(msg_send![request, valueForHTTPHeaderField: &*range]),
            )
        };
        let Some(embedder) = embedder(view) else { return };
        let id = NEXT_TASK.replace(NEXT_TASK.get() + 1);
        TASKS.with_borrow_mut(|tasks| tasks.insert(id, task));
        (embedder.serve)(Asked { id, url, method, range });
    }

    /// WebKit raises an exception on an answer to a task it stopped, so a stopped task is forgotten.
    unsafe extern "C-unwind" fn stop_task(_: &AnyObject, _: Sel, _: *mut AnyObject, task: *mut AnyObject) {
        TASKS.with_borrow_mut(|tasks| tasks.retain(|_, waiting| !std::ptr::eq(Retained::as_ptr(waiting), task)));
    }

    /// Answers the request asked under `id`, unless WebKit stopped waiting for it. Runs on the main thread.
    pub fn answer(id: u64, reply: Response<Vec<u8>>) {
        let Some(task) = TASKS.with_borrow_mut(|tasks| tasks.remove(&id)) else { return };
        // SAFETY: the task is live and unanswered, and each call is made as `WKURLSchemeTask` documents.
        unsafe {
            let fields: Retained<AnyObject> = msg_send![class!(NSMutableDictionary), new];
            for (name, value) in reply.headers() {
                let (name, value) = (NSString::from_str(name.as_str()), value.to_str().map(NSString::from_str));
                if let Ok(value) = value {
                    let () = msg_send![&*fields, setObject: &*value, forKey: &*name];
                }
            }
            let request: *mut AnyObject = msg_send![&*task, request];
            let url: *mut AnyObject = msg_send![request, URL];
            let response: Allocated<AnyObject> = msg_send![class!(NSHTTPURLResponse), alloc];
            let response: Retained<AnyObject> = msg_send![
                response,
                initWithURL: url,
                statusCode: isize::from(i16::try_from(reply.status().as_u16()).expect("an HTTP status fits an i16")),
                HTTPVersion: &*NSString::from_str("HTTP/1.1"),
                headerFields: &*fields
            ];
            let body = reply.body();
            let data: Retained<AnyObject> =
                msg_send![class!(NSData), dataWithBytes: body.as_ptr().cast::<c_void>(), length: body.len()];
            let () = msg_send![&*task, didReceiveResponse: &*response];
            let () = msg_send![&*task, didReceiveData: &*data];
            let () = msg_send![&*task, didFinish];
        }
    }

    /// The edge drag is the screen's while the page's history has nowhere to go back to, and WebKit's own otherwise.
    unsafe extern "C-unwind" fn edge_drag_begins(_: &AnyObject, _: Sel, recognizer: *mut AnyObject) -> Bool {
        // SAFETY: the recognizer `Page::new` added to a WKWebView.
        let back: bool = unsafe {
            let view: *mut AnyObject = msg_send![recognizer, view];
            msg_send![view, canGoBack]
        };
        Bool::new(!back)
    }

    unsafe extern "C-unwind" fn edge_dragged(_: &AnyObject, _: Sel, recognizer: *mut AnyObject) {
        // SAFETY: the recognizer `Page::new` added to a WKWebView, which measures the drag in its own points.
        let (state, view, moved, speed): (isize, *mut AnyObject, CGPoint, CGPoint) = unsafe {
            let view: *mut AnyObject = msg_send![recognizer, view];
            (
                msg_send![recognizer, state],
                view,
                msg_send![recognizer, translationInView: view],
                msg_send![recognizer, velocityInView: view],
            )
        };
        let drag = match state {
            BEGAN => Drag::Start,
            CHANGED => Drag::Move,
            ENDED => Drag::End,
            CANCELLED | FAILED => Drag::Cancel,
            _ => return,
        };
        tell(view, Told::Device(PhoneChange::EdgeDragged { drag, x: moved.x, velocity: speed.x }));
    }

    fn delegate() -> Retained<AnyObject> {
        DELEGATE.with(|delegate| {
            delegate
                .get_or_init(|| {
                    let mut class = ClassBuilder::new(c"OctopusPageDelegate", NSObject::class())
                        .expect("no other class is named OctopusPageDelegate");
                    // SAFETY: each function has the signature its selector is called with.
                    unsafe {
                        class.add_method(
                            sel!(observeValueForKeyPath:ofObject:change:context:),
                            observed as unsafe extern "C-unwind" fn(_, _, _, _, _, _),
                        );
                        let failed = failed_load as unsafe extern "C-unwind" fn(_, _, _, _, _);
                        class.add_method(sel!(webView:didFailProvisionalNavigation:withError:), failed);
                        class.add_method(sel!(webView:didFailNavigation:withError:), failed);
                        class.add_method(
                            sel!(webView:decidePolicyForNavigationAction:decisionHandler:),
                            decide as unsafe extern "C-unwind" fn(_, _, _, _, _),
                        );
                        class.add_method(
                            sel!(webView:decidePolicyForNavigationResponse:decisionHandler:),
                            decide_response as unsafe extern "C-unwind" fn(_, _, _, _, _),
                        );
                        let became = became_download as unsafe extern "C-unwind" fn(_, _, _, _, _);
                        class.add_method(sel!(webView:navigationAction:didBecomeDownload:), became);
                        class.add_method(sel!(webView:navigationResponse:didBecomeDownload:), became);
                        class.add_method(
                            sel!(download:decideDestinationUsingResponse:suggestedFilename:completionHandler:),
                            download_destination as unsafe extern "C-unwind" fn(_, _, _, _, _, _),
                        );
                        class.add_method(
                            sel!(downloadDidFinish:),
                            download_finished as unsafe extern "C-unwind" fn(_, _, _),
                        );
                        class.add_method(
                            sel!(download:didFailWithError:resumeData:),
                            download_failed as unsafe extern "C-unwind" fn(_, _, _, _, _),
                        );
                        class.add_method(
                            sel!(webView:contextMenuConfigurationForElement:completionHandler:),
                            held_menu as unsafe extern "C-unwind" fn(_, _, _, _, _),
                        );
                        class.add_method(
                            sel!(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:),
                            create_window as unsafe extern "C-unwind" fn(_, _, _, _, _, _) -> _,
                        );
                        class.add_method(
                            sel!(webViewWebContentProcessDidTerminate:),
                            process_ended as unsafe extern "C-unwind" fn(_, _, _),
                        );
                        let task = start_task as unsafe extern "C-unwind" fn(_, _, _, _);
                        class.add_method(sel!(webView:startURLSchemeTask:), task);
                        let task = stop_task as unsafe extern "C-unwind" fn(_, _, _, _);
                        class.add_method(sel!(webView:stopURLSchemeTask:), task);
                        class.add_method(
                            sel!(gestureRecognizerShouldBegin:),
                            edge_drag_begins as unsafe extern "C-unwind" fn(_, _, _) -> _,
                        );
                        class.add_method(sel!(edgeDragged:), edge_dragged as unsafe extern "C-unwind" fn(_, _, _));
                    }
                    // SAFETY: the class is an NSObject with no state of its own.
                    unsafe { msg_send![class.register(), new] }
                })
                .clone()
        })
    }

    /// On iOS WebKit selects a match where it lies, so the page scrolls to one out of view itself.
    const REVEAL_SELECTION: &str = "(() => { const found = getSelection(); if (found.rangeCount === 0) return; \
        const box = found.getRangeAt(0).getBoundingClientRect(); \
        if (box.top < 0 || box.bottom > innerHeight) scrollBy(0, (box.top + box.bottom - innerHeight) / 2); })()";

    /// What `view`'s find runs once it ends: the match it selected scrolls into view.
    pub(super) fn reveal_match(view: &AnyObject) -> RcBlock<dyn Fn(*mut AnyObject)> {
        // SAFETY: the view is a live WKWebView, which the block keeps until the find ends.
        let view = unsafe { Retained::retain(std::ptr::from_ref(view).cast_mut()) };
        RcBlock::new(move |result: *mut AnyObject| {
            // SAFETY: WebKit passes the find's `WKFindResult`, and the view runs the script in its page; a nil
            // handler leaves the script's result unread.
            unsafe {
                let found: bool = result.as_ref().is_some_and(|result| msg_send![result, matchFound]);
                if let (true, Some(view)) = (found, &view) {
                    let unanswered: Option<&DynBlock<dyn Fn(*mut AnyObject, *mut AnyObject)>> = None;
                    let script = NSString::from_str(REVEAL_SELECTION);
                    let () = msg_send![&**view, evaluateJavaScript: &*script, completionHandler: unanswered];
                }
            }
        })
    }

    /// A page drawn in a `WKWebView` octopus makes, which has no access to the app. Lives on the main thread.
    pub struct Page(Retained<AnyObject>);

    impl Page {
        /// A page with no URL yet, whose every change `watcher` hears.
        pub fn new(embedder: Embedder, watcher: impl Fn(Told<PhoneChange>) + 'static) -> Self {
            let delegate = delegate();
            // SAFETY: each call is made as WebKit and UIKit document, on the main thread, with live objects.
            let view: Retained<AnyObject> = unsafe {
                let configuration: Retained<AnyObject> = msg_send![class!(WKWebViewConfiguration), new];
                let scheme = NSString::from_str(crate::served_files::SCHEME);
                let () = msg_send![&*configuration, setURLSchemeHandler: &*delegate, forURLScheme: &*scheme];
                let () = msg_send![&*configuration, setAllowsInlineMediaPlayback: true];
                let view: Allocated<AnyObject> = msg_send![class!(WKWebView), alloc];
                let view: Retained<AnyObject> =
                    msg_send![view, initWithFrame: CGRect::ZERO, configuration: &*configuration];
                let () = msg_send![&*view, setNavigationDelegate: &*delegate];
                let () = msg_send![&*view, setUIDelegate: &*delegate];
                let () = msg_send![&*view, setAllowsBackForwardNavigationGestures: true];
                let edge: Allocated<AnyObject> = msg_send![class!(UIScreenEdgePanGestureRecognizer), alloc];
                let edge: Retained<AnyObject> = msg_send![edge, initWithTarget: &*delegate, action: sel!(edgeDragged:)];
                let () = msg_send![&*edge, setEdges: LEFT_EDGE];
                let () = msg_send![&*edge, setDelegate: &*delegate];
                let () = msg_send![&*view, addGestureRecognizer: &*edge];
                view
            };
            EMBEDDERS.with_borrow_mut(|embedders| embedders.insert(key(&*view), Rc::new(embedder)));
            let page = Self(view);
            page.view().watch(watcher);
            page
        }

        pub fn view(&self) -> View<'_> {
            View(&self.0)
        }

        /// The page's `WKWebView`, a `UIView` its embedder places.
        pub fn native(&self) -> &AnyObject {
            &self.0
        }

        pub fn load(&self, url: &str) {
            // SAFETY: WKWebView's own load of a request for a URL; a string that is no URL makes none.
            unsafe {
                let url: *mut AnyObject = msg_send![class!(NSURL), URLWithString: &*NSString::from_str(url)];
                let Some(url) = url.as_ref() else { return };
                let request: *mut AnyObject = msg_send![class!(NSURLRequest), requestWithURL: url];
                let _: *mut AnyObject = msg_send![&*self.0, loadRequest: request];
            }
        }
    }

    impl Drop for Page {
        fn drop(&mut self) {
            self.view().unwatch();
            EMBEDDERS.with_borrow_mut(|embedders| embedders.remove(&key(&*self.0)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_load_webkit_ends_on_purpose_is_no_failure() {
        assert!(on_purpose("NSURLErrorDomain", -999));
        assert!(on_purpose("WebKitErrorDomain", 102));
        assert!(on_purpose("WebKitErrorDomain", 204));
        assert!(!on_purpose("NSURLErrorDomain", -1100));
        assert!(!on_purpose("NSURLErrorDomain", 204));
    }

    #[test]
    fn a_download_takes_the_lowest_free_number_after_its_stem() {
        let folder = Path::new("/d");
        let taken = |names: &'static [&str]| move |path: &Path| names.iter().any(|name| folder.join(name) == path);
        assert_eq!(unclaimed(folder, "a.zip", taken(&[])), folder.join("a.zip"));
        assert_eq!(unclaimed(folder, "a.zip", taken(&["a.zip", "a 2.zip"])), folder.join("a 3.zip"));
        assert_eq!(unclaimed(folder, "notes", taken(&["notes"])), folder.join("notes 2"));
    }
}
