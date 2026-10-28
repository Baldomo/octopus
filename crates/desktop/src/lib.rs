mod browser;
mod commands;
mod layers;
pub mod types;

use std::sync::Arc;

use anyhow::Context;
use octopus_client::Identity;
use octopus_client::servers::{Config, Servers};
use octopus_shell::Client;
use octopus_shell::microphone::Access;
use octopus_shell::ui::UiStore;
use tauri::{Manager, Runtime};

use crate::browser::Browsers;
use crate::layers::Layers;

/// The app's config, assets and capabilities, on any runtime.
fn context<R: Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}

/// Opens the window and blocks until it closes.
pub fn run(launch_problem: Option<String>) -> anyhow::Result<()> {
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
        .invoke_handler(tauri::generate_handler![
            commands::title_bar_double_click,
            commands::browser_open,
            commands::browser_load,
            commands::browser_show_page,
            commands::browser_go,
            commands::browser_close,
        ])
        .setup(move |app| {
            octopus_shell::start(app.handle(), client, launch_problem);
            let ui = Arc::clone(app.state::<Arc<UiStore>>().inner());
            app.manage(Browsers::new(Layers::open(app, ui)?));
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
    }
}
