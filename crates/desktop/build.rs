const DESIGN: &str = "../../DESIGN.html";

fn token(design: &str, name: &str) -> String {
    let declared = format!("--{name}:");
    let start = design.find(&declared).unwrap_or_else(|| panic!("DESIGN.html declares --{name}")) + declared.len();
    let value = &design[start..];
    value[..value.find(';').unwrap_or_else(|| panic!("--{name} ends with ;"))].trim().to_string()
}

/// Whether `color` is `#` and 6 or 8 hex digits, the spelling the window's color parser takes.
fn is_hex(color: &str) -> bool {
    color
        .strip_prefix('#')
        .is_some_and(|digits| [6, 8].contains(&digits.len()) && digits.chars().all(|digit| digit.is_ascii_hexdigit()))
}

fn main() {
    println!("cargo:rerun-if-changed={DESIGN}");
    let design = std::fs::read_to_string(DESIGN).expect("read DESIGN.html");
    // The window paints --bg before the UI loads, so launch never flashes another color.
    let bg = token(&design, "bg");
    let (light, dark) = bg
        .strip_prefix("light-dark(")
        .and_then(|pair| pair.strip_suffix(')'))
        .and_then(|pair| pair.split_once(','))
        .map(|(light, dark)| (light.trim(), dark.trim()))
        .filter(|(light, dark)| is_hex(light) && is_hex(dark))
        .unwrap_or_else(|| panic!("--bg is light-dark(#rrggbb,#rrggbb), not {bg}"));
    println!("cargo:rustc-env=OCTOPUS_BG_LIGHT={light}");
    println!("cargo:rustc-env=OCTOPUS_BG_DARK={dark}");
    // The traffic lights center on the title bar, and the float layer hangs from its bottom edge.
    let chrome = token(&design, "chrome-h");
    println!("cargo:rustc-env=OCTOPUS_CHROME_H={}", chrome.strip_suffix("px").expect("--chrome-h is in px"));
    tauri_build::build();
}
