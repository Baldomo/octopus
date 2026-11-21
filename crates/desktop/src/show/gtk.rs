//! The show's platform on Linux: the monitors and the desktop's wallpaper, the pointer, the show's windows over
//! everything, a snapshot of octopus's own window, the cursor and the alert sound, on GTK's X11 backend.

use std::fs;
use std::path::{Path, PathBuf};

use gtk::gdk::{self, cairo};
use gtk::gdk_pixbuf::{Colorspace, InterpType, Pixbuf};
use gtk::gio;
use gtk::prelude::*;
use tauri::WebviewWindow;
use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};

use super::{Display, JPEG_QUALITY, ShowPoint, ShowRect, covering};

/// Where the desktop's sound themes keep an alert sound, in the order the show looks for one.
const ALERTS: [&str; 3] = [
    "/usr/share/sounds/freedesktop/stereo/dialog-warning.oga",
    "/usr/share/sounds/Yaru/stereo/dialog-warning.oga",
    "/usr/share/sounds/ocean/stereo/dialog-warning.oga",
];

/// The alert sound the infection plays as the computer seems to break: the desktop's own warning sound.
pub fn alert_sound() -> Option<&'static Path> {
    ALERTS.iter().map(Path::new).find(|sound| sound.exists())
}

/// Every monitor, in GDK's order, each with the desktop's wallpaper drawn to cover it.
pub fn displays() -> Vec<Display> {
    let Some(screen) = gdk::Display::default() else { return Vec::new() };
    let wallpaper = gnome_wallpaper().or_else(kde_wallpaper);
    (0..screen.n_monitors())
        .filter_map(|i| screen.monitor(i))
        .map(|monitor| {
            let area = monitor.geometry();
            let rect = ShowRect {
                x: area.x().into(),
                y: area.y().into(),
                width: area.width().into(),
                height: area.height().into(),
            };
            let scale = f64::from(monitor.scale_factor());
            let wallpaper =
                wallpaper.as_deref().and_then(|path| covered(path, rect.width * scale, rect.height * scale));
            Display { rect, wallpaper }
        })
        .collect()
}

/// The wallpaper GNOME, or a desktop built on its settings, shows: the dark one under a dark style.
fn gnome_wallpaper() -> Option<PathBuf> {
    let schemas = gio::SettingsSchemaSource::default()?;
    let background = schemas.lookup("org.gnome.desktop.background", true)?;
    let dark = schemas.lookup("org.gnome.desktop.interface", true).is_some_and(|interface| {
        interface.has_key("color-scheme")
            && gio::Settings::new("org.gnome.desktop.interface").string("color-scheme") == "prefer-dark"
    });
    let key = if dark && background.has_key("picture-uri-dark") { "picture-uri-dark" } else { "picture-uri" };
    gio::File::for_uri(&gio::Settings::new("org.gnome.desktop.background").string(key)).path()
}

/// The wallpaper KDE Plasma shows on its first desktop: an image, or the largest image of a wallpaper package.
fn kde_wallpaper() -> Option<PathBuf> {
    let config = fs::read_to_string(dirs::config_dir()?.join("plasma-org.kde.plasma.desktop-appletsrc")).ok()?;
    let image = config.lines().find_map(|line| line.strip_prefix("Image="))?;
    let path = PathBuf::from(image.strip_prefix("file://").unwrap_or(image));
    if !path.is_dir() {
        return Some(path);
    }
    fs::read_dir(path.join("contents/images"))
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .max_by_key(|image| image.metadata().map(|meta| meta.len()).unwrap_or(0))
}

/// The image at `path` scaled to cover `width` by `height` pixels and cut to them about its middle, as a JPEG data URL.
fn covered(path: &Path, width: f64, height: f64) -> Option<String> {
    let image = Pixbuf::from_file(path).ok()?;
    let natural = (f64::from(image.width()), f64::from(image.height()));
    let at = covering(natural, (width, height));
    let (w, h) = (width.round() as i32, height.round() as i32);
    let covered = Pixbuf::new(Colorspace::Rgb, false, 8, w, h)?;
    let k = at.width / natural.0;
    image.scale(&covered, 0, 0, w, h, at.x, at.y, k, k, InterpType::Bilinear);
    jpeg_url(&covered)
}

fn jpeg_url(image: &Pixbuf) -> Option<String> {
    let quality = ((JPEG_QUALITY * 100.0).round() as u32).to_string();
    let jpeg = image.save_to_bufferv("jpeg", &[("quality", quality.as_str())]).ok()?;
    Some(super::jpeg_url(&jpeg))
}

/// The pointer in the same logical coordinates as the monitors.
pub fn pointer() -> ShowPoint {
    let at = gdk::Display::default()
        .and_then(|screen| screen.default_seat())
        .and_then(|seat| seat.pointer())
        .map(|pointer| {
            let (_, x, y) = pointer.position_double();
            (x, y)
        })
        .unwrap_or_default();
    ShowPoint { x: at.0, y: at.1 }
}

/// Fills the show's window's monitor with it, above the desktop's panels.
pub fn cover(window: &WebviewWindow) -> anyhow::Result<()> {
    window.set_fullscreen(true)?;
    Ok(())
}

/// WebKitGTK keeps drawing a window the show covers, so octopus's layers need nothing to keep drawing.
pub fn keep_drawing(_: &tauri::Webview, _: bool) -> tauri::Result<()> {
    Ok(())
}

/// Hands `done` a JPEG data URL of the webview's picture, at the device pixels it draws, or none when WebKitGTK gives
/// no picture.
pub fn snapshot(
    webview: &tauri::Webview,
    _: f64,
    done: impl FnOnce(Option<String>) + Send + 'static,
) -> tauri::Result<()> {
    webview.with_webview(move |platform| {
        let view = platform.inner();
        view.snapshot(SnapshotRegion::Visible, SnapshotOptions::NONE, None::<&gio::Cancellable>, move |surface| {
            let picture = surface.ok().and_then(|surface| {
                let image = cairo::ImageSurface::try_from(surface).ok()?;
                jpeg_url(&gdk::pixbuf_get_from_surface(&image, 0, 0, image.width(), image.height())?)
            });
            done(picture);
        });
    })
}

/// Hides or shows the cursor over the show's windows, which cover every monitor while it plays.
pub fn cursor(windows: &[WebviewWindow], shown: bool) {
    for window in windows {
        if let Err(error) = window.set_cursor_visible(shown) {
            eprintln!("octopus app: {} the cursor: {error}", if shown { "show" } else { "hide" });
        }
    }
}
