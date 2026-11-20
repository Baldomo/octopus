mod browser;
mod commands;
#[cfg(target_os = "linux")]
mod launcher;
mod layers;
mod menu;
#[cfg(target_os = "macos")]
mod relocate;
mod setup;
pub mod types;
mod update;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use octopus_client::Identity;
use octopus_client::servers::{Config, Servers};
use octopus_shell::Client;
use octopus_shell::browsing::BrowserPages;
use octopus_shell::microphone::Access;
use octopus_shell::ui::UiStore;
use tauri::{Manager, Runtime};

use crate::browser::Browsers;
use crate::layers::Layers;
pub use crate::setup::Install;
use crate::setup::Setup;

/// The app's config, assets and capabilities, on any runtime.
fn context<R: Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}

macro_rules! app_commands {
    ($($command:ident),* $(,)?) => {
        /// The app's own command names, which the UI invokes.
        const COMMANDS: &[&str] = &[$(stringify!($command)),*];

        fn invoke_handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
            tauri::generate_handler![$(commands::$command),*]
        }
    };
}

app_commands![
    title_bar_double_click,
    browser_load,
    browser_show_page,
    browser_go,
    browser_find,
    browser_forget,
    app_menu,
    file_outside,
    haptic_alignment,
    setup,
    setup_retry,
    skill_install,
];

/// Writes the octopus skill for the user's Claude Code, returning where.
pub struct Skill(pub fn() -> anyhow::Result<PathBuf>);

/// What a launch hands the window.
pub struct Launch {
    /// Why the launch could not look at this machine's server.
    pub problem: Option<String>,
    pub install: Option<Install>,
    pub skill: Skill,
    pub app: App,
}

/// What a launch runs as.
pub enum App {
    /// A build from source, which stays the build it is.
    Source,
    /// The Mac app's bundle on a disk image, in a translocated copy or in the Trash, which the window offers to move
    /// to Applications.
    #[cfg(target_os = "macos")]
    Transient(PathBuf),
    /// The Mac app's bundle or the AppImage, which the window keeps updated and, on Linux, points the app launcher at.
    Installed(PathBuf),
}

/// Opens the window, first offering to move a transient app to Applications, installs this machine's server
/// meanwhile when given one to install, updates an installed app in the background, and blocks until the window
/// closes. While the user keeps the skill, every
/// launch writes it again, so it follows the installed octopus; the first open asks before writing it.
pub fn run(launch: Launch) -> anyhow::Result<()> {
    let Launch { problem, install, skill, app: runs_as } = launch;
    let config = Config::user()?;
    let client = Client {
        me: Identity::load(&config)?,
        servers: Servers::load(&config)?,
        config,
        // macOS asks for the microphone itself, the first time a capture opens it.
        microphone_access: Box::new(|| Ok(Access::Granted)),
    };
    tauri::Builder::default()
        .plugin(octopus_shell::plugin())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(invoke_handler())
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            let moved = match &runs_as {
                App::Transient(bundle) => {
                    relocate::offer(objc2::MainThreadMarker::new().expect("tauri sets up on the main thread"), bundle)
                }
                _ => Ok(()),
            };
            octopus_shell::start(app.handle(), client, problem);
            menu::relay(app.handle());
            let ui = Arc::clone(app.state::<Arc<UiStore>>().inner());
            #[cfg(target_os = "macos")]
            if let Err(error) = moved {
                ui.report(format!("move octopus to Applications: {error:#}"));
            }
            if let App::Installed(file) = runs_as {
                #[cfg(target_os = "linux")]
                if let Err(error) = launcher::write(&file) {
                    ui.report(format!("add octopus to the app launcher: {error:#}"));
                }
                update::start(app.handle(), Arc::clone(&ui), file);
            }
            app.manage(BrowserPages::new(app.handle().clone()));
            if install.is_none()
                && ui.ui().prefs.skill
                && let Err(error) = (skill.0)()
            {
                ui.report(format!("write the octopus skill: {error:#}"));
            }
            app.manage(skill);
            app.manage(Browsers::new(app.handle().clone(), Layers::open(app, ui)?));
            app.manage(Setup::start(app.handle().clone(), install));
            Ok(())
        })
        .run(context())
        .context("run the app")
}

#[cfg(test)]
mod tests {
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder};
    use tauri::webview::InvokeRequest;
    use tauri::{App, WebviewWindow, WebviewWindowBuilder};

    fn app() -> App<MockRuntime> {
        mock_builder()
            .plugin(octopus_shell::commands().build())
            .plugin(tauri_plugin_opener::init())
            .build(super::context())
            .expect("the app builds on the mock runtime")
    }

    fn invoke(webview: &WebviewWindow<MockRuntime>, cmd: &str, body: serde_json::Value) -> Result<(), String> {
        let request = InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().expect("a url"),
            body: InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.into(),
        };
        get_ipc_response(webview, request).map(drop).map_err(|refusal| refusal.to_string())
    }

    #[test]
    fn the_ui_layers_reach_the_core_and_nothing_their_capabilities_leave_out() {
        let app = app();
        let layer = |label: &str| WebviewWindowBuilder::new(&app, label, Default::default()).build().expect(label);
        let name = serde_json::json!({ "name": "octopus" });
        let (base, float) = (layer("main"), layer("float"));
        for webview in [&base, &float] {
            assert_eq!(invoke(webview, "plugin:octopus|auto_color", name.clone()), Ok(()), "{}", webview.label());
        }
        let refused = invoke(&layer("browser-0"), "plugin:octopus|auto_color", name);
        assert!(refused.is_err_and(|refusal| refusal.contains("not allowed")), "a browser page reaches no command");
        let closed = invoke(&base, "plugin:window|close", serde_json::json!({}));
        assert!(closed.is_err_and(|refusal| refusal.contains("not allowed")), "the window's own commands stay shut");
        let file = invoke(&base, "plugin:opener|open_url", serde_json::json!({ "url": "file:///etc/hosts" }));
        assert!(file.is_err_and(|refusal| refusal.contains("Not allowed")), "the system browser opens only web links");
    }
}
