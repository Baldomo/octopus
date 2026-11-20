//! On macOS, builds the Octopus Microphone HAL plug-in as a signed bundle in OUT_DIR, which `octopus install`
//! embeds and copies into /Library/Audio/Plug-Ins/HAL, and gives the `octopus` binary the Info.plist macOS reads
//! the app's identity and microphone prompt from.

use std::path::Path;
use std::process::Command;

const SOURCE: &str = "microphone/OctopusMicrophone.c";
const INFO: &str = "microphone/Info.plist";
const APP_INFO: &str = "octopus.plist";

fn main() {
    println!("cargo::rerun-if-changed={SOURCE}");
    println!("cargo::rerun-if-changed={INFO}");
    println!("cargo::rerun-if-changed={APP_INFO}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        bundle(Path::new(&std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")));
        let plist = std::path::absolute(APP_INFO).expect("resolve the app's Info.plist");
        println!("cargo::rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{}", plist.display());
    }
}

fn run(command: &mut Command) {
    let status = command.status().unwrap_or_else(|error| panic!("run {command:?}: {error}"));
    assert!(status.success(), "{command:?} exited with {status}");
}

fn bundle(out: &Path) {
    let bundle = out.join("OctopusMicrophone.driver");
    let contents = bundle.join("Contents");
    let macos = contents.join("MacOS");
    if bundle.exists() {
        std::fs::remove_dir_all(&bundle).expect("clear the previous bundle");
    }
    std::fs::create_dir_all(&macos).expect("create the bundle");
    std::fs::copy(INFO, contents.join("Info.plist")).expect("copy Info.plist");
    run(Command::new("xcrun")
        .args([
            "clang",
            "-bundle",
            "-O2",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-mmacosx-version-min=14.0",
            "-arch",
            "arm64",
            "-arch",
            "x86_64",
            "-framework",
            "CoreAudio",
            "-framework",
            "CoreFoundation",
            "-o",
        ])
        .arg(macos.join("OctopusMicrophone"))
        .arg(SOURCE));
    run(Command::new("codesign").args(["--force", "--sign", "-"]).arg(&bundle));
}
