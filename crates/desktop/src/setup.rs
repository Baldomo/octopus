//! Installing this machine's server while the window shows, which the first open waits on and retries.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::AppHandle;
use ts_rs::TS;

use octopus_shell::lock;

pub(crate) const SETUP_EVENT: &str = "setup";

/// This machine's server to install: its name, its log, whether no service ran it before this launch, and the
/// install, which runs again on every retry.
pub struct Install {
    pub server: String,
    pub first: bool,
    pub log: PathBuf,
    pub run: Box<dyn Fn() -> anyhow::Result<()> + Send + Sync>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum Progress {
    Installing,
    Failed { error: String },
    Installed,
}

/// The install this launch runs, as the window shows it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub(crate) struct SetupView {
    pub server: String,
    pub log: String,
    pub progress: Progress,
}

/// A launch that installs nothing holds no install.
pub(crate) struct Setup {
    app: AppHandle,
    install: Option<Arc<Install>>,
    view: Arc<Mutex<Option<SetupView>>>,
}

impl Setup {
    pub fn start(app: AppHandle, install: Option<Install>) -> Self {
        let view = install.as_ref().map(|install| SetupView {
            server: install.server.clone(),
            log: install.log.display().to_string(),
            progress: Progress::Installing,
        });
        let setup = Self { app, install: install.map(Arc::new), view: Arc::new(Mutex::new(view)) };
        setup.run();
        setup
    }

    pub fn view(&self) -> Option<SetupView> {
        lock(&self.view).clone()
    }

    /// Installs again after a failure; an install under way or done stays as it is.
    pub fn retry(&self) {
        let failed = lock(&self.view).as_ref().is_some_and(|view| matches!(view.progress, Progress::Failed { .. }));
        if failed {
            self.run();
        }
    }

    fn run(&self) {
        let Some(install) = self.install.clone() else { return };
        let (app, view) = (self.app.clone(), Arc::clone(&self.view));
        let report = move |progress: Progress| {
            let shown = lock(&view).as_mut().map(|shown| {
                shown.progress = progress;
                shown.clone()
            });
            octopus_shell::emit(&app, SETUP_EVENT, shown);
        };
        report(Progress::Installing);
        std::thread::spawn(move || {
            report(match (install.run)() {
                Ok(()) => Progress::Installed,
                Err(error) => Progress::Failed { error: format!("{error:#}") },
            })
        });
    }
}
