//! The window stacks native webviews as layers, bottom to top: base, the UI's own webview filling the window; page,
//! the browser panel on screen at its rect; float, a second webview of the UI rendering every floating surface. A
//! webview draws over and takes every click and pointer move in its rect above those beneath it, so each layer's
//! bounds are exactly what it shows, and float stays above every page. The keyboard sits in float while a modal is
//! open and in base otherwise.
//!
//! A webview's native rect and the content it draws change in different processes, so no frame may depend on both
//! changing at once. The float layer's top-left corner never moves: its surfaces lay out in window coordinates, and
//! the layer only grows and shrinks from that corner. It never hides either, because a hidden webview stops rendering
//! and shows a stale frame when it returns; it collapses to nothing at its corner instead. The page layer swaps one
//! page for another in a single turn of the main thread, so no frame shows both or neither.

use std::sync::{Arc, Mutex};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tauri::utils::config::{LogicalPosition as ConfigPosition, WindowConfig};
use tauri::webview::WebviewBuilder;
use tauri::{LogicalPosition, LogicalSize, Webview, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent};
use ts_rs::TS;

use crate::lock;
use crate::ui::{Cover, UiStore};

/// The label of the window and of its base webview, as `tauri.conf.json` declares them.
pub const BASE: &str = "main";
pub const FLOAT: &str = "float";

/// A rect of the window, in the UI's CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The window's size in CSS pixels, which the float layer lays its surfaces out in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, TS)]
pub struct Viewport {
    pub width: f64,
    pub height: f64,
}

/// The float layer's corner sits on the window's left edge at this height, just below the title bar.
pub fn title_bar_height() -> f64 {
    env!("OCTOPUS_CHROME_H").parse().expect("build.rs emits --chrome-h in px")
}

const TRAFFIC_LIGHTS_INSET: f64 = 14.0;
const TITLE_BAR_BORDER: f64 = 1.0;
const APPKIT_LIGHT_BELOW_GIVEN_Y: f64 = 2.0;

/// The position that centers the traffic lights on the title bar's midline, above its bottom border.
fn traffic_lights() -> ConfigPosition {
    let midline = (title_bar_height() - TITLE_BAR_BORDER) / 2.0;
    ConfigPosition { x: TRAFFIC_LIGHTS_INSET, y: midline - APPKIT_LIGHT_BELOW_GIVEN_Y }
}

/// The float layer's rect: from its corner to the far edges of every surface that is out.
pub fn float_bounds(surfaces: impl Iterator<Item = Bounds>) -> Bounds {
    let top = title_bar_height();
    let (right, bottom) = surfaces.fold((0.0_f64, top), |(right, bottom), rect| {
        (right.max(rect.x + rect.width), bottom.max(rect.y + rect.height))
    });
    Bounds { x: 0.0, y: top, width: right, height: bottom - top }
}

pub struct Layers {
    window: Window,
    base: Webview,
    float: Webview,
    ui: Arc<UiStore>,
    /// The page on screen, at its rect.
    page: Arc<Mutex<Option<(Webview, Bounds)>>>,
}

fn viewport(window: &Window) -> tauri::Result<Viewport> {
    let size = window.inner_size()?.to_logical::<f64>(window.scale_factor()?);
    Ok(Viewport { width: size.width, height: size.height })
}

/// Runs `act` in one turn of the main thread, so every native change it makes lands in the same frame.
fn on_main_thread<T: Send + 'static>(
    window: &Window,
    act: impl FnOnce() -> tauri::Result<T> + Send + 'static,
) -> tauri::Result<T> {
    let (done, acted) = std::sync::mpsc::channel();
    window.run_on_main_thread(move || done.send(act()).expect("on_main_thread waits for its act"))?;
    acted.recv().map_err(|_| tauri::Error::FailedToReceiveMessage)?
}

/// A child webview of the window without the keyboard, moved into its layer and placed in the turn of the main thread
/// that creates it, so no frame lays the window out with it anywhere else.
fn add_child(window: &Window, webview: WebviewBuilder<tauri::Wry>, at: Option<Bounds>) -> tauri::Result<Webview> {
    let parent = window.clone();
    on_main_thread(window, move || {
        let webview =
            parent.add_child(webview.focused(false), LogicalPosition::new(0.0, 0.0), LogicalSize::new(0.0, 0.0))?;
        host::adopt(&webview)?;
        host::place(&webview, at)?;
        Ok(webview)
    })
}

impl Layers {
    /// Opens the window with its base and float layers; from then on the float layer follows the shared state.
    pub fn open(app: &tauri::App, ui: Arc<UiStore>) -> anyhow::Result<Arc<Self>> {
        let background: tauri::window::Color =
            env!("OCTOPUS_BG").parse().map_err(|error: String| anyhow::anyhow!("DESIGN.html --bg: {error}"))?;
        let config =
            app.config().app.windows.iter().find(|window| window.label == BASE).context("tauri.conf.json window")?;
        let config = WindowConfig { traffic_light_position: Some(traffic_lights()), ..config.clone() };
        let base: Webview =
            WebviewWindowBuilder::from_config(app, &config)?.background_color(background).build()?.as_ref().clone();
        let window = base.window();
        host::adopt_base(&base)?;
        #[cfg(target_os = "macos")]
        if let Err(error) = pointer::track_only_where_topmost() {
            ui.report(format!("the cursor may flicker over floating surfaces: {error}"));
        }
        let float = WebviewBuilder::new(FLOAT, WebviewUrl::App("index.html".into())).transparent(true);
        let float = add_child(&window, float, Some(ui.cover().float))?;
        let layers = Arc::new(Self { window: window.clone(), base, float, ui: Arc::clone(&ui), page: Arc::default() });
        let follower = Arc::clone(&layers);
        ui.follow(move |before, after| {
            follower.track(before, after).map_err(|error| format!("place the float layer: {error}"))
        });
        layers.resize()?;
        let watcher = Arc::clone(&layers);
        window.on_window_event(move |event| {
            let laid_out = match event {
                WindowEvent::Resized(_) => watcher.resize(),
                WindowEvent::ScaleFactorChanged { .. } => watcher.resize().and_then(|()| watcher.replace()),
                _ => Ok(()),
            };
            if let Err(error) = laid_out {
                watcher.ui.report(format!("lay out the window: {error}"));
            }
        });
        Ok(layers)
    }

    fn track(&self, before: &Cover, after: &Cover) -> tauri::Result<()> {
        if after.float != before.float {
            host::place(&self.float, Some(after.float))?;
        }
        match (before.modal, after.modal) {
            (false, true) => self.float.set_focus(),
            (true, false) => self.base.set_focus(),
            _ => Ok(()),
        }
    }

    fn resize(&self) -> tauri::Result<()> {
        self.ui.resize(viewport(&self.window)?);
        Ok(())
    }

    /// A webview takes its rect in device pixels, so a new scale re-applies every layer's rect.
    fn replace(&self) -> tauri::Result<()> {
        host::place(&self.float, Some(self.ui.cover().float))?;
        lock(&self.page).as_ref().map_or(Ok(()), |(page, bounds)| host::place(page, Some(*bounds)))
    }

    /// Adds a page webview, hidden until shown, beneath the float layer.
    pub fn add_page(&self, page: WebviewBuilder<tauri::Wry>) -> tauri::Result<Webview> {
        let page = add_child(&self.window, page, None)?;
        host::raise(&self.window, &self.float)?;
        if self.ui.cover().modal {
            self.float.set_focus()?;
        }
        Ok(page)
    }

    /// Puts `next` on screen at its rect in place of the page there, or empties the page layer. The main thread
    /// serializes every swap, and a page that fails to land still lets the shown one leave.
    pub fn show_page(&self, next: Option<(Webview, Bounds)>) -> tauri::Result<()> {
        let page = Arc::clone(&self.page);
        on_main_thread(&self.window, move || {
            let mut page = lock(&page);
            let shown = std::mem::replace(&mut *page, next.clone()).map(|(shown, _)| shown);
            let landed = next.as_ref().map_or(Ok(()), |(next, bounds)| host::place(next, Some(*bounds)));
            let stays = |shown: &Webview| next.as_ref().is_some_and(|(next, _)| next.label() == shown.label());
            let left = shown.filter(|shown| !stays(shown)).map_or(Ok(()), |shown| host::place(&shown, None));
            landed.and(left)
        })
    }

    pub fn close_page(&self, closed: &Webview) -> tauri::Result<()> {
        lock(&self.page).take_if(|(shown, _)| shown.label() == closed.label());
        closed.close()
    }
}

/// macOS and Windows stack a window's webviews as native child views in the order they were added, and re-adding
/// one raises it.
#[cfg(not(target_os = "linux"))]
mod host {
    use tauri::{LogicalPosition, LogicalSize, Rect, Webview, Window};

    use super::Bounds;

    pub fn adopt_base(_: &Webview) -> tauri::Result<()> {
        Ok(())
    }

    pub fn adopt(_: &Webview) -> tauri::Result<()> {
        Ok(())
    }

    pub fn place(webview: &Webview, bounds: Option<Bounds>) -> tauri::Result<()> {
        match bounds {
            Some(bounds) => {
                webview.set_bounds(Rect {
                    position: LogicalPosition::new(bounds.x, bounds.y).into(),
                    size: LogicalSize::new(bounds.width, bounds.height).into(),
                })?;
                webview.show()
            }
            None => webview.hide(),
        }
    }

    pub fn raise(window: &Window, webview: &Webview) -> tauri::Result<()> {
        webview.reparent(window)
    }
}

/// Tauri packs a window's webviews side by side into its `gtk::Box`, where no webview takes a position. octopus
/// moves the base webview into a `gtk::Overlay` in its place and every other webview onto that overlay, which puts
/// each child at the rect octopus holds for it and stacks them in the overlay's order.
#[cfg(target_os = "linux")]
mod host {
    use std::cell::{OnceCell, RefCell};
    use std::collections::HashMap;

    use gtk::prelude::*;
    use tauri::{Webview, Window};

    use super::Bounds;

    thread_local! {
        static OVERLAY: OnceCell<gtk::Overlay> = const { OnceCell::new() };
        static RECTS: RefCell<HashMap<gtk::Widget, gtk::gdk::Rectangle>> = RefCell::new(HashMap::new());
    }

    fn overlay() -> gtk::Overlay {
        OVERLAY.with(|overlay| overlay.get().expect("the base webview is adopted first").clone())
    }

    /// Runs `act` on the main thread with the webview's widget.
    fn with_widget(webview: &Webview, act: impl FnOnce(gtk::Widget) + Send + 'static) -> tauri::Result<()> {
        webview.with_webview(move |platform| act(platform.inner().upcast()))
    }

    fn packed_box(widget: &gtk::Widget) -> gtk::Box {
        widget.parent().and_then(|parent| parent.downcast().ok()).expect("tauri packs a webview into its window's box")
    }

    pub fn adopt_base(base: &Webview) -> tauri::Result<()> {
        with_widget(base, |widget| {
            let packed = packed_box(&widget);
            packed.remove(&widget);
            let overlay = gtk::Overlay::new();
            overlay.add(&widget);
            overlay.connect_get_child_position(|_, child| RECTS.with(|rects| rects.borrow().get(child).cloned()));
            packed.pack_start(&overlay, true, true, 0);
            overlay.show();
            OVERLAY.with(|cell| cell.set(overlay).expect("one base webview"));
        })
    }

    pub fn adopt(webview: &Webview) -> tauri::Result<()> {
        with_widget(webview, |widget| {
            packed_box(&widget).remove(&widget);
            overlay().add_overlay(&widget);
            widget.connect_destroy(|widget| {
                RECTS.with(|rects| rects.borrow_mut().remove(widget));
            });
        })
    }

    pub fn place(webview: &Webview, bounds: Option<Bounds>) -> tauri::Result<()> {
        with_widget(webview, move |widget| {
            match bounds {
                Some(bounds) => {
                    let rect = gtk::gdk::Rectangle::new(
                        bounds.x.round() as i32,
                        bounds.y.round() as i32,
                        bounds.width.round() as i32,
                        bounds.height.round() as i32,
                    );
                    RECTS.with(|rects| rects.borrow_mut().insert(widget.clone(), rect));
                    widget.show();
                }
                None => widget.hide(),
            }
            overlay().queue_resize();
        })
    }

    /// The overlay restacks its children's windows in its order on every allocation.
    pub fn raise(_: &Window, webview: &Webview) -> tauri::Result<()> {
        with_widget(webview, |widget| {
            let overlay = overlay();
            overlay.reorder_overlay(&widget, -1);
            overlay.queue_resize();
        })
    }
}

/// AppKit sends each webview's tracking observer every mouse move in the webview's rect, even where a webview above
/// covers it, and the observer has WebKit set the cursor for its own content under the pointer. Every observer passes
/// a mouse move on only when its webview is the topmost view under the pointer, so only the top layer sets the cursor.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod pointer {
    use std::sync::OnceLock;

    use anyhow::Context;
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::{MainThreadMarker, sel};
    use objc2_app_kit::{NSEvent, NSView};

    type MouseMoved = unsafe extern "C-unwind" fn(&AnyObject, Sel, &NSEvent);

    static WEBKIT_MOUSE_MOVED: OnceLock<MouseMoved> = OnceLock::new();

    pub fn track_only_where_topmost() -> anyhow::Result<()> {
        let observer = AnyClass::get(c"WKMouseTrackingObserver").context("WebKit has no WKMouseTrackingObserver")?;
        let method =
            observer.instance_method(sel!(mouseMoved:)).context("WKMouseTrackingObserver has no mouseMoved:")?;
        // SAFETY: `mouseMoved:` takes the receiver, its selector and an event, and returns nothing.
        let webkit = unsafe { std::mem::transmute::<Imp, MouseMoved>(method.implementation()) };
        WEBKIT_MOUSE_MOVED.set(webkit).ok().context("the pointer gate installs once")?;
        let gated: MouseMoved = mouse_moved;
        // SAFETY: the gate has the signature of the implementation it takes the place of.
        unsafe { method.set_implementation(std::mem::transmute::<MouseMoved, Imp>(gated)) };
        Ok(())
    }

    unsafe extern "C-unwind" fn mouse_moved(observer: &AnyObject, selector: Sel, event: &NSEvent) {
        if observes_topmost_view(observer, event) {
            let webkit = WEBKIT_MOUSE_MOVED.get().expect("the gate replaces WebKit's mouseMoved: after saving it");
            // SAFETY: WebKit's own implementation, called with the arguments AppKit passed.
            unsafe { webkit(observer, selector, event) }
        }
    }

    fn observes_topmost_view(observer: &AnyObject, event: &NSEvent) -> bool {
        let main_thread = MainThreadMarker::new().expect("AppKit delivers mouse moves on the main thread");
        let content = event.window(main_thread).and_then(|window| window.contentView());
        let mut view = content.and_then(|content| content.hitTest(event.locationInWindow()));
        while let Some(shown) = view {
            if observes(observer, &shown) {
                return true;
            }
            // SAFETY: a view in the window's hierarchy holds its superview for as long as it is attached.
            view = unsafe { shown.superview() };
        }
        false
    }

    fn observes(observer: &AnyObject, view: &NSView) -> bool {
        view.trackingAreas().iter().any(|area| area.owner().is_some_and(|owner| std::ptr::eq(&*owner, observer)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_float_layer_grows_and_shrinks_from_a_corner_that_never_moves() {
        let y = title_bar_height();
        let sidebar = Bounds { x: 8.0, y: 46.0, width: 286.0, height: 820.0 };
        let scrim = Bounds { x: 0.0, y, width: 1440.0, height: 862.0 };
        assert_eq!(float_bounds(std::iter::empty()), Bounds { x: 0.0, y, width: 0.0, height: 0.0 });
        assert_eq!(float_bounds([sidebar].into_iter()), Bounds { x: 0.0, y, width: 294.0, height: 866.0 - y });
        assert_eq!(float_bounds([sidebar, scrim].into_iter()), scrim);
    }
}
