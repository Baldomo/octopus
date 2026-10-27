//! The state every layer of the window renders, held here so a component reads the same values in whichever layer
//! it sits. Its prefs persist across launches.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::Context;
use octopus_client::servers::{config, create_parent};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use ts_rs::TS;

use crate::layers::{Bounds, Viewport, float_bounds};
use crate::{emit, lock};

pub const UI_EVENT: &str = "ui";

const PREFS_FILE: &str = "app.json";
/// How many opened tasks stay mounted, so switching among them skips the mount.
const RECENT: usize = 8;
const PROBLEM_SHOWN: Duration = Duration::from_secs(8);

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
    pub modal: Option<Modal>,
    pub reveal: Option<Reveal>,
    pub problem: Option<String>,
    pub viewport: Viewport,
    /// Where the float layer covers the window: from its fixed corner past every surface of it that is out.
    pub float: Bounds,
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
    ShowModal {
        modal: Option<Modal>,
    },
    /// A surface of the float layer is out at `rect`; a modal one takes the keyboard.
    Float {
        surface: String,
        rect: Bounds,
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
    rect: Bounds,
    modal: bool,
}

/// What the float layer covers, and whether a modal among it holds the keyboard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cover {
    pub float: Bounds,
    pub modal: bool,
}

/// The shared `Ui`, and what the app keeps beside it to derive the float layer's cover.
struct State {
    ui: Ui,
    /// Each surface of the float layer that is out, by name.
    out: BTreeMap<String, Out>,
}

impl State {
    fn starting(prefs: Prefs) -> Self {
        let ui = Ui {
            version: 0,
            recent: prefs.open.iter().cloned().collect(),
            sidebar: prefs.pinned,
            prefs,
            modal: None,
            reveal: None,
            problem: None,
            viewport: Viewport::default(),
            float: float_bounds(std::iter::empty()),
        };
        Self { ui, out: BTreeMap::new() }
    }

    fn apply(&mut self, act: UiAct) {
        let Self { ui, out } = self;
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
            UiAct::ShowModal { modal } => ui.modal = modal,
            UiAct::Float { surface, rect, modal } => drop(out.insert(surface, Out { rect, modal })),
            UiAct::Sink { surface } => drop(out.remove(&surface)),
            UiAct::Reveal { task, path } => {
                ui.show_surface(Surface::Files);
                let nth = ui.reveal.as_ref().map_or(1, |reveal| reveal.nth + 1);
                ui.reveal = Some(Reveal { task, path, nth });
            }
            UiAct::Report { problem } => ui.problem = Some(problem),
        }
    }

    fn cover(&self) -> Cover {
        Cover {
            float: float_bounds(self.out.values().map(|out| out.rect)),
            modal: self.out.values().any(|out| out.modal),
        }
    }
}

impl Ui {
    fn show_surface(&mut self, surface: Surface) {
        self.prefs.surface = surface;
        self.prefs.view = View::Task;
    }
}

/// Places the float layer for its cover before and after a change, or names the problem it met.
type Follower = Box<dyn Fn(&Cover, &Cover) -> Result<(), String> + Send + Sync>;

pub struct UiStore {
    app: AppHandle,
    state: Mutex<State>,
    /// Counts problems shown, so a problem's timer clears only the problem it showed.
    problems: AtomicU64,
    /// Sees every change with the float layer's cover before and after it.
    follower: OnceLock<Follower>,
}

fn load_prefs() -> anyhow::Result<Prefs> {
    let path = config(PREFS_FILE)?;
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).with_context(|| format!("parse {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Prefs::default()),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

fn save_prefs(prefs: &Prefs) -> anyhow::Result<()> {
    let path = config(PREFS_FILE)?;
    create_parent(&path)?;
    std::fs::write(&path, serde_json::to_string_pretty(prefs)?).with_context(|| format!("write {}", path.display()))
}

impl UiStore {
    /// Prefs written by an earlier build or by hand start the window from the defaults, and the status bar says so.
    pub fn load(app: AppHandle, launch_problem: Option<String>) -> Arc<Self> {
        let (prefs, prefs_problem) = match load_prefs() {
            Ok(prefs) => (prefs, None),
            Err(error) => (Prefs::default(), Some(format!("{error:#}; started from the default prefs"))),
        };
        let problem = [launch_problem, prefs_problem].into_iter().flatten().reduce(|a, b| format!("{a}; {b}"));
        let store = Arc::new(Self {
            app,
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

    pub fn cover(&self) -> Cover {
        lock(&self.state).cover()
    }

    pub fn follow(&self, follower: impl Fn(&Cover, &Cover) -> Result<(), String> + Send + Sync + 'static) {
        assert!(self.follower.set(Box::new(follower)).is_ok(), "one follower watches the ui");
    }

    /// Fails only when the prefs it changed cannot be saved.
    pub fn act(self: &Arc<Self>, act: UiAct) -> Result<(), String> {
        let (before, after) = self.change(|state| state.apply(act));
        if before.prefs == after.prefs {
            return Ok(());
        }
        save_prefs(&after.prefs).map_err(|error| format!("{error:#}"))
    }

    pub fn resize(self: &Arc<Self>, viewport: Viewport) {
        self.change(|state| state.ui.viewport = viewport);
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
            let (before, covered) = (state.ui.clone(), state.cover());
            change(&mut state);
            let cover = state.cover();
            state.ui.float = cover.float;
            state.ui.version += 1;
            let placed = self.follower.get().map_or(Ok(()), |follower| follower(&covered, &cover));
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
        let sidebar = Bounds { x: 8.0, y: 46.0, width: 286.0, height: 820.0 };
        let popover = Bounds { x: 400.0, y: 100.0, width: 200.0, height: 100.0 };
        let mut state = State::starting(Prefs::default());
        let mut covered = |act| {
            state.apply(act);
            state.cover()
        };
        let float = |surface: &str, rect, modal| UiAct::Float { surface: surface.into(), rect, modal };
        let cover = |surfaces: &[Bounds], modal| Cover { float: float_bounds(surfaces.iter().copied()), modal };
        assert_eq!(covered(float("sidebar", sidebar, false)), cover(&[sidebar], false));
        assert_eq!(covered(float("modal", popover, true)), cover(&[sidebar, popover], true));
        assert_eq!(covered(UiAct::Sink { surface: "modal".into() }), cover(&[sidebar], false));
        assert_eq!(covered(UiAct::Sink { surface: "sidebar".into() }), cover(&[], false));
    }
}
