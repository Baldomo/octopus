# octopus

A task manager for coding agents, written in Rust. It runs on macOS and Linux, with a phone app for iOS and Android.

## Where things are decided

- `spec/` describes how octopus behaves. `spec/README.md` lists each spec file and the code it governs. The spec changes only when @zabealbe asks, and a new rule from him goes into the file it belongs to.
- If your code disagrees with the spec, say so in the pull request and tag @zabealbe.
- `DESIGN.html` defines how the desktop app looks. Take colors, fonts, sizes and timing from its `#tokens` style block, and components from its `#components` block and its examples.
- `DESIGN-MOBILE.html` does the same for the phone app, on top of `DESIGN.html`'s tokens. Its fonts only include the characters already on the page, so new text needs the font subsets regenerated.

## Commands

| what | command |
|------|---------|
| build the UI (do this before any cargo command) | `npm --prefix ui ci && npm --prefix ui run build` |
| typecheck the UI and check its formatting | `npm --prefix ui run typecheck && npm --prefix ui run format:check` |
| regenerate the UI's types after changing a type it uses | `cargo run -p octopus-shell --example types && cargo run -p octopus-desktop --example types` |
| regenerate the skill after changing an op or a command | `cargo run -p octopus-cli --example skill` |
| regenerate the app icons after changing `logo/` (this strips the iOS icons' alpha channel, which App Store validation rejects, and cuts the desktop icon to the macOS rounded square, since the Dock draws it unmasked) | `(cd crates/mobile && cargo tauri icon ../../logo/icon.json) && git clean -fdq crates/mobile/icons && swift logo/flatten.swift crates/mobile/gen/apple/Assets.xcassets/AppIcon.appiconset/*.png && swift logo/macos.swift logo/octopus.png crates/desktop/icons/icon.png` |
| typecheck | `cargo check --workspace` |
| check formatting | `cargo fmt --all --check` |
| lint | `cargo clippy --workspace --all-targets -- -D warnings` |
| run all tests | `cargo nextest run --workspace` |
| install the phone app's tooling (once) | `cargo install tauri-cli --version '^2' --locked && rustup target add aarch64-apple-ios-sim aarch64-linux-android` |
| run the phone app on the Android emulator (after building the UI, with `JAVA_HOME`, `ANDROID_HOME` and `NDK_HOME` set) | `(cd crates/mobile && cargo tauri android build --debug --target aarch64 --apk) && adb install -r crates/mobile/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk && adb shell am start -n com.noi.octopus/.MainActivity` |
| run the phone app on the iOS Simulator (after building the UI) | `(cd crates/mobile && cargo tauri ios build --debug --target aarch64-sim) && xcrun simctl install booted ~/Library/Developer/Xcode/DerivedData/octopus-mobile-*/Build/Products/debug-iphonesimulator/octopus.app && xcrun simctl launch booted com.noi.octopus` |
| lint the phone app | `cargo clippy -p octopus-mobile --target aarch64-apple-ios-sim -- -D warnings` |
