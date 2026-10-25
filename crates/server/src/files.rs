use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};

use octopus_proto::Error;
use octopus_proto::ops::{Entry, EntryKind, Match};

const MAX_MATCHES: usize = 1000;

fn io(path: &Path, error: std::io::Error) -> Error {
    match error.kind() {
        std::io::ErrorKind::NotFound => Error::not_found(path.display()),
        _ => Error::internal(format!("{}: {error}", path.display())),
    }
}

/// A checkout-relative path as the wire spells it, `/`-separated on every platform.
pub fn wire_path(relative: &Path) -> String {
    let components: Vec<_> = relative.components().map(|component| component.as_os_str().to_string_lossy()).collect();
    components.join("/")
}

pub fn path_inside_checkout(checkout: &Path, path: &str) -> Result<PathBuf, Error> {
    let relative = Path::new(path);
    if relative.components().any(|component| !matches!(component, Component::Normal(_) | Component::CurDir)) {
        return Err(Error::invalid(format!("{path} is not a path inside the checkout")));
    }
    Ok(checkout.join(relative))
}

pub fn list(checkout: &Path, path: &str) -> Result<Vec<Entry>, Error> {
    let directory = path_inside_checkout(checkout, path)?;
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&directory).map_err(|error| io(&directory, error))? {
        let entry = entry.map_err(|error| io(&directory, error))?;
        let kind =
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) { EntryKind::Directory } else { EntryKind::File };
        entries.push(Entry { name: entry.file_name().to_string_lossy().to_string(), kind });
    }
    entries.sort_by(|a, b| (a.kind != EntryKind::Directory, &a.name).cmp(&(b.kind != EntryKind::Directory, &b.name)));
    Ok(entries)
}

pub fn read(checkout: &Path, path: &str) -> Result<Vec<u8>, Error> {
    let file = path_inside_checkout(checkout, path)?;
    std::fs::read(&file).map_err(|error| io(&file, error))
}

pub fn write(checkout: &Path, path: &str, content: &[u8]) -> Result<u64, Error> {
    let file = path_inside_checkout(checkout, path)?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|error| io(parent, error))?;
    }
    std::fs::write(&file, content).map_err(|error| io(&file, error))?;
    Ok(content.len() as u64)
}

/// Skips files git ignores.
pub fn search_literal(checkout: &Path, pattern: &str) -> Vec<Match> {
    let mut matches = Vec::new();
    for entry in ignore::WalkBuilder::new(checkout).build().flatten() {
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let Ok(file) = std::fs::File::open(entry.path()) else {
            continue;
        };
        let relative = wire_path(entry.path().strip_prefix(checkout).expect("the walker yields paths under its root"));
        for (index, line) in BufReader::new(file).lines().map_while(Result::ok).enumerate() {
            if line.contains(pattern) {
                matches.push(Match { path: relative.clone(), line: index as u64 + 1, text: line });
                if matches.len() == MAX_MATCHES {
                    return matches;
                }
            }
        }
    }
    matches
}
