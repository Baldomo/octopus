//! The OS's clipboard, which the page can't read in one tap: iOS asks the user to confirm every read but one from its
//! own edit menu, and Android's webview keeps the clipboard from the page. The app reads it instead and hands the page
//! its text, and each of its images as a file on this phone, which the page copies to a server as it would a dropped
//! one. On iOS a terminal's selection menu is the system's edit menu, and the page hears its Copy and Paste as
//! `phone-edit-menu`.

use std::path::PathBuf;

use anyhow::Context;
use octopus_shell::Offered;
use serde::Serialize;
use tauri::{AppHandle, Manager};

#[derive(Clone, Default, Serialize)]
pub(crate) struct Pasted {
    text: String,
    files: Vec<PathBuf>,
}

impl Pasted {
    /// Lets the page copy the pasted files to a server.
    fn offered(self, app: &AppHandle) -> Self {
        app.state::<Offered>().add(&self.files);
        self
    }
}

/// The folder in the app's cache that pasted images are kept under, each in a folder of its own, until the next launch
/// deletes them: the page may upload a pasted path any time before the app quits.
struct Kept(PathBuf);

/// Deletes the images an earlier launch kept, and keeps this launch's in the same folder.
pub(crate) fn clear(app: &AppHandle) -> anyhow::Result<()> {
    let root = app.path().app_cache_dir()?.join("pasted");
    match std::fs::remove_dir_all(&root) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("clear {}", root.display()))
        }
        _ => {
            app.manage(Kept(root));
            Ok(())
        }
    }
}

/// Reads the clipboard. Async, so the page goes on while the OS answers.
#[tauri::command]
pub(crate) async fn phone_paste(app: AppHandle) -> Result<Pasted, String> {
    let pasted = os::read(&app).await.map_err(|error| format!("{error:#}"))?;
    Ok(pasted.offered(&app))
}

/// Opens the edit menu, or moves the open one. Sync, as is closing it, so each lands in the order the page sent it.
#[cfg(target_os = "ios")]
#[tauri::command]
pub(crate) fn phone_edit_menu(app: AppHandle, menu: os::EditMenu) -> Result<(), String> {
    os::open(&app, menu).map_err(|error| format!("{error:#}"))
}

/// Closes the edit menu `key` names, while it shows.
#[cfg(target_os = "ios")]
#[tauri::command]
pub(crate) fn phone_edit_menu_close(app: AppHandle, key: u32) -> Result<(), String> {
    os::close(&app, key).map_err(|error| format!("{error:#}"))
}

#[cfg(target_os = "android")]
mod os {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use anyhow::Context;
    use octopus_shell::ui::UiStore;
    use serde::{Deserialize, Serialize};
    use tauri::{AppHandle, Manager};

    use super::{Kept, Pasted};

    #[derive(Serialize)]
    struct Paste<'a> {
        folder: &'a Path,
    }

    /// The clipboard as `DevicePlugin.kt` read it, with why each item it couldn't read failed.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Clip {
        text: String,
        files: Vec<PathBuf>,
        problems: Vec<String>,
    }

    /// `DevicePlugin.kt` copies each image into a folder of its own under the kept folder.
    pub(super) async fn read(app: &AppHandle) -> anyhow::Result<Pasted> {
        let paste = Paste { folder: &app.state::<Kept>().inner().0 };
        let clip: Clip = crate::device::run(app, "paste", paste).context("read the clipboard")?;
        for problem in clip.problems {
            app.state::<Arc<UiStore>>().report(problem);
        }
        Ok(Pasted { text: clip.text, files: clip.files })
    }
}

#[cfg(target_os = "ios")]
mod os {
    #![allow(unsafe_code, reason = "UIKit's pasteboard and edit menu, and WebKit's responder, have no safe binding")]

    use std::cell::{OnceCell, RefCell};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};

    use anyhow::Context;
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, ProtocolObject, Sel};
    use objc2::{DefinedClass, MainThreadOnly, define_class, ffi, msg_send, sel};
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_foundation::{NSArray, NSData, NSError, NSItemProvider, NSObject, NSObjectProtocol, NSString, NSURL};
    use objc2_ui_kit::{
        UICommand, UIEditMenuConfiguration, UIEditMenuInteraction, UIEditMenuInteractionAnimating,
        UIEditMenuInteractionDelegate, UIMenu, UIMenuElement, UIPasteboard, UIResponder, UIView, UIViewAutoresizing,
    };
    use octopus_shell::ui::UiStore;
    use serde::{Deserialize, Serialize};
    use tauri::{AppHandle, Manager};
    use tokio::sync::oneshot;

    use super::{Kept, Pasted};

    const CHOSE: &str = "phone-edit-menu";
    const IMAGE: &str = "public.image";
    const TEXT: &str = "public.utf8-plain-text";

    /// Reads the clipboard once the user lets the OS's own prompt allow it.
    pub(super) async fn read(app: &AppHandle) -> anyhow::Result<Pasted> {
        Ok(collect(app, clipboard(app)).await)
    }

    /// Starts loading each item of the clipboard.
    fn clipboard(app: &AppHandle) -> Vec<Loading> {
        let kept = &app.state::<Kept>().inner().0;
        // SAFETY: the general pasteboard is the OS's, and reading its items is safe from any thread.
        unsafe { UIPasteboard::generalPasteboard().itemProviders() }
            .iter()
            .filter_map(|provider| load(&provider, kept))
            .collect()
    }

    /// The terminal's selection menu, the system's edit menu, about a rectangle of the webview in the OS's points:
    /// Copy while there is a selection, whose text it copies, and Paste while the terminal takes input. Each menu the
    /// page opens has a key of its own, which the page hears back with what the menu did.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(crate) struct EditMenu {
        key: u32,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        selection: Option<String>,
        paste: bool,
    }

    impl EditMenu {
        fn rect(&self) -> CGRect {
            CGRect::new(CGPoint::new(self.x, self.y), CGSize::new(self.width, self.height))
        }
    }

    #[derive(Clone, Serialize)]
    #[serde(tag = "did", rename_all = "lowercase")]
    enum Did {
        Copy { key: u32 },
        Paste { key: u32, pasted: Pasted },
    }

    /// The menu the page last opened, and whether it still shows.
    struct Opened {
        menu: EditMenu,
        shows: bool,
    }

    define_class!(
        /// A view over the webview, which touches pass through, that shows the edit menu and answers its Copy and
        /// Paste.
        #[unsafe(super(UIView, UIResponder, NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "OctopusEditMenu"]
        #[ivars = Menu]
        struct MenuView;

        impl MenuView {
            #[unsafe(method(canPerformAction:withSender:))]
            fn can_perform(&self, action: Sel, _sender: Option<&AnyObject>) -> bool {
                self.offers(action) == Some(true)
            }

            #[unsafe(method(copy:))]
            fn copy(&self, _sender: Option<&AnyObject>) {
                let Some((key, text)) = self.ivars().opened.borrow().as_ref().and_then(|opened| {
                    opened.menu.selection.clone().map(|text| (opened.menu.key, text))
                }) else {
                    return;
                };
                // SAFETY: the general pasteboard is the OS's, written here on the main thread.
                unsafe { UIPasteboard::generalPasteboard().setString(Some(&NSString::from_str(&text))) };
                octopus_shell::emit(&self.ivars().app, CHOSE, Did::Copy { key });
            }

            /// The edit menu's own Paste lets the app read the clipboard without asking the user.
            #[unsafe(method(paste:))]
            fn paste(&self, _sender: Option<&AnyObject>) {
                let Some(key) = self.ivars().opened.borrow().as_ref().map(|opened| opened.menu.key) else { return };
                let app = self.ivars().app.clone();
                let loads = clipboard(&app);
                tauri::async_runtime::spawn(async move {
                    let pasted = collect(&app, loads).await.offered(&app);
                    octopus_shell::emit(&app, CHOSE, Did::Paste { key, pasted });
                });
            }
        }

        unsafe impl NSObjectProtocol for MenuView {}

        unsafe impl UIEditMenuInteractionDelegate for MenuView {
            /// The system's own Copy and Paste among what the responders suggest, and nothing else.
            #[unsafe(method_id(editMenuInteraction:menuForConfiguration:suggestedActions:))]
            fn menu_for(
                &self,
                _: &UIEditMenuInteraction,
                _: &UIEditMenuConfiguration,
                suggested: &NSArray<UIMenuElement>,
            ) -> Option<Retained<UIMenu>> {
                Some(UIMenu::menuWithChildren(&NSArray::from_retained_slice(&copy_and_paste(suggested)), self.mtm()))
            }

            #[unsafe(method(editMenuInteraction:targetRectForConfiguration:))]
            fn target_rect(&self, _: &UIEditMenuInteraction, _: &UIEditMenuConfiguration) -> CGRect {
                self.ivars().opened.borrow().as_ref().map_or(CGRect::ZERO, |opened| opened.menu.rect())
            }

            #[unsafe(method(editMenuInteraction:willDismissMenuForConfiguration:animator:))]
            fn will_dismiss(
                &self,
                _: &UIEditMenuInteraction,
                _: &UIEditMenuConfiguration,
                _: &ProtocolObject<dyn UIEditMenuInteractionAnimating>,
            ) {
                if let Some(opened) = self.ivars().opened.borrow_mut().as_mut() {
                    opened.shows = false;
                }
            }
        }
    );

    struct Menu {
        app: AppHandle,
        interaction: OnceCell<Retained<UIEditMenuInteraction>>,
        opened: RefCell<Option<Opened>>,
    }

    impl MenuView {
        fn over(view: &UIView, app: AppHandle) -> Retained<Self> {
            let mtm = view.mtm();
            let menu = Menu { app, interaction: OnceCell::new(), opened: RefCell::new(None) };
            // SAFETY: UIView's designated initializer takes the view's frame.
            let this: Retained<Self> =
                unsafe { msg_send![super(Self::alloc(mtm).set_ivars(menu)), initWithFrame: view.bounds()] };
            this.setAutoresizingMask(UIViewAutoresizing::FlexibleWidth | UIViewAutoresizing::FlexibleHeight);
            this.setUserInteractionEnabled(false);
            let interaction = UIEditMenuInteraction::initWithDelegate(
                UIEditMenuInteraction::alloc(mtm),
                Some(ProtocolObject::from_ref(&*this)),
            );
            this.addInteraction(ProtocolObject::from_ref(&*interaction));
            let _ = this.ivars().interaction.set(interaction);
            view.addSubview(&this);
            this
        }

        fn interaction(&self) -> &UIEditMenuInteraction {
            self.ivars().interaction.get().expect("set on making the view")
        }

        /// Opens a new menu, or moves the one the page opened as it follows its selection.
        fn show(&self, menu: EditMenu) {
            let mut opened = self.ivars().opened.borrow_mut();
            match opened.as_mut() {
                Some(open) if open.menu.key == menu.key => {
                    let offers = |menu: &EditMenu| (menu.selection.is_some(), menu.paste);
                    let reload = offers(&open.menu) != offers(&menu);
                    open.menu = menu;
                    if open.shows {
                        drop(opened);
                        if reload {
                            self.interaction().reloadVisibleMenu();
                        }
                        self.interaction().updateVisibleMenuPositionAnimated(false);
                    }
                }
                _ => {
                    let rect = menu.rect();
                    let at = CGPoint::new(rect.origin.x + rect.size.width / 2.0, rect.origin.y);
                    *opened = Some(Opened { menu, shows: true });
                    drop(opened);
                    // SAFETY: the menu needs no identifier, and its source point is in this view's coordinates.
                    let configuration = unsafe {
                        UIEditMenuConfiguration::configurationWithIdentifier_sourcePoint(None, at, self.mtm())
                    };
                    self.interaction().presentEditMenuWithConfiguration(&configuration);
                }
            }
        }

        fn close(&self, key: u32) {
            let closed = self.ivars().opened.borrow_mut().take_if(|open| open.menu.key == key);
            if closed.is_some_and(|open| open.shows) {
                self.interaction().dismissMenu();
            }
        }

        /// Whether the open menu offers `action`; none while the page has no menu open.
        fn offers(&self, action: Sel) -> Option<bool> {
            self.ivars().opened.borrow().as_ref().map(|opened| {
                (action == sel!(copy:) && opened.menu.selection.is_some())
                    || (action == sel!(paste:) && opened.menu.paste)
            })
        }
    }

    fn copy_and_paste(elements: &NSArray<UIMenuElement>) -> Vec<Retained<UIMenuElement>> {
        elements
            .iter()
            .flat_map(|element| {
                if let Some(menu) = element.downcast_ref::<UIMenu>() {
                    return copy_and_paste(&menu.children());
                }
                // SAFETY: a command's action is the selector it sends.
                let ours = element
                    .downcast_ref::<UICommand>()
                    .is_some_and(|command| [sel!(copy:), sel!(paste:)].contains(&unsafe { command.action() }));
                if ours { vec![element] } else { Vec::new() }
            })
            .collect()
    }

    type CanPerform = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, Sel, *mut AnyObject) -> Bool;
    type TargetFor = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, Sel, *mut AnyObject) -> *mut AnyObject;

    static WEBKIT: OnceLock<(CanPerform, TargetFor)> = OnceLock::new();

    /// While the keyboard is up, WebKit's content view is the first responder the edit menu asks for its actions and
    /// sends them to. While the page has a menu open, it offers the menu's actions alone and hands them to the menu.
    fn route_through_menu() -> anyhow::Result<()> {
        unsafe extern "C-unwind" fn can_perform(
            this: *mut AnyObject,
            cmd: Sel,
            action: Sel,
            sender: *mut AnyObject,
        ) -> Bool {
            match MENU.with(|menu| menu.get().and_then(|menu| menu.offers(action))) {
                Some(offered) => Bool::new(offered),
                // SAFETY: WebKit's own implementation, called with the arguments it was sent.
                None => unsafe { (WEBKIT.get().expect("stored before replacing").0)(this, cmd, action, sender) },
            }
        }
        unsafe extern "C-unwind" fn target_for(
            this: *mut AnyObject,
            cmd: Sel,
            action: Sel,
            sender: *mut AnyObject,
        ) -> *mut AnyObject {
            let menu = MENU.with(|menu| {
                menu.get()
                    .filter(|menu| menu.offers(action) == Some(true))
                    .map(|menu| Retained::as_ptr(menu).cast_mut())
            });
            // SAFETY: WebKit's own implementation, called with the arguments it was sent.
            menu.map_or_else(
                || unsafe { (WEBKIT.get().expect("stored before replacing").1)(this, cmd, action, sender) },
                |menu| menu.cast(),
            )
        }

        let content = AnyClass::get(c"WKContentView").context("WebKit has no WKContentView")?;
        let current = |selector: Sel| content.instance_method(selector).map(|method| method.implementation());
        let can = current(sel!(canPerformAction:withSender:)).context("WKContentView answers no canPerformAction")?;
        let target = current(sel!(targetForAction:withSender:)).context("WKContentView answers no targetForAction")?;
        // SAFETY: each IMP is the one the runtime calls for its selector, whose signature its type names.
        let _ = WEBKIT.set(unsafe {
            (std::mem::transmute::<Imp, CanPerform>(can), std::mem::transmute::<Imp, TargetFor>(target))
        });
        // SAFETY: each replacement takes and returns what the selector's type encoding names.
        unsafe {
            let class = std::ptr::from_ref(content).cast_mut();
            ffi::class_replaceMethod(
                class,
                sel!(canPerformAction:withSender:),
                std::mem::transmute::<CanPerform, Imp>(can_perform),
                c"B@::@".as_ptr(),
            );
            ffi::class_replaceMethod(
                class,
                sel!(targetForAction:withSender:),
                std::mem::transmute::<TargetFor, Imp>(target_for),
                c"@@::@".as_ptr(),
            );
        }
        Ok(())
    }

    thread_local! {
        static MENU: OnceCell<Retained<MenuView>> = const { OnceCell::new() };
    }

    fn on_menu(app: &AppHandle, act: impl FnOnce(&MenuView) + Send + 'static) -> anyhow::Result<()> {
        let owner = app.clone();
        crate::webview::on_view(app, move |view| {
            if MENU.with(|menu| menu.get().is_none())
                && let Err(error) = route_through_menu()
            {
                return owner.state::<Arc<UiStore>>().report(format!("{error:#}"));
            }
            MENU.with(|menu| act(menu.get_or_init(|| MenuView::over(view, owner))));
        })
        .context("reach the edit menu")
    }

    pub(super) fn open(app: &AppHandle, menu: EditMenu) -> anyhow::Result<()> {
        on_menu(app, move |view| view.show(menu))
    }

    pub(super) fn close(app: &AppHandle, key: u32) -> anyhow::Result<()> {
        on_menu(app, move |view| view.close(key))
    }

    enum Piece {
        Text(String),
        File(PathBuf),
    }

    type Loading = oneshot::Receiver<Result<Piece, String>>;

    /// Starts loading `provider`'s image as a file kept under `kept`, else its text; an item that holds neither pastes
    /// nothing.
    fn load(provider: &NSItemProvider, kept: &Path) -> Option<Loading> {
        let (sent, loading) = oneshot::channel();
        let sent = Mutex::new(Some(sent));
        let answer = move |piece: Result<Piece, String>| {
            if let Some(sent) = sent.lock().expect("no answer panics holding the lock").take() {
                let _ = sent.send(piece);
            }
        };
        let image = NSString::from_str(IMAGE);
        let text = NSString::from_str(TEXT);
        if provider.hasItemConformingToTypeIdentifier(&image) {
            let kept = kept.to_owned();
            let copied = RcBlock::new(move |url: *mut NSURL, error: *mut NSError| {
                // SAFETY: the OS hands the block a file URL, valid until the block returns, or else an error.
                answer(unsafe { url.as_ref() }.ok_or_else(|| failure(error)).and_then(|url| keep(url, &kept)));
            });
            // SAFETY: the block is Send, as the method needs: it answers through a channel behind a mutex.
            unsafe { provider.loadFileRepresentationForTypeIdentifier_completionHandler(&image, &copied) };
        } else if provider.hasItemConformingToTypeIdentifier(&text) {
            let read = RcBlock::new(move |data: *mut NSData, error: *mut NSError| {
                // SAFETY: the OS hands the block the text's bytes, or else an error.
                let data = unsafe { data.as_ref() }.ok_or_else(|| failure(error));
                answer(data.map(|data| Piece::Text(String::from_utf8_lossy(&data.to_vec()).into_owned())));
            });
            // SAFETY: as for the image's block.
            unsafe { provider.loadDataRepresentationForTypeIdentifier_completionHandler(&text, &read) };
        } else {
            return None;
        }
        Some(loading)
    }

    fn failure(error: *mut NSError) -> String {
        // SAFETY: the OS hands the block an error, or nil, whenever it hands it no item.
        let reason = unsafe { error.as_ref() }.map(|error| error.localizedDescription().to_string());
        format!("paste the clipboard: {}", reason.as_deref().unwrap_or("the OS gave no reason"))
    }

    /// Copies the OS's file of a pasted image, which it deletes once the block returns, into a folder of its own under
    /// `kept`, named `image` with the file's extension.
    fn keep(url: &NSURL, kept: &Path) -> Result<Piece, String> {
        static FOLDERS: AtomicUsize = AtomicUsize::new(0);
        let path = url.path().map(|path| PathBuf::from(path.to_string())).ok_or("the pasted image has no path")?;
        let copy = |error: std::io::Error| format!("keep the pasted image: {error}");
        let folder = kept.join(FOLDERS.fetch_add(1, Ordering::Relaxed).to_string());
        std::fs::create_dir_all(&folder).map_err(copy)?;
        let image = folder.join(Path::new("image").with_extension(path.extension().unwrap_or_default()));
        std::fs::copy(&path, &image).map_err(copy)?;
        Ok(Piece::File(image))
    }

    /// What the clipboard held once every item has loaded, in the clipboard's order, reporting the items that failed to.
    async fn collect(app: &AppHandle, loads: Vec<Loading>) -> Pasted {
        let mut pasted = Pasted::default();
        for load in loads {
            match load.await.unwrap_or_else(|_| Err("the OS dropped an item of the clipboard".to_owned())) {
                Ok(Piece::Text(text)) => pasted.text.push_str(&text),
                Ok(Piece::File(file)) => pasted.files.push(file),
                Err(problem) => app.state::<Arc<UiStore>>().report(problem),
            }
        }
        pasted
    }
}
