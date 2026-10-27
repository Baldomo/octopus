const DESIGN: &str = "../../DESIGN.html";

fn token(design: &str, name: &str) -> String {
    let declared = format!("--{name}:");
    let start = design.find(&declared).unwrap_or_else(|| panic!("DESIGN.html declares --{name}")) + declared.len();
    let value = &design[start..];
    value[..value.find(';').unwrap_or_else(|| panic!("--{name} ends with ;"))].trim().to_string()
}

fn main() {
    println!("cargo:rerun-if-changed={DESIGN}");
    let design = std::fs::read_to_string(DESIGN).expect("read DESIGN.html");
    // The window paints --bg before the UI loads, so launch never flashes another color.
    println!("cargo:rustc-env=OCTOPUS_BG={}", token(&design, "bg"));
    // The traffic lights center on the title bar, and the float layer hangs from its bottom edge.
    let chrome = token(&design, "chrome-h");
    println!("cargo:rustc-env=OCTOPUS_CHROME_H={}", chrome.strip_suffix("px").expect("--chrome-h is in px"));
    tauri_build::build();
}
