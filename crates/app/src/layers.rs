//! The window stacks native webviews as layers, bottom to top: base, the UI's own webview filling the window; page,
//! the browser panel on screen at its rect; float, a second webview of the UI rendering every floating surface. A
//! webview draws over the webviews beneath it in its rect, and float stays above every page. Float covers what its
//! surfaces draw, shadows included, but takes the pointer only in their input rects, so a click or hover on a shadow
//! reaches the layer beneath. A host that cannot shape a webview's input bounds float by the input rects instead,
//! and the shadows end there. The keyboard sits in float while a modal is open and in base otherwise.
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
use tauri::{LogicalPosition, LogicalSize, Theme, Webview, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent};
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

/// A surface of the float layer that is out: it takes the pointer in `input` and draws in `drawn`, shadows included.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Floated {
    pub input: Bounds,
    pub drawn: Bounds,
}

/// From the float layer's corner to the far edges of every rect.
fn from_corner(rects: impl Iterator<Item = Bounds>) -> Bounds {
    let top = title_bar_height();
    let (right, bottom) = rects.fold((0.0_f64, top), |(right, bottom), rect| {
        (right.max(rect.x + rect.width), bottom.max(rect.y + rect.height))
    });
    Bounds { x: 0.0, y: top, width: right, height: bottom - top }
}

/// The float layer's rect and the rects in it that take the pointer, for the surfaces that are out.
pub fn float_cover(surfaces: &[Floated]) -> (Bounds, Vec<Bounds>) {
    let inputs = surfaces.iter().map(|surface| surface.input);
    if host::SHAPES_INPUT {
        let drawn = surfaces.iter().flat_map(|surface| [surface.input, surface.drawn]);
        (from_corner(drawn), inputs.collect())
    } else {
        let float = from_corner(inputs);
        (float, vec![float])
    }
}

/// Paints DESIGN.html's `--bg` of the OS appearance under the UI, so no frame shows another color.
fn paint(window: &Window, base: &Webview, theme: Theme) -> tauri::Result<()> {
    let bg = if theme == Theme::Dark { env!("OCTOPUS_BG_DARK") } else { env!("OCTOPUS_BG_LIGHT") };
    let color: tauri::window::Color = bg.parse().expect("build.rs emits --bg as two hex colors");
    window.set_background_color(Some(color))?;
    base.set_background_color(Some(color))
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
        let config =
            app.config().app.windows.iter().find(|window| window.label == BASE).context("tauri.conf.json window")?;
        let config = WindowConfig { traffic_light_position: Some(traffic_lights()), ..config.clone() };
        let base: Webview = WebviewWindowBuilder::from_config(app, &config)?.visible(false).build()?.as_ref().clone();
        let window = base.window();
        paint(&window, &base, window.theme()?)?;
        window.show()?;
        host::adopt_base(&base)?;
        let float = WebviewBuilder::new(FLOAT, WebviewUrl::App("index.html".into())).transparent(true);
        let float = add_child(&window, float, Some(ui.cover().float))?;
        #[cfg(target_os = "macos")]
        {
            let ui = Arc::clone(&ui);
            float.with_webview(move |platform| {
                if let Err(error) = pointer::install(platform.inner()) {
                    ui.report(format!("floating surfaces may block the pointer and flicker the cursor: {error}"));
                }
            })?;
        }
        let layers = Arc::new(Self { window: window.clone(), base, float, ui: Arc::clone(&ui), page: Arc::default() });
        let follower = Arc::clone(&layers);
        ui.follow(move |before, after| {
            follower.track(before, after).map_err(|error| format!("place the float layer: {error}"))
        });
        layers.resize()?;
        let watcher = Arc::clone(&layers);
        window.on_window_event(move |event| {
            let handled = match event {
                WindowEvent::Resized(_) => watcher.resize().map_err(|error| ("lay out", error)),
                WindowEvent::ScaleFactorChanged { .. } => {
                    watcher.resize().and_then(|()| watcher.replace()).map_err(|error| ("lay out", error))
                }
                WindowEvent::ThemeChanged(theme) => {
                    paint(&watcher.window, &watcher.base, *theme).map_err(|error| ("paint", error))
                }
                _ => Ok(()),
            };
            if let Err((job, error)) = handled {
                watcher.ui.report(format!("{job} the window: {error}"));
            }
        });
        Ok(layers)
    }

    fn track(&self, before: &Cover, after: &Cover) -> tauri::Result<()> {
        if after.float != before.float {
            host::place(&self.float, Some(after.float))?;
        }
        if after.input != before.input {
            host::take_input(&self.float, after.float, after.input.clone())?;
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
/// one raises it. macOS shapes a webview's input through its hit test; WebView2 takes every click in its window, so
/// Windows cannot.
#[cfg(not(target_os = "linux"))]
mod host {
    use tauri::{LogicalPosition, LogicalSize, Rect, Webview, Window};

    use super::Bounds;

    pub const SHAPES_INPUT: bool = cfg!(target_os = "macos");

    /// The webview at `bounds` takes the pointer only in `input`, rects of the window.
    #[cfg(target_os = "macos")]
    pub fn take_input(webview: &Webview, bounds: Bounds, input: Vec<Bounds>) -> tauri::Result<()> {
        webview.with_webview(move |_| super::pointer::take_input(bounds, input))
    }

    #[cfg(not(target_os = "macos"))]
    pub fn take_input(_: &Webview, _: Bounds, _: Vec<Bounds>) -> tauri::Result<()> {
        Ok(())
    }

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

    pub const SHAPES_INPUT: bool = true;

    /// The webview's window takes the pointer only in `input`, rects of the window, and the overlay passes every
    /// other point to the child beneath.
    pub fn take_input(webview: &Webview, bounds: Bounds, input: Vec<Bounds>) -> tauri::Result<()> {
        with_widget(webview, move |widget| {
            let region = gtk::cairo::Region::create();
            for rect in input {
                let (x, y) = (rect.x - bounds.x, rect.y - bounds.y);
                let (left, top) = (x.floor() as i32, y.floor() as i32);
                let (right, bottom) = ((x + rect.width).ceil() as i32, (y + rect.height).ceil() as i32);
                let local = gtk::cairo::RectangleInt::new(left, top, right - left, bottom - top);
                region.union_rectangle(&local).expect("cairo unions rectangles into a region it created");
            }
            overlay().set_overlay_pass_through(&widget, true);
            widget.input_shape_combine_region(Some(&region));
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

/// AppKit asks each view's hit test which view takes a click or a scroll, and sends each webview's tracking observer
/// every pointer event in the webview's rect, even where a webview above covers it; the observer has WebKit set the
/// cursor for its own content under the pointer. The float webview's hit test finds nothing outside its input rects,
/// so AppKit hands those points to the view beneath, and an observer passes an event on only while its webview is
/// the topmost view under the pointer, so only that webview tracks the pointer and sets the cursor. The float webview
/// also gets the first move after it stops being topmost: that move lands in its own transparent area, which shows
/// its page that the pointer left its surfaces.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod pointer {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::sync::OnceLock;

    use anyhow::Context;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::{MainThreadMarker, sel};
    use objc2_app_kit::{NSEvent, NSView};
    use objc2_core_foundation::CGPoint;

    use super::Bounds;

    type PointerEvent = unsafe extern "C-unwind" fn(&AnyObject, Sel, &NSEvent);
    type HitTest = unsafe extern "C-unwind" fn(&NSView, Sel, CGPoint) -> *mut NSView;

    static WEBKIT_HIT_TEST: OnceLock<HitTest> = OnceLock::new();
    static WEBKIT_MOUSE_MOVED: OnceLock<PointerEvent> = OnceLock::new();
    static WEBKIT_MOUSE_ENTERED: OnceLock<PointerEvent> = OnceLock::new();
    static WEBKIT_MOUSE_EXITED: OnceLock<PointerEvent> = OnceLock::new();

    thread_local! {
        /// The float webview, and the rects of it, in its own flipped coordinates, that take the pointer.
        static FLOAT: RefCell<Option<(Retained<NSView>, Vec<Bounds>)>> = const { RefCell::new(None) };
        /// Whether each observer's webview was topmost at the last pointer event the observer saw, by address.
        static TOPMOST: RefCell<HashMap<usize, bool>> = RefCell::new(HashMap::new());
    }

    /// Puts `gate` in place of `class`'s implementation of `selector`, and keeps the one it replaced in `saved`.
    fn swap(class: &AnyClass, selector: Sel, gate: Imp, saved: impl FnOnce(Imp) -> bool) -> anyhow::Result<()> {
        let method = class.instance_method(selector).with_context(|| format!("{class} has no {selector}"))?;
        // SAFETY: every gate has the signature of the implementation it takes the place of.
        let webkit = unsafe { method.set_implementation(gate) };
        anyhow::ensure!(saved(webkit), "the pointer gates install once");
        Ok(())
    }

    /// Takes the float webview's pointer, given as the platform's `WKWebView`, on the main thread.
    pub fn install(float: *mut c_void) -> anyhow::Result<()> {
        // SAFETY: tauri hands over its live WKWebView.
        let float = unsafe { Retained::retain(float.cast::<NSView>()) }.context("the float layer has no WKWebView")?;
        let hit_test: HitTest = hit_test;
        // SAFETY: an `Imp` is any function pointer, and `HitTest` matches `hitTest:`.
        swap(float.class(), sel!(hitTest:), unsafe { std::mem::transmute::<HitTest, Imp>(hit_test) }, |webkit| {
            // SAFETY: WebKit's `hitTest:` takes the receiver, its selector and a point, and returns a view or nil.
            WEBKIT_HIT_TEST.set(unsafe { std::mem::transmute::<Imp, HitTest>(webkit) }).is_ok()
        })?;
        FLOAT.with_borrow_mut(|shaped| *shaped = Some((float, Vec::new())));
        let observer = AnyClass::get(c"WKMouseTrackingObserver").context("WebKit has no WKMouseTrackingObserver")?;
        let gates: [(Sel, PointerEvent, &OnceLock<PointerEvent>); 3] = [
            (sel!(mouseMoved:), mouse_moved, &WEBKIT_MOUSE_MOVED),
            (sel!(mouseEntered:), mouse_entered, &WEBKIT_MOUSE_ENTERED),
            (sel!(mouseExited:), mouse_exited, &WEBKIT_MOUSE_EXITED),
        ];
        for (selector, gate, saved) in gates {
            // SAFETY: an `Imp` is any function pointer, and `PointerEvent` matches each of these selectors.
            swap(observer, selector, unsafe { std::mem::transmute::<PointerEvent, Imp>(gate) }, |webkit| {
                // SAFETY: each takes the receiver, its selector and an event, and returns nothing.
                saved.set(unsafe { std::mem::transmute::<Imp, PointerEvent>(webkit) }).is_ok()
            })?;
        }
        Ok(())
    }

    /// The float webview at `bounds` takes the pointer only in `input`, rects of the window; runs on the main thread.
    pub fn take_input(bounds: Bounds, input: Vec<Bounds>) {
        let local = input.into_iter().map(|rect| Bounds { x: rect.x - bounds.x, y: rect.y - bounds.y, ..rect });
        FLOAT.with_borrow_mut(|float| {
            if let Some((_, taken)) = float {
                *taken = local.collect();
            }
        });
    }

    unsafe extern "C-unwind" fn hit_test(view: &NSView, selector: Sel, point: CGPoint) -> *mut NSView {
        let passes = FLOAT.with_borrow(|float| {
            float.as_ref().is_some_and(|(float, input)| std::ptr::eq(&**float, view) && !takes(view, point, input))
        });
        if passes {
            return std::ptr::null_mut();
        }
        let webkit = WEBKIT_HIT_TEST.get().expect("the gate replaces WebKit's hitTest: after saving it");
        // SAFETY: WebKit's own implementation, called with the arguments AppKit passed.
        unsafe { webkit(view, selector, point) }
    }

    /// Whether `point`, in the coordinates of the view's superview, falls in one of `input`. A WKWebView is flipped,
    /// so its own coordinates run down from its top-left corner, as the rects do.
    fn takes(view: &NSView, point: CGPoint, input: &[Bounds]) -> bool {
        // SAFETY: a view in the window's hierarchy holds its superview for as long as it is attached.
        let superview = unsafe { view.superview() };
        let CGPoint { x, y } = view.convertPoint_fromView(point, superview.as_deref());
        input.iter().any(|rect| x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height)
    }

    fn key(observer: &AnyObject) -> usize {
        std::ptr::from_ref(observer) as usize
    }

    /// Records whether the observer's webview is topmost for this event, and returns whether it was at the last one.
    fn now_topmost(observer: &AnyObject, topmost: bool) -> bool {
        TOPMOST.with_borrow_mut(|seen| seen.insert(key(observer), topmost)).unwrap_or(false)
    }

    /// # Safety
    /// The arguments are the ones AppKit passed to the gate of `webkit`'s selector.
    unsafe fn forward(webkit: &OnceLock<PointerEvent>, observer: &AnyObject, selector: Sel, event: &NSEvent) {
        let webkit = webkit.get().expect("each gate replaces WebKit's implementation after saving it");
        // SAFETY: WebKit's own implementation, called with the arguments AppKit passed.
        unsafe { webkit(observer, selector, event) }
    }

    unsafe extern "C-unwind" fn mouse_moved(observer: &AnyObject, selector: Sel, event: &NSEvent) {
        let topmost = observes_topmost_view(observer, event);
        let left = now_topmost(observer, topmost) && observes_float(observer);
        if topmost || left {
            // SAFETY: the arguments AppKit passed.
            unsafe { forward(&WEBKIT_MOUSE_MOVED, observer, selector, event) }
        }
    }

    unsafe extern "C-unwind" fn mouse_entered(observer: &AnyObject, selector: Sel, event: &NSEvent) {
        let topmost = observes_topmost_view(observer, event);
        now_topmost(observer, topmost);
        if topmost {
            // SAFETY: the arguments AppKit passed.
            unsafe { forward(&WEBKIT_MOUSE_ENTERED, observer, selector, event) }
        }
    }

    unsafe extern "C-unwind" fn mouse_exited(observer: &AnyObject, selector: Sel, event: &NSEvent) {
        if TOPMOST.with_borrow_mut(|seen| seen.remove(&key(observer))).unwrap_or(false) {
            // SAFETY: the arguments AppKit passed.
            unsafe { forward(&WEBKIT_MOUSE_EXITED, observer, selector, event) }
        }
    }

    fn observes_topmost_view(observer: &AnyObject, event: &NSEvent) -> bool {
        let main_thread = MainThreadMarker::new().expect("AppKit delivers pointer events on the main thread");
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

    fn observes_float(observer: &AnyObject) -> bool {
        FLOAT.with_borrow(|float| float.as_ref().is_some_and(|(float, _)| observes(observer, float)))
    }

    fn observes(observer: &AnyObject, view: &NSView) -> bool {
        view.trackingAreas().iter().any(|area| area.owner().is_some_and(|owner| std::ptr::eq(&*owner, observer)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_float_layer_grows_from_a_fixed_corner_and_takes_the_pointer_only_where_its_surfaces_do() {
        let y = title_bar_height();
        let column = Bounds { x: 0.0, y, width: 294.0, height: 828.0 };
        let sidebar = Floated { input: column, drawn: Bounds { x: -40.0, y: 22.0, width: 382.0, height: 900.0 } };
        let scrim = Bounds { x: 0.0, y, width: 1440.0, height: 862.0 };
        let modal = Floated { input: scrim, drawn: scrim };
        assert_eq!(float_cover(&[]).0, Bounds { x: 0.0, y, width: 0.0, height: 0.0 });
        if host::SHAPES_INPUT {
            assert_eq!(float_cover(&[sidebar]), (Bounds { x: 0.0, y, width: 342.0, height: 922.0 - y }, vec![column]));
            assert_eq!(float_cover(&[sidebar, modal]), (Bounds { height: 922.0 - y, ..scrim }, vec![column, scrim]));
        } else {
            assert_eq!(float_cover(&[sidebar]), (column, vec![column]));
            assert_eq!(float_cover(&[sidebar, modal]), (scrim, vec![scrim]));
        }
    }
}
