//! What the OS keeps for this phone: its device key, in the iOS Keychain or sealed under an Android Keystore key, and
//! the name it reports for the device. On Android the system bars also take the app's appearance from here.

use anyhow::Context;
use octopus_proto::device::DeviceKey;
use octopus_shell::microphone::Access;
use tauri::AppHandle;

/// The key made on the first launch, kept by the OS for this device alone. A launch that makes one, the first or one
/// after a backup restored the app's files onto another device, runs `before_making` first, so a launch that dies
/// between the two runs it again.
pub fn key(app: &AppHandle, before_making: impl FnOnce() -> anyhow::Result<()>) -> anyhow::Result<DeviceKey> {
    if let Some(pkcs8) = os::kept(app)? {
        return DeviceKey::from_pkcs8(&pkcs8).map_err(|error| anyhow::anyhow!("the kept device key: {error}"));
    }
    before_making()?;
    let key = DeviceKey::generate();
    os::keep(app, key.pkcs8()).context("keep the new device key")?;
    Ok(key)
}

pub fn name(app: &AppHandle) -> anyhow::Result<String> {
    os::name(app)
}

/// How far the status bar and the display's cutout reach over the page, in CSS pixels.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Insets {
    pub top: f64,
    pub left: f64,
    pub right: f64,
}

/// The page's insets where the webview does not report them as its safe area, as on Android.
pub fn insets(app: &AppHandle) -> anyhow::Result<Option<Insets>> {
    os::insets(app)
}

/// Whether the phone may record, asking the user the first time and waiting for the answer.
pub fn microphone(app: &AppHandle) -> Result<Access, String> {
    match os::microphone(app) {
        Ok(true) => Ok(Access::Granted),
        Ok(false) => Ok(Access::Denied),
        Err(error) => Err(format!("{error:#}")),
    }
}

#[cfg(target_os = "ios")]
mod os {
    use anyhow::{Context, anyhow};
    use objc2::MainThreadMarker;
    use objc2_ui_kit::UIDevice;
    use security_framework::access_control::{ProtectionMode, SecAccessControl};
    use security_framework::passwords::{PasswordOptions, generic_password, set_generic_password_options};
    use tauri::AppHandle;

    const SERVICE: &str = "com.zabealbe.octopus";
    const ACCOUNT: &str = "device-key";
    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    pub fn kept(_: &AppHandle) -> anyhow::Result<Option<Vec<u8>>> {
        match generic_password(PasswordOptions::new_generic_password(SERVICE, ACCOUNT)) {
            Ok(pkcs8) => Ok(Some(pkcs8)),
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(error) => Err(anyhow!("read the device key from the Keychain: {error}")),
        }
    }

    /// Readable once the phone has been unlocked since it started, and never restored onto another device.
    pub fn keep(_: &AppHandle, pkcs8: &[u8]) -> anyhow::Result<()> {
        let mut options = PasswordOptions::new_generic_password(SERVICE, ACCOUNT);
        let protection = Some(ProtectionMode::AccessibleAfterFirstUnlockThisDeviceOnly);
        options.set_access_control(SecAccessControl::create_with_protection(protection, 0)?);
        set_generic_password_options(pkcs8, options).context("write the device key to the Keychain")
    }

    pub fn name(_: &AppHandle) -> anyhow::Result<String> {
        let main = MainThreadMarker::new().context("UIKit reports the device's name on the main thread")?;
        Ok(UIDevice::currentDevice(main).name().to_string())
    }

    /// WebKit reports the safe area to CSS itself.
    pub fn insets(_: &AppHandle) -> anyhow::Result<Option<super::Insets>> {
        Ok(None)
    }

    /// Once granted, the audio session records as well as plays, which capturing needs, beside whatever else plays.
    #[allow(unsafe_code, reason = "AVFAudio's shared instances and their calls have no safe binding")]
    pub fn microphone(_: &AppHandle) -> anyhow::Result<bool> {
        use block2::RcBlock;
        use objc2::runtime::Bool;
        use objc2_avf_audio::{
            AVAudioApplication, AVAudioApplicationRecordPermission, AVAudioSession, AVAudioSessionCategoryOptions,
            AVAudioSessionCategoryPlayAndRecord,
        };

        // SAFETY: the shared application and session live for the app, and each call is made as AVFAudio documents.
        unsafe {
            let granted = match AVAudioApplication::sharedInstance().recordPermission() {
                AVAudioApplicationRecordPermission::Granted => true,
                AVAudioApplicationRecordPermission::Denied => false,
                _ => {
                    let (answer, answered) = std::sync::mpsc::channel();
                    let reply = RcBlock::new(move |granted: Bool| {
                        let _asker_gone = answer.send(granted.as_bool());
                    });
                    AVAudioApplication::requestRecordPermissionWithCompletionHandler(&reply);
                    answered.recv().context("the microphone prompt went unanswered")?
                }
            };
            if granted {
                let session = AVAudioSession::sharedInstance();
                let category =
                    AVAudioSessionCategoryPlayAndRecord.context("AVFAudio names no play-and-record category")?;
                session
                    .setCategory_withOptions_error(category, AVAudioSessionCategoryOptions::MixWithOthers)
                    .map_err(|error| anyhow!("let the audio session record: {error}"))?;
                session.setActive_error(true).map_err(|error| anyhow!("start the audio session: {error}"))?;
            }
            Ok(granted)
        }
    }
}

#[cfg(target_os = "android")]
mod os {
    use anyhow::{Context, anyhow};
    use base64::Engine;
    use std::sync::Arc;

    use base64::engine::general_purpose::STANDARD;
    use octopus_shell::client_menu::Appearance;
    use octopus_shell::ui::UiStore;
    use serde::de::IgnoredAny;
    use serde::{Deserialize, Serialize};
    use tauri::plugin::{PluginHandle, TauriPlugin};
    use tauri::{AppHandle, Manager, Wry};

    /// `DevicePlugin.kt`, which seals the key under a Keystore key that never leaves the device.
    struct Keeper(PluginHandle<Wry>);

    pub fn plugin() -> TauriPlugin<Wry> {
        tauri::plugin::Builder::new("device")
            .setup(|app, api| {
                app.manage(Keeper(api.register_android_plugin("com.zabealbe.octopus", "DevicePlugin")?));
                Ok(())
            })
            .build()
    }

    #[derive(Deserialize)]
    struct Kept {
        pkcs8: Option<String>,
    }

    #[derive(Serialize)]
    struct Keep<'a> {
        pkcs8: &'a str,
    }

    #[derive(Deserialize)]
    struct Named {
        name: String,
    }

    #[derive(Deserialize)]
    struct Answered {
        granted: bool,
    }

    pub fn microphone(app: &AppHandle) -> anyhow::Result<bool> {
        let answered: Answered = keeper(app).run_mobile_plugin("microphone", ()).context("ask for the microphone")?;
        Ok(answered.granted)
    }

    fn keeper(app: &AppHandle) -> &PluginHandle<Wry> {
        &app.state::<Keeper>().inner().0
    }

    pub fn kept(app: &AppHandle) -> anyhow::Result<Option<Vec<u8>>> {
        let kept: Kept = keeper(app).run_mobile_plugin("kept", ()).context("open the sealed device key")?;
        kept.pkcs8
            .map(|pkcs8| STANDARD.decode(pkcs8).map_err(|error| anyhow!("the sealed device key: {error}")))
            .transpose()
    }

    pub fn keep(app: &AppHandle, pkcs8: &[u8]) -> anyhow::Result<()> {
        let pkcs8 = STANDARD.encode(pkcs8);
        keeper(app).run_mobile_plugin::<IgnoredAny>("keep", Keep { pkcs8: &pkcs8 }).context("seal the device key")?;
        Ok(())
    }

    pub fn name(app: &AppHandle) -> anyhow::Result<String> {
        let named: Named = keeper(app).run_mobile_plugin("name", ()).context("read the device's name")?;
        Ok(named.name)
    }

    pub fn insets(app: &AppHandle) -> anyhow::Result<Option<super::Insets>> {
        Ok(Some(keeper(app).run_mobile_plugin("insets", ()).context("read the page's insets")?))
    }

    #[derive(Serialize)]
    struct Bars {
        appearance: Appearance,
    }

    /// Draws the system bars' icons for `appearance` off the calling thread, which may be the one Android draws on.
    fn show_bars(app: &AppHandle, appearance: Appearance) {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(error) = keeper(&app).run_mobile_plugin::<IgnoredAny>("bars", Bars { appearance }) {
                eprintln!("octopus app: draw the system bars for the {appearance:?} appearance: {error}");
            }
        });
    }

    /// The system bars' icons read against the page in whichever appearance the client menu picks.
    pub fn follow_appearance(app: &tauri::App) {
        let ui = Arc::clone(app.state::<Arc<UiStore>>().inner());
        let handle = app.handle().clone();
        show_bars(&handle, ui.native().appearance);
        ui.follow(move |before, after| {
            if before.appearance != after.appearance {
                show_bars(&handle, after.appearance);
            }
            Ok(())
        });
    }
}

#[cfg(target_os = "android")]
pub use os::{follow_appearance, plugin};
