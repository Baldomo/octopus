//! Each browser panel of a task renders as a native webview in the window's page layer. The webview loads the panel's
//! `url` each time its `load` moves, and reports every page it navigates to back to the server. A page on a loopback
//! host of a server on another machine reaches that port on the server's machine through a tunnel, and a file page
//! loads through `octopus_shell::pages`. A link the user opens in a new tab, and a page's new window, go to the UI,
//! which opens them as browser panels in the task's order of tabs.

use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use octopus_client::tunnel::{Tunnel, listen_on_localhost, loopback_port};
use octopus_proto::ops::BrowserReport;
use octopus_proto::types::{BrowserId, web_page};
use octopus_shell::hub::Hub;
use octopus_shell::pages::Pages;
use octopus_shell::task_lists::TaskRef;
use octopus_shell::ui::{Bounds, UiStore};
use octopus_shell::{emit, lock};
use serde::{Deserialize, Serialize};
use tauri::webview::{DownloadEvent, NewWindowResponse, WebviewBuilder};
use tauri::{AppHandle, Manager, Url, Webview, WebviewUrl};
use ts_rs::TS;

use crate::layers::Layers;

pub const PAGED_EVENT: &str = "browser_paged";
pub const FAILED_EVENT: &str = "browser_failed";
pub const DOWNLOADED_EVENT: &str = "browser_downloaded";
pub const OPENED_EVENT: &str = "browser_opened";
pub const HOVERED_EVENT: &str = "browser_hovered";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Go {
    Back,
    Forward,
    Reload,
    ReloadFromOrigin,
}

/// The page a browser panel's webview shows on this client, whenever it or its loading changes.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Paged {
    /// The task's key.
    pub task: String,
    pub panel: BrowserId,
    pub url: String,
    pub loading: bool,
    /// Whether the panel's history goes back, and forward.
    pub back: bool,
    pub forward: bool,
    /// The port of the server's machine the page reaches through a tunnel, when it does.
    pub tunnel: Option<u16>,
}

/// A page a browser panel's webview could not load.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Failed {
    /// The task's key.
    pub task: String,
    pub panel: BrowserId,
    pub url: String,
    pub error: String,
}

/// A page a browser panel's page opens in a new tab: a link the user opened there, or a new window the page asked for.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Opened {
    /// The task's key.
    pub task: String,
    /// The panel whose page opened it.
    pub panel: BrowserId,
    pub url: String,
    /// Whether the new tab takes the selection, else it opens behind the selected one.
    pub select: bool,
}

/// The link under the pointer in the page on screen, and the page's rect in the UI's CSS pixels.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Hovered {
    pub link: String,
    pub page: Bounds,
}

/// What a webview tells of the page it shows.
enum Change {
    Seen {
        url: Option<String>,
        loading: bool,
        back: bool,
        forward: bool,
    },
    Failed {
        url: String,
        error: String,
    },
    #[cfg(target_os = "macos")]
    Opened {
        url: String,
        select: bool,
    },
    Hovered {
        link: Option<String>,
    },
}

/// A download a page started, finished in this machine's Downloads folder.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Downloaded {
    pub path: String,
    pub success: bool,
}

/// A browser panel anywhere in the client: `<server>/<task>/<panel>`.
fn panel_key(at: &TaskRef, panel: &BrowserId) -> String {
    format!("{}/{panel}", at.key())
}

/// The browser panel the page layer shows, at its rect.
#[derive(Debug, Clone, PartialEq, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct OnScreen {
    pub at: TaskRef,
    pub panel: BrowserId,
    pub bounds: Bounds,
}

/// The page a webview shows, and the last one it reported. A load that fails leaves the webview on the page it
/// reported before, which the server already moved past.
#[derive(Default)]
struct Page {
    url: Option<Url>,
    title: String,
    reported: Option<(String, String)>,
}

struct Rendered {
    webview: Webview,
    /// The panel's `load` the webview last loaded.
    load: u64,
}

/// One listening tunnel, held while any panel has shown a page on its port.
struct Held {
    server: String,
    panels: BTreeSet<String>,
    _tunnel: Tunnel,
}

pub struct Browsers {
    app: AppHandle,
    layers: Arc<Layers>,
    next: AtomicU64,
    rendered: Mutex<HashMap<String, Rendered>>,
    /// Keyed by port: this client's `localhost:<port>` reaches one server at a time.
    tunnels: tokio::sync::Mutex<HashMap<u16, Held>>,
}

fn failed(error: tauri::Error) -> String {
    error.to_string()
}

impl Browsers {
    pub fn new(app: AppHandle, layers: Arc<Layers>) -> Self {
        Self {
            app,
            layers,
            next: AtomicU64::new(0),
            rendered: Mutex::default(),
            tunnels: tokio::sync::Mutex::default(),
        }
    }

    fn with<T>(
        &self,
        at: &TaskRef,
        panel: &BrowserId,
        act: impl FnOnce(&Webview) -> tauri::Result<T>,
    ) -> Result<T, String> {
        let key = panel_key(at, panel);
        let rendered = lock(&self.rendered);
        let rendered = rendered.get(&key).ok_or_else(|| format!("no browser panel {key}"))?;
        act(&rendered.webview).map_err(failed)
    }

    /// Loads the panel's `url` into its webview, created hidden and blank on its first page so that it is watched
    /// before it loads, unless the webview already loaded this `load`; `show_page` puts it on screen.
    pub async fn load(
        &self,
        app: &AppHandle,
        hub: &Arc<Hub>,
        at: &TaskRef,
        panel: &BrowserId,
        url: &str,
        load: u64,
    ) -> Result<(), String> {
        let key = panel_key(at, panel);
        if lock(&self.rendered).get(&key).is_some_and(|rendered| rendered.load == load) {
            return Ok(());
        }
        let page = web_page(url)?;
        self.hold_tunnel(hub, &at.server, &key, &page).await?;
        let loaded = app.state::<Arc<Pages>>().loaded_from(&at.server, &page)?;
        let mut rendered = lock(&self.rendered);
        let rendered = match rendered.entry(key) {
            Entry::Occupied(existing) => existing.into_mut(),
            Entry::Vacant(first) => first.insert(Rendered { webview: self.create(app, at, panel)?, load }),
        };
        rendered.load = load;
        rendered.webview.navigate(loaded).map_err(failed)
    }

    fn create(&self, app: &AppHandle, at: &TaskRef, panel: &BrowserId) -> Result<Webview, String> {
        let label = format!("browser-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let blank = WebviewUrl::External(Url::parse("about:blank").expect("about:blank is a URL"));
        let page = Arc::new(Mutex::new(Page::default()));
        let titled = Arc::clone(&page);
        let (app_for_title, at_for_title, panel_for_title) = (app.clone(), at.clone(), panel.clone());
        let (app_for_window, at_for_window, panel_for_window) = (app.clone(), at.clone(), panel.clone());
        let (app_for_navigation, at_for_navigation, key) = (app.clone(), at.clone(), panel_key(at, panel));
        let downloading = Mutex::new(HashMap::<Url, std::path::PathBuf>::new());
        let builder = WebviewBuilder::new(label, blank)
            .on_navigation(move |url| {
                hold_while_navigating(&app_for_navigation, &at_for_navigation, &key, url);
                true
            })
            .on_document_title_changed(move |_, title| {
                lock(&titled).title = title;
                report(&app_for_title, &at_for_title, &panel_for_title, &titled);
            })
            .on_new_window(move |url, _| {
                open_tab(&app_for_window, &at_for_window, &panel_for_window, &url, true);
                NewWindowResponse::Deny
            })
            .on_download(move |webview, event| {
                match event {
                    DownloadEvent::Requested { url, destination } => {
                        lock(&downloading).insert(url, destination.clone());
                    }
                    DownloadEvent::Finished { url, path, success } => {
                        let path = path.or_else(|| lock(&downloading).remove(&url)).unwrap_or_default();
                        let path = path.display().to_string();
                        emit(webview.app_handle(), DOWNLOADED_EVENT, Downloaded { path, success });
                    }
                    _ => {}
                }
                true
            });
        let webview = self.layers.add_page(builder).map_err(failed)?;
        let (app, at, panel, layers) = (app.clone(), at.clone(), panel.clone(), Arc::clone(&self.layers));
        native::watch(&webview, move |change| match change {
            Change::Seen { url, loading, back, forward } => {
                let url = url.as_deref().and_then(|url| Url::parse(url).ok()).filter(|url| url.scheme() != "about");
                let finished = !loading && url.is_some();
                lock(&page).url.clone_from(&url);
                let shown = url.as_ref().map(|url| page_of(&app, url));
                let hub = app.state::<Arc<Hub>>();
                let tunnel = shown
                    .as_ref()
                    .and_then(|shown| loopback_port(shown).filter(|_| hub.is_another_machine(&at.server)));
                let url = shown.map(String::from).unwrap_or_default();
                let paged = Paged { task: at.key(), panel: panel.clone(), url, loading, back, forward, tunnel };
                emit(&app, PAGED_EVENT, paged);
                if finished {
                    report(&app, &at, &panel, &page);
                }
            }
            Change::Failed { url, error } => {
                let url = Url::parse(&url).map_or(url, |url| page_of(&app, &url).to_string());
                emit(&app, FAILED_EVENT, Failed { task: at.key(), panel: panel.clone(), url, error });
            }
            #[cfg(target_os = "macos")]
            Change::Opened { url, select } => {
                if let Ok(url) = Url::parse(&url) {
                    open_tab(&app, &at, &panel, &url, select);
                }
            }
            Change::Hovered { link } => {
                let link = link.as_deref().and_then(|link| Url::parse(link).ok()).map(|link| page_of(&app, &link));
                let hovered = link.zip(layers.page_bounds()).map(|(link, page)| Hovered { link: link.into(), page });
                emit(&app, HOVERED_EVENT, hovered);
            }
        })
        .map_err(failed)?;
        Ok(webview)
    }

    /// Holds the tunnel to the page's port for the panel `key`, when the page is on a loopback host of a server on
    /// another machine.
    async fn hold_tunnel(&self, hub: &Arc<Hub>, server: &str, key: &str, page: &Url) -> Result<(), String> {
        let Some(port) = loopback_port(page).filter(|_| hub.is_another_machine(server)) else { return Ok(()) };
        let mut tunnels = self.tunnels.lock().await;
        match tunnels.get_mut(&port) {
            Some(held) if held.server != server => {
                Err(format!("this client's port {port} already tunnels to {}", hub.label(&held.server)))
            }
            Some(held) => {
                held.panels.insert(key.to_string());
                Ok(())
            }
            None => {
                let listeners = listen_on_localhost(port).await.map_err(|error| error.to_string())?;
                let (hub, dialed) = (Arc::clone(hub), server.to_string());
                let tunnel = Tunnel::new(port, listeners, Arc::new(move || hub.connection(&dialed)));
                let panels = BTreeSet::from([key.to_string()]);
                tunnels.insert(port, Held { server: server.to_string(), panels, _tunnel: tunnel });
                Ok(())
            }
        }
    }

    pub fn show_page(&self, on_screen: Option<OnScreen>) -> Result<(), String> {
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
        let mut tunnels = self.tunnels.lock().await;
        for held in tunnels.values_mut() {
            held.panels.remove(&key);
        }
        tunnels.retain(|_, held| !held.panels.is_empty());
        let rendered = lock(&self.rendered).remove(&key).ok_or_else(|| format!("no browser panel {key}"))?;
        native::unwatch(&rendered.webview).map_err(failed)?;
        self.layers.close_page(&rendered.webview).map_err(failed)
    }
}

/// A frame of the page navigating to a loopback port of a server on another machine reaches it through a tunnel
/// held before the navigation goes on.
fn hold_while_navigating(app: &AppHandle, at: &TaskRef, key: &str, url: &Url) {
    let hub = Arc::clone(app.state::<Arc<Hub>>().inner());
    let held = tauri::async_runtime::block_on(app.state::<Browsers>().hold_tunnel(&hub, &at.server, key, url));
    if let Err(problem) = held {
        app.state::<Arc<UiStore>>().report(problem);
    }
}

/// Records on the server each new page or title the webview shows.
fn report(app: &AppHandle, at: &TaskRef, panel: &BrowserId, page: &Mutex<Page>) {
    let (url, title) = {
        let mut page = lock(page);
        let Some(url) = &page.url else { return };
        let shown = (page_of(app, url).to_string(), page.title.clone());
        if page.reported.as_ref() == Some(&shown) {
            return;
        }
        page.reported = Some(shown.clone());
        shown
    };
    let hub = Arc::clone(app.state::<Arc<Hub>>().inner());
    let (at, panel) = (at.clone(), panel.clone());
    tauri::async_runtime::spawn(async move {
        let reported = hub.call(&at.server, &BrowserReport { task: at.task.clone(), panel, url, title }).await;
        if let Err(error) = reported {
            eprintln!("octopus app: report the page of a browser panel of {}: {error}", at.key());
        }
    });
}

/// The page a webview shows at `loaded`, as the server records it.
fn page_of(app: &AppHandle, loaded: &Url) -> Url {
    app.state::<Arc<Pages>>().page_of(loaded)
}

fn open_tab(app: &AppHandle, at: &TaskRef, panel: &BrowserId, url: &Url, select: bool) {
    let url = page_of(app, url).to_string();
    emit(app, OPENED_EVENT, Opened { task: at.key(), panel: panel.clone(), url, select });
}

/// Whether a browser page holds the keyboard in the window.
pub fn page_has_focus(window: &tauri::Window) -> bool {
    native::has_focus(window)
}

/// WebKit tells its embedder what a page shows through key-value observing of the webview, and a failed load through
/// its navigation delegate, whose class wry declares without the failure methods; octopus adds them, and makes the
/// delegate the webview's observer. A link clicked with ⌘ or the middle button reaches the navigation delegate's
/// policy, which octopus takes over from wry for watched webviews. The link under the pointer and the context menu
/// reach the UI delegate through WebKit's private methods, which octopus adds too. WebKit notes which methods a
/// delegate answers when it is set, so a webview set its delegates before the methods existed has them set again.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod native {
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
    type MouseOver =
        unsafe extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject, *mut AnyObject, usize, *mut AnyObject);
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
            let url = match info.as_ref() {
                Some(info) => text(msg_send![info, objectForKey: &*key]),
                None => None,
            };
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
        let opened =
            unsafe { action.as_ref() }.filter(|_| watched(webview)).and_then(|action| unsafe { new_tab(action) });
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
        if let (Some(link), Some(view), Some(proposed)) = (link, unsafe { webview.as_ref() }, unsafe { menu.as_ref() })
        {
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

    pub fn unwatch(webview: &Webview) -> tauri::Result<()> {
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
    pub fn has_focus(window: &tauri::Window) -> bool {
        window.ns_window().is_ok_and(|window| {
            // SAFETY: tauri hands over its live NSWindow.
            let responder: *mut AnyObject = unsafe { msg_send![&*window.cast::<AnyObject>(), firstResponder] };
            watched(responder)
        })
    }
}

/// WebKitGTK tells what a page shows through the webview's properties and signals, which end with the webview.
#[cfg(target_os = "linux")]
mod native {
    use std::rc::Rc;

    use tauri::Webview;
    use webkit2gtk::{FindControllerExt, FindOptions, HitTestResultExt, WebViewExt};

    use super::{Change, Go};

    pub fn watch(webview: &Webview, watcher: impl Fn(Change) + Send + 'static) -> tauri::Result<()> {
        webview.with_webview(move |platform| {
            let view = platform.inner();
            let watcher: Rc<dyn Fn(Change)> = Rc::new(watcher);
            let seen = |watcher: Rc<dyn Fn(Change)>| {
                move |view: &webkit2gtk::WebView| {
                    watcher(Change::Seen {
                        url: view.uri().map(|uri| uri.to_string()),
                        loading: view.is_loading(),
                        back: view.can_go_back(),
                        forward: view.can_go_forward(),
                    });
                }
            };
            view.connect_uri_notify(seen(Rc::clone(&watcher)));
            view.connect_is_loading_notify(seen(Rc::clone(&watcher)));
            let hovering = Rc::clone(&watcher);
            view.connect_mouse_target_changed(move |_, hit, _| {
                hovering(Change::Hovered { link: hit.link_uri().map(|link| link.to_string()) });
            });
            view.connect_load_failed(move |_, _, url, error| {
                watcher(Change::Failed { url: url.to_string(), error: error.to_string() });
                true
            });
        })
    }

    pub fn unwatch(_: &Webview) -> tauri::Result<()> {
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

    pub fn has_focus(_: &tauri::Window) -> bool {
        false
    }
}
