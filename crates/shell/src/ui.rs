//! The state every layer of the window renders, held here so a component reads the same values in whichever layer
//! it sits. Its prefs persist across launches.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::Context;
use octopus_client::servers::{Config, create_parent};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use ts_rs::TS;

use crate::client_menu::{Appearance, Zoom, ZoomStep};
use crate::{emit, lock};

pub const UI_EVENT: &str = "ui";
/// Whether the window shows anything, which the app tells the UI on every change.
pub const SHOWN_EVENT: &str = "window-shown";

const PREFS_FILE: &str = "app.json";
/// How many opened tasks stay mounted, so switching among them skips the mount.
const RECENT: usize = 8;
const PROBLEM_SHOWN: Duration = Duration::from_secs(8);

/// A rect of the window, in the UI's CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A size of the window, which the float layer lays its surfaces out in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, TS)]
pub struct Viewport {
    pub width: f64,
    pub height: f64,
}

/// A surface of the float layer that is out: it takes the pointer in `input` and draws in `drawn`, shadows included.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Floated {
    pub input: Bounds,
    pub drawn: Bounds,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum View {
    #[default]
    Task,
    Tasks,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    #[default]
    Files,
    Browser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Modal {
    Palette,
    NewTask,
    PairServer,
    Shortcuts,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Prefs {
    pub pinned: bool,
    pub collapsed: Vec<String>,
    pub surface: Surface,
    pub view: View,
    /// The open task's key.
    pub open: Option<String>,
    /// The section the Tasks board shows.
    pub board: Option<String>,
    pub appearance: Appearance,
    pub zoom: Zoom,
}

/// A file the palette opens in Files.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Reveal {
    /// The task's key.
    pub task: String,
    pub path: String,
    /// Counts reveals, so revealing the same file again opens it again.
    pub nth: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Ui {
    /// Raised by every change, so a layer drops a state that arrives after a newer one.
    pub version: u64,
    pub prefs: Prefs,
    /// The opened tasks' keys, newest first.
    pub recent: Vec<String>,
    pub sidebar: bool,
    pub client_menu: bool,
    pub modal: Option<Modal>,
    pub reveal: Option<Reveal>,
    pub problem: Option<String>,
    /// The window's size in the UI's CSS pixels, which the zoom scales.
    pub viewport: Viewport,
    /// Where the float layer takes the pointer over the layers beneath it.
    pub float_input: Vec<Bounds>,
}

/// Every change a layer makes to the shared state.
#[derive(Debug, Clone, PartialEq, Deserialize, TS)]
#[serde(tag = "do", rename_all = "snake_case", deny_unknown_fields)]
pub enum UiAct {
    OpenTask {
        key: String,
    },
    ShowView {
        view: View,
    },
    ShowBoard {
        section: String,
    },
    ShowSurface {
        surface: Surface,
    },
    TogglePin,
    ToggleCollapsed {
        section: String,
    },
    ShowSidebar {
        shown: bool,
    },
    ToggleSidebar,
    /// Showing the client menu closes a modal, and opening a modal closes the client menu.
    ShowClientMenu {
        shown: bool,
    },
    ToggleClientMenu,
    SetAppearance {
        appearance: Appearance,
    },
    Zoom {
        step: ZoomStep,
    },
    ShowModal {
        modal: Option<Modal>,
    },
    /// A surface of the float layer is out; a modal one takes the keyboard.
    Float {
        surface: String,
        at: Floated,
        modal: bool,
    },
    /// A surface of the float layer is gone.
    Sink {
        surface: String,
    },
    Reveal {
        task: String,
        path: String,
    },
    Report {
        problem: String,
    },
}

/// A surface of the float layer that is out.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Out {
    at: Floated,
    modal: bool,
}

/// What the app applies to the native window. `surfaces` are in the UI's CSS pixels at `zoom`.
#[derive(Debug, Clone, PartialEq)]
pub struct Native {
    pub surfaces: Vec<Floated>,
    pub modal: bool,
    pub zoom: Zoom,
    pub appearance: Appearance,
}

/// The shared `Ui`, and what the app keeps beside it to derive what the window shows natively.
struct State {
    ui: Ui,
    /// Each surface of the float layer that is out, by name.
    out: BTreeMap<String, Out>,
    /// The window's size in points.
    window: Viewport,
}

impl State {
    fn starting(prefs: Prefs) -> Self {
        let ui = Ui {
            version: 0,
            recent: prefs.open.iter().cloned().collect(),
            sidebar: prefs.pinned,
            client_menu: false,
            prefs,
            modal: None,
            reveal: None,
            problem: None,
            viewport: Viewport::default(),
            float_input: Vec::new(),
        };
        Self { ui, out: BTreeMap::new(), window: Viewport::default() }
    }

    fn apply(&mut self, act: UiAct) {
        let Self { ui, out, .. } = self;
        match act {
            UiAct::OpenTask { key } => {
                ui.recent.retain(|recent| *recent != key);
                ui.recent.insert(0, key.clone());
                ui.recent.truncate(RECENT);
                ui.prefs.open = Some(key);
                ui.prefs.view = View::Task;
            }
            UiAct::ShowView { view } => ui.prefs.view = view,
            UiAct::ShowBoard { section } => {
                ui.prefs.board = Some(section);
                ui.prefs.view = View::Tasks;
            }
            UiAct::ShowSurface { surface } => ui.show_surface(surface),
            UiAct::TogglePin => ui.prefs.pinned = !ui.prefs.pinned,
            UiAct::ToggleCollapsed { section } => match ui.prefs.collapsed.iter().position(|name| *name == section) {
                Some(at) => drop(ui.prefs.collapsed.remove(at)),
                None => ui.prefs.collapsed.push(section),
            },
            UiAct::ShowSidebar { shown } => ui.sidebar = shown,
            UiAct::ToggleSidebar => ui.sidebar = !ui.sidebar,
            UiAct::ShowClientMenu { shown } => ui.show_client_menu(shown),
            UiAct::ToggleClientMenu => ui.show_client_menu(!ui.client_menu),
            UiAct::SetAppearance { appearance } => ui.prefs.appearance = appearance,
            UiAct::Zoom { step } => ui.prefs.zoom = ui.prefs.zoom.step(step),
            UiAct::ShowModal { modal } => ui.show_modal(modal),
            UiAct::Float { surface, at, modal } => drop(out.insert(surface, Out { at, modal })),
            UiAct::Sink { surface } => drop(out.remove(&surface)),
            UiAct::Reveal { task, path } => {
                ui.show_surface(Surface::Files);
                let nth = ui.reveal.as_ref().map_or(1, |reveal| reveal.nth + 1);
                ui.reveal = Some(Reveal { task, path, nth });
            }
            UiAct::Report { problem } => ui.problem = Some(problem),
        }
    }

    fn native(&self) -> Native {
        Native {
            surfaces: self.out.values().map(|out| out.at).collect(),
            modal: self.out.values().any(|out| out.modal),
            zoom: self.ui.prefs.zoom,
            appearance: self.ui.prefs.appearance,
        }
    }

    fn fit_viewport(&mut self) {
        let scale = self.ui.prefs.zoom.scale();
        self.ui.viewport = Viewport { width: self.window.width / scale, height: self.window.height / scale };
    }
}

impl Ui {
    fn show_client_menu(&mut self, shown: bool) {
        self.client_menu = shown;
        if shown {
            self.modal = None;
        }
    }

    fn show_modal(&mut self, modal: Option<Modal>) {
        self.client_menu &= modal.is_none();
        self.modal = modal;
    }

    fn show_surface(&mut self, surface: Surface) {
        self.prefs.surface = surface;
        self.prefs.view = View::Task;
    }
}

/// Applies what the window shows natively before and after a change, or names the problem it met.
type Follower = Box<dyn Fn(&Native, &Native) -> Result<(), String> + Send + Sync>;

pub struct UiStore {
    app: AppHandle,
    config: Config,
    state: Mutex<State>,
    /// Counts problems shown, so a problem's timer clears only the problem it showed.
    problems: AtomicU64,
    /// Sees every change with what the window shows natively before and after it.
    follower: OnceLock<Follower>,
}

fn load_prefs(config: &Config) -> anyhow::Result<Prefs> {
    let path = config.file(PREFS_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_prefs(&text).with_context(|| format!("parse {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Prefs::default()),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

/// A pref the file lacks, as one written before that pref existed does, takes its default.
fn parse_prefs(text: &str) -> anyhow::Result<Prefs> {
    let serde_json::Value::Object(saved) = serde_json::from_str(text)? else {
        anyhow::bail!("prefs are not a JSON object");
    };
    let serde_json::Value::Object(mut prefs) = serde_json::to_value(Prefs::default())? else {
        unreachable!("Prefs serializes as an object");
    };
    prefs.extend(saved);
    Ok(serde_json::from_value(serde_json::Value::Object(prefs))?)
}

fn save_prefs(config: &Config, prefs: &Prefs) -> anyhow::Result<()> {
    let path = config.file(PREFS_FILE);
    create_parent(&path)?;
    std::fs::write(&path, serde_json::to_string_pretty(prefs)?).with_context(|| format!("write {}", path.display()))
}

impl UiStore {
    /// Prefs the app cannot parse start the window from the defaults, and the status bar says so.
    pub fn load(app: AppHandle, config: Config, launch_problem: Option<String>) -> Arc<Self> {
        let (prefs, prefs_problem) = match load_prefs(&config) {
            Ok(prefs) => (prefs, None),
            Err(error) => (Prefs::default(), Some(format!("{error:#}; started from the default prefs"))),
        };
        let problem = [launch_problem, prefs_problem].into_iter().flatten().reduce(|a, b| format!("{a}; {b}"));
        let store = Arc::new(Self {
            app,
            config,
            state: Mutex::new(State::starting(prefs)),
            problems: AtomicU64::new(0),
            follower: OnceLock::new(),
        });
        if let Some(problem) = problem {
            store.change(|state| state.ui.problem = Some(problem));
        }
        store
    }

    pub fn ui(&self) -> Ui {
        lock(&self.state).ui.clone()
    }

    pub fn native(&self) -> Native {
        lock(&self.state).native()
    }

    pub fn follow(&self, follower: impl Fn(&Native, &Native) -> Result<(), String> + Send + Sync + 'static) {
        assert!(self.follower.set(Box::new(follower)).is_ok(), "one follower watches the ui");
    }

    /// Fails only when the prefs it changed cannot be saved.
    pub fn act(self: &Arc<Self>, act: UiAct) -> Result<(), String> {
        let (before, after) = self.change(|state| state.apply(act));
        if before.prefs == after.prefs {
            return Ok(());
        }
        save_prefs(&self.config, &after.prefs).map_err(|error| format!("{error:#}"))
    }

    /// Takes the window's size in points.
    pub fn resize(self: &Arc<Self>, window: Viewport) {
        self.change(|state| state.window = window);
    }

    pub fn report(self: &Arc<Self>, problem: String) {
        self.act(UiAct::Report { problem }).expect("a report changes no prefs");
    }

    /// Applies `change` and returns the `Ui` before and after it. The follower places the float layer before any
    /// layer hears of the change, both under the state's lock, so the float layer and every layer's state move
    /// through the changes in the order they were made. A new problem shows until a newer one replaces it or its
    /// time is up.
    fn change(self: &Arc<Self>, change: impl FnOnce(&mut State)) -> (Ui, Ui) {
        let (before, after, placed) = {
            let mut state = lock(&self.state);
            let (before, was) = (state.ui.clone(), state.native());
            change(&mut state);
            state.fit_viewport();
            let native = state.native();
            state.ui.float_input = native.surfaces.iter().map(|surface| surface.input).collect();
            state.ui.version += 1;
            let placed = self.follower.get().map_or(Ok(()), |follower| follower(&was, &native));
            let after = state.ui.clone();
            emit(&self.app, UI_EVENT, after.clone());
            (before, after, placed)
        };
        if after.problem.is_some() && after.problem != before.problem {
            self.clear_problem_later();
        }
        if let Err(problem) = placed {
            self.report(problem);
        }
        (before, after)
    }

    fn clear_problem_later(self: &Arc<Self>) {
        let (store, shown) = (Arc::clone(self), self.problems.fetch_add(1, Ordering::Relaxed) + 1);
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(PROBLEM_SHOWN).await;
            store.change(|state| {
                if store.problems.load(Ordering::Relaxed) == shown {
                    state.ui.problem = None;
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_saved_before_a_pref_existed_keep_theirs_and_take_its_default() {
        let saved = r#"{"pinned":true,"collapsed":["octopus"],"surface":"files","view":"task","open":"local/tsk_1","board":null}"#;
        let prefs = parse_prefs(saved).expect("parses");
        assert_eq!((prefs.pinned, prefs.open.as_deref()), (true, Some("local/tsk_1")));
        assert_eq!((prefs.appearance, prefs.zoom), (Appearance::System, Zoom::default()));
        assert!(parse_prefs(r#"{"pinned":true,"retired":1}"#).is_err());
    }

    #[test]
    fn recent_tasks_keep_the_newest_eight_once_each() {
        let mut state = State::starting(Prefs::default());
        for key in ["a", "b", "c", "d", "e", "f", "g", "h", "i", "b"] {
            state.apply(UiAct::OpenTask { key: key.into() });
        }
        assert_eq!(state.ui.recent, ["b", "i", "h", "g", "f", "e", "d", "c"]);
        assert_eq!(state.ui.prefs.open.as_deref(), Some("b"));
    }

    #[test]
    fn the_float_layer_covers_every_surface_that_is_out_and_a_modal_one_holds_the_keyboard() {
        let rect = |x, y, width, height| Bounds { x, y, width, height };
        let sidebar = Floated { input: rect(0.0, 38.0, 294.0, 828.0), drawn: rect(0.0, 22.0, 342.0, 866.0) };
        let popover = Floated { input: rect(400.0, 100.0, 200.0, 100.0), drawn: rect(380.0, 90.0, 240.0, 140.0) };
        let mut state = State::starting(Prefs::default());
        let mut covered = |act| {
            state.apply(act);
            let native = state.native();
            (native.surfaces, native.modal)
        };
        let float = |surface: &str, at, modal| UiAct::Float { surface: surface.into(), at, modal };
        assert_eq!(covered(float("sidebar", sidebar, false)), (vec![sidebar], false));
        assert_eq!(covered(float("modal", popover, true)), (vec![popover, sidebar], true));
        assert_eq!(covered(UiAct::Sink { surface: "modal".into() }), (vec![sidebar], false));
        assert_eq!(covered(UiAct::Sink { surface: "sidebar".into() }), (vec![], false));
    }
}
