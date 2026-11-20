//! The binary this machine's server runs. On Linux it is the server's own copy in `~/.octopus/bin`, linked from
//! `~/.local/bin` so the CLI is on the PATH; on macOS it is the binary that installed it.

use std::path::Path;
#[cfg(target_os = "linux")]
use std::path::PathBuf;

#[cfg(target_os = "linux")]
use anyhow::Context;

/// The server's copy, while it is there and an `octopus` sits on the PATH at `~/.local/bin`: the link to the copy,
/// or a file of the user's own, which stays theirs.
#[cfg(target_os = "linux")]
pub fn installed(home: &Path) -> Option<PathBuf> {
    let copy = copy(home);
    (copy.is_file() && std::fs::symlink_metadata(link_path(home)).is_ok()).then_some(copy)
}

/// Puts `source` where this machine's server runs it from and links it onto the PATH, returning the copy. The copy
/// is replaced by a rename, so a server starting meanwhile runs the old binary or the new one, never a torn one.
#[cfg(target_os = "linux")]
pub fn put(home: &Path, source: &Path) -> anyhow::Result<PathBuf> {
    let copy = copy(home);
    if !same_file(source, &copy) {
        let directory = copy.parent().expect("the copy sits in a directory");
        std::fs::create_dir_all(directory).with_context(|| format!("create {}", directory.display()))?;
        let staged = directory.join(format!(".octopus.{}", std::process::id()));
        let placed = std::fs::copy(source, &staged)
            .with_context(|| format!("copy {} to {}", source.display(), staged.display()))
            .and_then(|_| std::fs::rename(&staged, &copy).with_context(|| format!("replace {}", copy.display())));
        if placed.is_err() {
            let _gone_already = std::fs::remove_file(&staged);
        }
        placed?;
    }
    let link = link_path(home);
    if std::fs::symlink_metadata(&link).is_err() {
        let directory = link.parent().expect("the link sits in a directory");
        std::fs::create_dir_all(directory).with_context(|| format!("create {}", directory.display()))?;
        std::os::unix::fs::symlink(&copy, &link)
            .with_context(|| format!("link {} to {}", link.display(), copy.display()))?;
    }
    Ok(copy)
}

#[cfg(target_os = "linux")]
fn copy(home: &Path) -> PathBuf {
    super::octopus_dir(home).join("bin").join("octopus")
}

#[cfg(target_os = "linux")]
fn link_path(home: &Path) -> PathBuf {
    home.join(".local").join("bin").join("octopus")
}

#[cfg(target_os = "linux")]
fn same_file(one: &Path, other: &Path) -> bool {
    matches!((one.canonicalize(), other.canonicalize()), (Ok(one), Ok(other)) if one == other)
}

/// A program on a mounted disk image, or in the read-only copy macOS translocates a downloaded app to, is gone once
/// the image is ejected or the app is moved.
#[cfg(target_os = "macos")]
pub fn transient(program: &Path) -> bool {
    program.starts_with("/Volumes") || program.components().any(|part| part.as_os_str() == "AppTranslocation")
}
