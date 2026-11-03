//! A task's files as a client reaches them, kept inside the checkout no path leaves.

use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};

use octopus_proto::Error;
use octopus_proto::ops::{Entry, EntryKind, Match};

use octopus_proto::types::TaskId;

use crate::{Server, lock};

const MAX_MATCHES: usize = 1000;

fn io(path: &Path, error: std::io::Error) -> Error {
    match error.kind() {
        std::io::ErrorKind::NotFound => Error::not_found(path.display()),
        std::io::ErrorKind::AlreadyExists => Error::conflict(format!("{} already exists", path.display())),
        _ => Error::internal(format!("{}: {error}", path.display())),
    }
}

/// A checkout-relative path as the wire spells it, `/`-separated on every platform.
pub(crate) fn wire_path(relative: &Path) -> String {
    let components: Vec<_> = relative.components().map(|component| component.as_os_str().to_string_lossy()).collect();
    components.join("/")
}

fn outside(path: &str) -> Error {
    Error::invalid(format!("{path} is not a path inside the checkout"))
}

fn canonical(path: &Path) -> Result<PathBuf, Error> {
    path.canonicalize().map_err(|error| io(path, error))
}

/// The entry `path` names, every directory above it resolving inside the checkout, so no symlink on the way leads
/// out; the entry itself may be a symlink, which the caller acts on as a link.
fn entry(checkout: &Path, path: &str) -> Result<PathBuf, Error> {
    let relative = Path::new(path);
    if relative.components().any(|component| !matches!(component, Component::Normal(_) | Component::CurDir)) {
        return Err(outside(path));
    }
    let inside: PathBuf = relative.components().filter(|component| matches!(component, Component::Normal(_))).collect();
    if inside.as_os_str().is_empty() {
        return Ok(checkout.to_path_buf());
    }
    let joined = checkout.join(inside);
    let parent = joined.parent().expect("a path inside the checkout has a parent");
    if !resolves_within(nearest(parent), &canonical(checkout)?) {
        return Err(outside(path));
    }
    Ok(joined)
}

/// The deepest of `path` and its ancestors that is an entry, a dangling symlink included.
fn nearest(path: &Path) -> &Path {
    path.ancestors().find(|ancestor| ancestor.symlink_metadata().is_ok()).expect("the root directory exists")
}

/// A dangling symlink resolves nowhere, so it never counts as within.
fn resolves_within(path: &Path, root: &Path) -> bool {
    path.canonicalize().is_ok_and(|real| real.starts_with(root))
}

/// The entry `path` names, which resolves inside the checkout even through a symlink.
fn followed(checkout: &Path, path: &str) -> Result<PathBuf, Error> {
    let joined = entry(checkout, path)?;
    if joined.symlink_metadata().is_ok() && !resolves_within(&joined, &canonical(checkout)?) {
        return Err(outside(path));
    }
    Ok(joined)
}

/// An entry the checkout holds, never the checkout itself.
fn inner(checkout: &Path, path: &str) -> Result<PathBuf, Error> {
    if !Path::new(path).components().any(|component| matches!(component, Component::Normal(_))) {
        return Err(Error::invalid("the checkout itself cannot be moved, copied or deleted"));
    }
    let joined = entry(checkout, path)?;
    joined.symlink_metadata().map_err(|error| io(&joined, error))?;
    Ok(joined)
}

/// A path no entry holds yet.
fn vacant(checkout: &Path, path: &str) -> Result<PathBuf, Error> {
    let joined = entry(checkout, path)?;
    if joined.symlink_metadata().is_ok() {
        return Err(Error::conflict(format!("{path} already exists")));
    }
    Ok(joined)
}

/// Creates the directories above `target` that are missing.
fn make_parents(target: &Path) -> Result<&Path, Error> {
    let parent = target.parent().expect("a path inside the checkout has a parent");
    std::fs::create_dir_all(parent).map_err(|error| io(parent, error))?;
    Ok(parent)
}

pub fn list(checkout: &Path, path: &str) -> Result<Vec<Entry>, Error> {
    let directory = followed(checkout, path)?;
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

/// A directory of the checkout, for a shell to start in.
pub fn directory(checkout: &Path, path: &str) -> Result<PathBuf, Error> {
    let directory = followed(checkout, path)?;
    if !directory.is_dir() {
        return Err(Error::invalid(format!("{path} is not a directory of the checkout")));
    }
    Ok(directory)
}

pub fn read(checkout: &Path, path: &str) -> Result<Vec<u8>, Error> {
    let file = followed(checkout, path)?;
    std::fs::read(&file).map_err(|error| io(&file, error))
}

pub fn write(checkout: &Path, path: &str, content: &[u8]) -> Result<u64, Error> {
    let file = followed(checkout, path)?;
    make_parents(&file)?;
    std::fs::write(&file, content).map_err(|error| io(&file, error))?;
    Ok(content.len() as u64)
}

pub fn create(checkout: &Path, path: &str, kind: EntryKind) -> Result<(), Error> {
    let target = vacant(checkout, path)?;
    make_parents(&target)?;
    let made = match kind {
        EntryKind::File => std::fs::File::create_new(&target).map(drop),
        EntryKind::Directory => std::fs::create_dir(&target),
    };
    made.map_err(|error| io(&target, error))
}

/// A new file whose bytes gather beside it until they all arrive, so a broken upload leaves nothing at its path.
pub struct Upload {
    pub gathering: tempfile::NamedTempFile,
    pub bytes: u64,
    target: PathBuf,
}

pub fn upload(checkout: &Path, path: &str, bytes: u64) -> Result<Upload, Error> {
    let target = vacant(checkout, path)?;
    let parent = make_parents(&target)?;
    let gathering = tempfile::Builder::new()
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o644))
        .tempfile_in(parent)
        .map_err(|error| io(parent, error))?;
    Ok(Upload { gathering, bytes, target })
}

impl Upload {
    pub fn land(self) -> Result<(), Error> {
        self.gathering.persist_noclobber(&self.target).map(drop).map_err(|refused| io(&self.target, refused.error))
    }
}

/// Compares where the paths lead, so a symlink to the source on the way to `to` counts as inside it.
fn into_itself(from: &Path, to: &Path) -> Result<(), Error> {
    let directory = from.symlink_metadata().is_ok_and(|metadata| metadata.is_dir());
    if directory && resolves_within(nearest(to), &canonical(from)?) {
        return Err(Error::invalid(format!("{} cannot go inside itself", from.display())));
    }
    Ok(())
}

/// A rename that only changes the case of a name, on a file system that ignores case, finds the entry itself at its
/// new name.
fn same_entry(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (a.symlink_metadata(), b.symlink_metadata()) {
        (Ok(a), Ok(b)) => (a.dev(), a.ino()) == (b.dev(), b.ino()),
        _ => false,
    }
}

pub fn rename(checkout: &Path, from: &str, to: &str) -> Result<(), Error> {
    let source = inner(checkout, from)?;
    let target = entry(checkout, to)?;
    if !same_entry(&source, &target) {
        into_itself(&source, &target)?;
        make_parents(&vacant(checkout, to)?)?;
    }
    std::fs::rename(&source, &target).map_err(|error| io(&source, error))
}

/// A symlink copies as the link, so a copy never reads past the checkout.
fn copy_entry(from: &Path, to: &Path) -> Result<(), Error> {
    let kind = from.symlink_metadata().map_err(|error| io(from, error))?.file_type();
    if kind.is_symlink() {
        let link = std::fs::read_link(from).map_err(|error| io(from, error))?;
        std::os::unix::fs::symlink(link, to).map_err(|error| io(to, error))
    } else if kind.is_dir() {
        std::fs::create_dir(to).map_err(|error| io(to, error))?;
        for entry in std::fs::read_dir(from).map_err(|error| io(from, error))? {
            let entry = entry.map_err(|error| io(from, error))?;
            copy_entry(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(drop).map_err(|error| io(from, error))
    }
}

pub fn copy(checkout: &Path, from: &str, to: &str) -> Result<(), Error> {
    let source = inner(checkout, from)?;
    let target = vacant(checkout, to)?;
    into_itself(&source, &target)?;
    make_parents(&target)?;
    copy_entry(&source, &target)
}

pub fn delete(checkout: &Path, path: &str, trash: bool) -> Result<(), Error> {
    let target = inner(checkout, path)?;
    if trash {
        #[cfg_attr(not(target_os = "macos"), expect(unused_mut))]
        let mut context = trash::TrashContext::default();
        // Finder, the crate's default, would ask the user to let octopus control it.
        #[cfg(target_os = "macos")]
        trash::macos::TrashContextExtMacos::set_delete_method(&mut context, trash::macos::DeleteMethod::NsFileManager);
        return context
            .delete(&target)
            .map_err(|error| Error::internal(format!("trash {}: {error}", target.display())));
    }
    let removed = if target.symlink_metadata().is_ok_and(|metadata| metadata.is_dir()) {
        std::fs::remove_dir_all(&target)
    } else {
        std::fs::remove_file(&target)
    };
    removed.map_err(|error| io(&target, error))
}

/// Skips files git ignores.
pub(crate) fn search_literal(checkout: &Path, pattern: &str) -> Vec<Match> {
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

impl Server {
    pub(crate) fn checkout_dir(&self, task: &TaskId) -> Result<PathBuf, Error> {
        Ok(PathBuf::from(lock(&self.state).store.task(task)?.directory))
    }

    pub(crate) fn read_file(&self, task: &TaskId, path: &str) -> Result<Vec<u8>, Error> {
        read(&self.checkout_dir(task)?, path)
    }

    pub(crate) fn upload_file(&self, task: &TaskId, path: &str, bytes: u64) -> Result<Upload, Error> {
        upload(&self.checkout_dir(task)?, path, bytes)
    }
}
