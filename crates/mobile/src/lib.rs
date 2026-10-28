//! The phone app, built only for iOS and Android.
#![cfg(mobile)]

mod app;
mod browser;
mod client_menu;
mod device;
#[cfg(target_os = "ios")]
mod webview;

pub use app::run;
