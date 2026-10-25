const DESIGN: &str = "../../DESIGN.html";

/// The window paints DESIGN.html's `--bg` before the UI loads, so launch never flashes another color.
fn background() -> String {
    let design = std::fs::read_to_string(DESIGN).expect("read DESIGN.html");
    let start = design.find("--bg:").expect("DESIGN.html declares --bg") + "--bg:".len();
    let value = &design[start..];
    value[..value.find(';').expect("--bg ends with ;")].trim().to_string()
}

fn main() {
    println!("cargo:rerun-if-changed={DESIGN}");
    println!("cargo:rustc-env=OCTOPUS_BG={}", background());
    tauri_build::build();
}
