//! The AppImage's entry in the desktop's app launcher, written at every launch so it opens the AppImage wherever the
//! user keeps it now.

use std::path::Path;

use anyhow::{Context, anyhow};

const ICON: &[u8] = include_bytes!("../icons/icon.png");

fn entry(appimage: &Path, icon: &Path) -> String {
    let mut quoted = String::new();
    for c in appimage.display().to_string().chars() {
        match c {
            '\\' => quoted.push_str(r"\\\\"),
            '"' | '`' | '$' => quoted.extend(['\\', c]),
            '%' => quoted.push_str("%%"),
            c => quoted.push(c),
        }
    }
    format!(
        "[Desktop Entry]\nType=Application\nName=octopus\nExec=\"{quoted}\"\nIcon={}\nTerminal=false\nCategories=Development;\nStartupWMClass=octopus\n",
        icon.display()
    )
}

/// Writes the entry under `~/.local/share` when it differs from this launch's, with its icon when the entry changed
/// or the icon is gone.
pub fn write(appimage: &Path) -> anyhow::Result<()> {
    let share = dirs::data_dir().ok_or_else(|| anyhow!("no data directory"))?;
    let icon = share.join("icons").join("octopus.png");
    let desktop = share.join("applications").join("octopus.desktop");
    let contents = entry(appimage, &icon);
    let changed = std::fs::read(&desktop).ok().as_deref() != Some(contents.as_bytes());
    if changed || !icon.exists() {
        put(&icon, ICON)?;
    }
    if changed {
        put(&desktop, contents.as_bytes())?;
    }
    Ok(())
}

fn put(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let directory = path.parent().expect("an entry sits in a directory");
    std::fs::create_dir_all(directory).with_context(|| format!("create {}", directory.display()))?;
    std::fs::write(path, contents).with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_entry_opens_the_appimage_wherever_it_sits() {
        let appimage = r#"/home/ada/My "$Apps" `100%` a\b/octopus.AppImage"#;
        let entry = super::entry(appimage.as_ref(), "/home/ada/icon.png".as_ref());
        let exec = r#"Exec="/home/ada/My \"\$Apps\" \`100%%\` a\\\\b/octopus.AppImage""#;
        assert!(entry.contains(&format!("\n{exec}\n")), "{entry}");
        assert!(entry.contains("\nIcon=/home/ada/icon.png\n"), "{entry}");
    }
}
