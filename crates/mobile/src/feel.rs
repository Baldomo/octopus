//! The haptics the page asks for by what happened, played as each OS's own: UIKit's feedback generators on iOS and
//! the webview's `performHapticFeedback` on Android, which follows the system's touch feedback setting.

use serde::{Deserialize, Serialize};

pub use os::play;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Feel {
    /// A choice among several changed.
    Select,
    /// A key of the key bar sent.
    Tap,
    /// A held finger lifted a task or opened its menu.
    Lift,
    /// A lifted task landed on another state.
    Drop,
    /// A pull crossed the point where letting go acts.
    Threshold,
    Success,
    Warning,
    Error,
}

#[cfg(target_os = "ios")]
mod os {
    #![allow(unsafe_code, reason = "a WKWebView reaches UIKit only through its raw pointer")]

    use std::cell::OnceCell;

    use anyhow::Context;
    use objc2::rc::Retained;
    use objc2_ui_kit::{
        UIImpactFeedbackGenerator, UIImpactFeedbackStyle, UINotificationFeedbackGenerator, UINotificationFeedbackType,
        UISelectionFeedbackGenerator, UIView,
    };
    use tauri::{AppHandle, Manager};

    use super::Feel;

    /// Attached to the webview and kept between feels, each prepared again after it plays, so the Taptic Engine
    /// answers the next one without a delay.
    struct Generators {
        select: Retained<UISelectionFeedbackGenerator>,
        light: Retained<UIImpactFeedbackGenerator>,
        medium: Retained<UIImpactFeedbackGenerator>,
        rigid: Retained<UIImpactFeedbackGenerator>,
        notify: Retained<UINotificationFeedbackGenerator>,
    }

    impl Generators {
        fn on(view: &UIView) -> Self {
            let impact = |style| UIImpactFeedbackGenerator::feedbackGeneratorWithStyle_forView(style, view);
            Self {
                select: UISelectionFeedbackGenerator::feedbackGeneratorForView(view),
                light: impact(UIImpactFeedbackStyle::Light),
                medium: impact(UIImpactFeedbackStyle::Medium),
                rigid: impact(UIImpactFeedbackStyle::Rigid),
                notify: UINotificationFeedbackGenerator::feedbackGeneratorForView(view),
            }
        }

        fn play(&self, feel: Feel) {
            let impact = |generator: &UIImpactFeedbackGenerator| {
                generator.impactOccurred();
                generator.prepare();
            };
            let notify = |kind| {
                self.notify.notificationOccurred(kind);
                self.notify.prepare();
            };
            match feel {
                Feel::Select => {
                    self.select.selectionChanged();
                    self.select.prepare();
                }
                Feel::Tap | Feel::Threshold => impact(&self.light),
                Feel::Lift => impact(&self.medium),
                Feel::Drop => impact(&self.rigid),
                Feel::Success => notify(UINotificationFeedbackType::Success),
                Feel::Warning => notify(UINotificationFeedbackType::Warning),
                Feel::Error => notify(UINotificationFeedbackType::Error),
            }
        }
    }

    thread_local! {
        static GENERATORS: OnceCell<Generators> = const { OnceCell::new() };
    }

    pub fn play(app: &AppHandle, feel: Feel) -> anyhow::Result<()> {
        let window = app.get_webview_window("main").context("no main window")?;
        window
            .with_webview(move |webview| {
                // SAFETY: on iOS `inner` is the window's live WKWebView, a UIView, for as long as this closure runs.
                let view = unsafe { &*webview.inner().cast::<UIView>() };
                GENERATORS.with(|generators| generators.get_or_init(|| Generators::on(view)).play(feel));
            })
            .context("feel on the webview")
    }
}

#[cfg(target_os = "android")]
mod os {
    use anyhow::Context;
    use serde::Serialize;
    use serde::de::IgnoredAny;
    use tauri::AppHandle;

    use super::Feel;

    #[derive(Serialize)]
    struct Felt {
        feel: Feel,
    }

    pub fn play(app: &AppHandle, feel: Feel) -> anyhow::Result<()> {
        crate::device::run(app, "feel", Felt { feel }).map(|_: IgnoredAny| ()).context("feel")
    }
}
