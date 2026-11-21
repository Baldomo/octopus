//! The show's platform on macOS, all without a permission prompt: the screens and their wallpapers, the pointer, the
//! show's windows over everything, a snapshot of octopus's own window, the cursor and the alert sound.

use std::cell::RefCell;
use std::path::Path;

use anyhow::Context;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker};
use objc2::{msg_send, sel};
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSColor, NSCompositingOperation, NSCursor, NSDeviceRGBColorSpace,
    NSGraphicsContext, NSImage, NSImageCompressionFactor, NSScreen, NSScreenSaverWindowLevel, NSWindow,
    NSWindowCollectionBehavior, NSWorkspace,
};
use objc2_core_graphics::CGEvent;
use objc2_foundation::{NSData, NSDictionary, NSError, NSHomeDirectory, NSNumber, NSPoint, NSRect, NSSize};
use tauri::WebviewWindow;

use super::{Display, JPEG_QUALITY, ShowPoint, ShowRect, covering};

thread_local! {
    static HIDDEN: RefCell<bool> = const { RefCell::new(false) };
}

/// The alert sound the infection plays as the Mac seems to break.
pub fn alert_sound() -> Option<&'static Path> {
    Some(Path::new("/System/Library/Sounds/Basso.aiff")).filter(|sound| sound.exists())
}

/// The main display's height, which turns AppKit's bottom-left origin into the show's top-left one.
fn main_height(mtm: MainThreadMarker) -> f64 {
    NSScreen::screens(mtm).firstObject().map_or(0.0, |main| main.frame().size.height)
}

fn top_left(rect: NSRect, main: f64) -> ShowRect {
    ShowRect {
        x: rect.origin.x,
        y: main - rect.origin.y - rect.size.height,
        width: rect.size.width,
        height: rect.size.height,
    }
}

/// The pointer in global points from the main display's top left.
pub fn pointer() -> ShowPoint {
    let at = CGEvent::new(None).map(|event| CGEvent::location(Some(&event))).unwrap_or_default();
    ShowPoint { x: at.x, y: at.y }
}

/// Every display, in the order of NSScreen's screens.
pub fn displays() -> Vec<Display> {
    let mtm = MainThreadMarker::new().expect("the show reads the displays on the main thread");
    let main = main_height(mtm);
    NSScreen::screens(mtm)
        .iter()
        .map(|screen| Display { rect: top_left(screen.frame(), main), wallpaper: wallpaper(&screen) })
        .collect()
}

/// The screen's wallpaper drawn to cover the screen, as a JPEG data URL; none when it sits where reading it would
/// make macOS ask for access to a protected folder.
fn wallpaper(screen: &NSScreen) -> Option<String> {
    let url = NSWorkspace::sharedWorkspace().desktopImageURLForScreen(screen)?;
    let path = url.path()?.to_string();
    let home = NSHomeDirectory().to_string();
    let guarded = ["Desktop", "Documents", "Downloads", "Library/Mobile Documents", "Pictures/Photos Library"];
    if guarded.iter().any(|folder| Path::new(&path).starts_with(Path::new(&home).join(folder)))
        || path.starts_with("/Volumes/")
    {
        return None;
    }
    let image = NSImage::initWithContentsOfURL(NSImage::alloc(), &url)?;
    let size = screen.frame().size;
    let scale = screen.backingScaleFactor();
    let (width, height) = ((size.width * scale).round(), (size.height * scale).round());
    let natural = image.size();
    let at = covering((natural.width, natural.height), (width, height));
    jpeg_url(&image, width, height, NSRect::new(NSPoint::new(at.x, at.y), NSSize::new(at.width, at.height)))
}

/// The image drawn at `rect` in a `width` by `height` bitmap, as a JPEG data URL.
fn jpeg_url(image: &NSImage, width: f64, height: f64, rect: NSRect) -> Option<String> {
    let (width, height) = (width as isize, height as isize);
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            width,
            height,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            0,
            0,
        )
    }?;
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&context));
    image.drawInRect_fromRect_operation_fraction(rect, NSRect::ZERO, NSCompositingOperation::Copy, 1.0);
    NSGraphicsContext::restoreGraphicsState_class();
    let quality = NSNumber::new_f64(JPEG_QUALITY);
    let properties = NSDictionary::from_slices(&[unsafe { NSImageCompressionFactor }], &[&*quality as &AnyObject]);
    let data: Retained<NSData> =
        unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::JPEG, &properties) }?;
    Some(super::jpeg_url(&data.to_vec()))
}

/// Whether the webview keeps drawing while the show covers it: WebKit on macOS pauses a webview whose window is
/// covered.
pub fn keep_drawing(webview: &tauri::Webview, always: bool) -> tauri::Result<()> {
    let detect = !always;
    webview.with_webview(move |platform| {
        let view: &AnyObject = unsafe { &*platform.inner().cast() };
        let setter = sel!(_setWindowOcclusionDetectionEnabled:);
        if view.class().responds_to(setter) {
            let _: () = unsafe { msg_send![view, _setWindowOcclusionDetectionEnabled: detect] };
        }
    })
}

/// Hands `done` a JPEG data URL of the webview's picture at `scale`, or none when WebKit gives no picture.
pub fn snapshot(
    webview: &tauri::Webview,
    scale: f64,
    done: impl FnOnce(Option<String>) + Send + 'static,
) -> tauri::Result<()> {
    let done = std::sync::Mutex::new(Some(done));
    webview.with_webview(move |platform| {
        let view: &AnyObject = unsafe { &*platform.inner().cast() };
        let block = block2::RcBlock::new(move |image: *mut NSImage, _error: *mut NSError| {
            let picture = unsafe { image.as_ref() }.and_then(|image| {
                let size = image.size();
                let (width, height) = ((size.width * scale).round(), (size.height * scale).round());
                jpeg_url(image, width, height, NSRect::new(NSPoint::ZERO, NSSize::new(width, height)))
            });
            if let Some(done) = done.lock().ok().and_then(|mut done| done.take()) {
                done(picture);
            }
        });
        let config: *const AnyObject = std::ptr::null();
        let _: () = unsafe { msg_send![view, takeSnapshotWithConfiguration: config, completionHandler: &*block] };
    })
}

/// Lifts the show's window above everything on its display, on every space, with no background or shadow of its own.
pub fn cover(window: &WebviewWindow) -> anyhow::Result<()> {
    let ns_window = window.ns_window().context("the show's window has no NSWindow")?;
    let ns_window: &NSWindow = unsafe { &*ns_window.cast() };
    ns_window.setOpaque(false);
    ns_window.setBackgroundColor(Some(&NSColor::clearColor()));
    ns_window.setHasShadow(false);
    ns_window.setLevel(NSScreenSaverWindowLevel);
    ns_window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    Ok(())
}

/// Hides or shows the cursor everywhere, keeping AppKit's hide count balanced; the show's windows need not be named.
pub fn cursor(_: &[WebviewWindow], shown: bool) {
    HIDDEN.with_borrow_mut(|hidden| {
        if shown && *hidden {
            NSCursor::unhide();
        }
        if !shown && !*hidden {
            NSCursor::hide();
        }
        *hidden = !shown;
    });
}
