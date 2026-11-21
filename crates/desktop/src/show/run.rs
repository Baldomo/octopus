//! The show's run: a window over each display, and the show from the moment every page is ready to its end.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use octopus_client::demo::{self, Computer};
use octopus_shell::hub::Hub;
use octopus_shell::lock;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use super::{
    APP_EVENT, DRIVE_EVENT, END_EVENT, PLAY_EVENT, POINTER_EVENT, SPOT_EVENT, ShowApp, ShowDrive, ShowRect, ShowScene,
    ShowSpot, platform, sound,
};
use crate::layers::{BASE, FLOAT};

/// The seconds every display takes to start its first frame once the song is due.
const LEAD: f64 = 0.4;
/// The seconds the last frame stays after the song ends.
const TAIL: f64 = 0.1;
/// How long the pages have to get ready before the show ends unplayed.
const READY_BY: Duration = Duration::from_secs(15);
const POINTER_POLL: Duration = Duration::from_millis(16);
/// The display whose page drives the show and takes the keys.
const DRIVING: usize = 0;

/// The label of the page over `display` in the show's `play`th play.
fn page(play: u64, display: usize) -> String {
    format!("show-{play}-{display}")
}

/// One play of the show, from its windows opening to its end.
pub(crate) struct Show {
    app: AppHandle,
    /// The driving display's page.
    driver: String,
    scenes: HashMap<String, ShowScene>,
    ready: Mutex<HashSet<String>>,
    started: AtomicBool,
    toured: AtomicBool,
    ended: AtomicBool,
    sound: Mutex<Option<sound::Sound>>,
}

impl Show {
    /// Opens a show window over every display and lists the blank tidepool; none when there is no display. A window
    /// that fails to open closes every one opened.
    pub(super) fn start(app: &AppHandle, play: u64) -> anyhow::Result<Option<Arc<Self>>> {
        let scenes = scenes();
        if scenes.is_empty() {
            return Ok(None);
        }
        if let Err(error) =
            scenes.iter().enumerate().try_for_each(|(display, scene)| cover(app, &page(play, display), scene))
        {
            for display in 0..scenes.len() {
                if let Some(window) = app.get_webview_window(&page(play, display)) {
                    let _ = window.destroy();
                }
            }
            return Err(error);
        }
        let show = Arc::new(Self {
            app: app.clone(),
            driver: page(play, DRIVING),
            scenes: scenes.into_iter().enumerate().map(|(display, scene)| (page(play, display), scene)).collect(),
            ready: Mutex::default(),
            started: AtomicBool::new(false),
            toured: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            sound: Mutex::default(),
        });
        if let Err(error) = demo::pair(&show.app.state::<Arc<Hub>>().me, Computer::Blank) {
            eprintln!("octopus app: list tidepool for the show: {error:#}");
        }
        let waiting = Arc::clone(&show);
        thread::spawn(move || {
            thread::sleep(READY_BY);
            if !waiting.started.swap(true, Ordering::SeqCst) {
                waiting.end();
            }
        });
        Ok(Some(show))
    }

    pub(crate) fn playing(&self) -> bool {
        !self.ended.load(Ordering::SeqCst)
    }

    pub(crate) fn covers(&self, page: &str) -> bool {
        self.scenes.contains_key(page)
    }

    /// The scene of `page`, one of the show's own.
    pub(crate) fn scene(&self, page: &str) -> ShowScene {
        self.scenes[page].clone()
    }

    /// Marks `page` ready; the show starts once every page is.
    pub(crate) fn ready(self: &Arc<Self>, page: &str) {
        let all = {
            let mut ready = lock(&self.ready);
            ready.insert(page.to_string());
            ready.len() == self.scenes.len()
        };
        if all && !self.started.swap(true, Ordering::SeqCst) {
            Arc::clone(self).play();
        }
    }

    /// Hands every page octopus's window as it stands, its frame and a picture of it; a show that cannot have the
    /// window ends, as it dives into it.
    pub(crate) async fn app(&self) {
        match self.snapshot().await {
            Ok(app) => self.to_pages(APP_EVENT, app),
            Err(error) => {
                eprintln!("octopus app: snapshot octopus's window for the show: {error}");
                self.end();
            }
        }
    }

    async fn snapshot(&self) -> Result<ShowApp, String> {
        let base = self.app.get_webview(BASE).ok_or("octopus's window is closed")?;
        let window = base.window();
        let scale = window.scale_factor().map_err(|error| error.to_string())?;
        let at = window.inner_position().map_err(|error| error.to_string())?.to_logical::<f64>(scale);
        let size = window.inner_size().map_err(|error| error.to_string())?.to_logical::<f64>(scale);
        let frame = ShowRect { x: at.x, y: at.y, width: size.width, height: size.height };
        let (sent, taken) = tokio::sync::oneshot::channel();
        platform::snapshot(&base, scale, move |picture| {
            let _ = sent.send(picture);
        })
        .map_err(|error| error.to_string())?;
        let picture = taken.await.ok().flatten().ok_or("octopus's window gave no picture")?;
        Ok(ShowApp { frame, picture })
    }

    /// Hands both of octopus's layers the tour's step.
    pub(crate) fn drive(&self, step: ShowDrive) {
        if step == ShowDrive::End {
            return self.end_tour();
        }
        self.to_layers(&step);
    }

    pub(crate) fn spot(&self, spot: ShowSpot) {
        self.to_pages(SPOT_EVENT, spot);
    }

    /// Hides or shows the cursor while the show plays. Commands run on the main thread, as does the end's showing of
    /// the cursor, so a change asked before the end lands before it.
    pub(crate) fn cursor(&self, shown: bool) {
        if self.playing() {
            platform::cursor(&self.windows(), shown);
        }
    }

    pub(crate) fn alert(&self) {
        if let Some(sound) = &*lock(&self.sound) {
            sound.alert();
        }
    }

    /// Stops the sound, ends the tour, shows the cursor, closes every show window and hands the keyboard back to the
    /// window beneath.
    pub(crate) fn end(&self) {
        if self.ended.swap(true, Ordering::SeqCst) {
            return;
        }
        lock(&self.sound).take();
        self.end_tour();
        self.layers_drawn(false);
        let app = self.app.clone();
        let windows = self.windows();
        let _ = self.app.run_on_main_thread(move || {
            platform::cursor(&windows, true);
            for window in windows {
                let _ = window.destroy();
            }
            if let Some(base) = app.get_webview_window(BASE) {
                let _ = base.set_focus();
            }
            octopus_shell::emit(&app, END_EVENT, ());
        });
    }

    /// Puts octopus's window back as the tour found it and forgets tidepool, once: when the tour ends, or the show
    /// does first.
    fn end_tour(&self) {
        if self.toured.swap(true, Ordering::SeqCst) {
            return;
        }
        self.to_layers(&ShowDrive::End);
        if let Err(error) = demo::forget(&self.app.state::<Arc<Hub>>().me, Computer::Blank) {
            eprintln!("octopus app: forget tidepool after the show: {error:#}");
        }
    }

    fn to_pages(&self, event: &str, payload: impl Serialize + Clone) {
        for page in self.scenes.keys() {
            if let Err(error) = self.app.emit_to(page.as_str(), event, payload.clone()) {
                eprintln!("octopus app: emit {event}: {error}");
            }
        }
    }

    fn to_layers(&self, step: &ShowDrive) {
        for label in [BASE, FLOAT] {
            if let Err(error) = self.app.emit_to(label, DRIVE_EVENT, step) {
                eprintln!("octopus app: emit {DRIVE_EVENT}: {error}");
            }
        }
    }

    fn windows(&self) -> Vec<WebviewWindow> {
        self.scenes.keys().filter_map(|page| self.app.get_webview_window(page)).collect()
    }

    /// Starts the song and every page on the same instant, `LEAD` from now, and ends the show when the song does.
    /// octopus's window comes to the front beneath the show, so it is what the show uncovers, and keeps drawing though
    /// the show covers it; the show takes the keys.
    fn play(self: Arc<Self>) {
        let start = Instant::now() + Duration::from_secs_f64(LEAD);
        let epoch_ms = (SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64() + LEAD) * 1000.0;
        *lock(&self.sound) = Some(sound::play(start, platform::alert_sound()));
        for window in self.windows() {
            if let Err(error) = window.set_ignore_cursor_events(false) {
                eprintln!("octopus app: let the show's window take clicks: {error}");
            }
        }
        self.layers_drawn(true);
        for label in [BASE, self.driver.as_str()] {
            if let Some(window) = self.app.get_webview_window(label) {
                let _ = window.set_focus();
            }
        }
        self.to_pages(PLAY_EVENT, epoch_ms);
        let watching = Arc::clone(&self);
        thread::spawn(move || watching.follow_pointer());
        thread::spawn(move || {
            thread::sleep(
                (start + sound::length() + Duration::from_secs_f64(TAIL)).saturating_duration_since(Instant::now()),
            );
            self.end();
        });
    }

    /// Sends every page the pointer whenever it moves, until the show ends; the pointer is read on the main thread.
    fn follow_pointer(&self) {
        let mut last = None;
        while self.playing() {
            let (sent, read) = mpsc::channel();
            let _ = self.app.run_on_main_thread(move || {
                let _ = sent.send(platform::pointer());
            });
            if let Ok(at) = read.recv()
                && last != Some(at)
            {
                last = Some(at);
                self.to_pages(POINTER_EVENT, at);
            }
            thread::sleep(POINTER_POLL);
        }
    }

    /// Whether octopus's layers keep drawing while the show covers them.
    fn layers_drawn(&self, always: bool) {
        for label in [BASE, FLOAT] {
            if let Some(webview) = self.app.get_webview(label)
                && let Err(error) = platform::keep_drawing(&webview, always)
            {
                eprintln!("octopus app: keep {label} drawing under the show: {error}");
            }
        }
    }
}

/// Every display's scene: the display, the rect round every display, its wallpaper, and whether its page drives.
fn scenes() -> Vec<ShowScene> {
    let displays = platform::displays();
    let Some(bounds) = displays.iter().map(|display| display.rect).reduce(|around, rect| {
        let (x, y) = (around.x.min(rect.x), around.y.min(rect.y));
        let right = (around.x + around.width).max(rect.x + rect.width);
        let bottom = (around.y + around.height).max(rect.y + rect.height);
        ShowRect { x, y, width: right - x, height: bottom - y }
    }) else {
        return Vec::new();
    };
    displays
        .into_iter()
        .enumerate()
        .map(|(display, found)| ShowScene {
            display: found.rect,
            bounds,
            wallpaper: found.wallpaper,
            drives: display == DRIVING,
        })
        .collect()
}

/// Opens the page `label` in a window over its scene's display, lifted above everything. Each window shows from the
/// start, as WebKit paints no webview in a hidden window, and lets clicks through until the show starts.
fn cover(app: &AppHandle, label: &str, scene: &ShowScene) -> anyhow::Result<()> {
    let ShowRect { x, y, width, height } = scene.display;
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App("show.html".into()))
        .title("octopus")
        .transparent(true)
        .decorations(false)
        .resizable(false)
        .shadow(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .focused(false)
        .position(x, y)
        .inner_size(width, height)
        .on_navigation(|url| url.scheme() == "tauri")
        .build()?;
    platform::cover(&window)?;
    window.set_ignore_cursor_events(true)?;
    Ok(())
}
