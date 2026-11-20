//! A folder on the server's machine: the directory a path resolves to, the git repo it sits in, the branch that repo is
//! on, and the folders inside it, read without looking inside any of them so the OS asks the user nothing.

use std::ffi::{OsStr, OsString};
use std::fs::DirEntry;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use nix::unistd::{AccessFlags, access};
use octopus_proto::Error;
use octopus_proto::ops::{Folder, FolderGet, Inside, OsSetting, Subfolder};

use crate::git::{git_command, run_git};
use crate::login;

fn absolute(what: &str, path: &str) -> Result<(), Error> {
    match Path::new(path).is_absolute() {
        true => Ok(()),
        false => Err(Error::invalid(format!("{what} {path} is not an absolute path"))),
    }
}

/// A root or folder, `what` naming which: an absolute path to a directory, its symlinks, `.` and `..` resolved.
pub(crate) fn directory(what: &str, path: &str) -> Result<String, Error> {
    absolute(what, path)?;
    let canonical = std::fs::canonicalize(path).map_err(|error| Error::invalid(format!("{what} {path}: {error}")))?;
    if !canonical.is_dir() {
        return Err(Error::invalid(format!("{what} {path} is not a directory")));
    }
    Ok(canonical.to_string_lossy().into_owned())
}

/// The top of the git repo `folder` sits in, resolved, or None when it sits in none or the machine has no git.
pub(crate) fn repo(folder: &Path) -> Result<Option<PathBuf>, Error> {
    let output = match git_command(folder, &["rev-parse", "--show-toplevel"]).env("LC_ALL", "C").output() {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::internal(format!("run git: {error}"))),
    };
    if !output.status.success() {
        let why = String::from_utf8_lossy(&output.stderr);
        if why.contains("not a git repository") || why.contains("must be run in a work tree") {
            return Ok(None);
        }
        return Err(Error::conflict(format!("git rev-parse in {}: {}", folder.display(), why.trim())));
    }
    let top = Path::new(OsStr::from_bytes(output.stdout.strip_suffix(b"\n").unwrap_or(&output.stdout)));
    std::fs::canonicalize(top).map(Some).map_err(|error| Error::internal(format!("{}: {error}", top.display())))
}

/// The branch `repo`'s HEAD is on, or None on a detached HEAD.
fn head_branch(repo: &Path) -> Result<Option<String>, Error> {
    let output = run_git(repo, &["symbolic-ref", "--short", "--quiet", "HEAD"])?;
    Ok(output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim_end().to_string()))
}

/// `~` reads the home folder of the server's user. Reading a folder the OS guards waits for the OS to let the server
/// in, which on macOS can mean the user answering a prompt.
pub(crate) fn folder_get(op: FolderGet) -> Result<Folder, Error> {
    let home = std::fs::canonicalize(login::home()?).map_err(Error::internal)?;
    let asked = match op.folder.as_str() {
        "~" => home.to_string_lossy().into_owned(),
        path => path.to_string(),
    };
    absolute("folder", &asked)?;
    let opened = match std::fs::canonicalize(&asked) {
        Ok(folder) => match std::fs::read_dir(&folder) {
            Ok(listed) => Ok((listed, folder)),
            Err(error) => Err((folder, error)),
        },
        Err(error) => Err((Path::new(&asked).components().collect(), error)),
    };
    let (listed, folder) = match opened {
        Ok(opened) => opened,
        Err((folder, error)) if error.kind() == io::ErrorKind::PermissionDenied => {
            let inside = Inside::KeptOut { setting: kept_out_by(&home, &folder, &error) };
            return Ok(Folder { folder: folder.to_string_lossy().into_owned(), repo: None, branch: None, inside });
        }
        Err((_, error)) => return Err(Error::invalid(format!("folder {asked}: {error}"))),
    };
    let here = guarded_place(&home, &folder).map(|(place, _)| place);
    let mut folders: Vec<Subfolder> =
        listed.flatten().filter_map(|entry| sighted(&entry, &folder, &home, here.as_deref())).collect();
    folders.sort_by(|one, other| one.name.cmp(&other.name));
    let repo = repo(&folder)?;
    let branch = repo.as_deref().map(head_branch).transpose()?.flatten();
    Ok(Folder {
        folder: folder.to_string_lossy().into_owned(),
        repo: repo.map(|repo| repo.to_string_lossy().into_owned()),
        branch,
        inside: Inside::Listed { folders },
    })
}

/// The entry, when it is a folder or a symlink that resolves to one. It is guarded when its path enters a place the OS
/// guards beyond `here`, the listed folder's, or its user may not read and open it.
fn sighted(entry: &DirEntry, folder: &Path, home: &Path, here: Option<&Path>) -> Option<Subfolder> {
    let enters = |path: &Path| guarded_place(home, path).is_some_and(|(place, _)| Some(place.as_path()) != here);
    let kind = entry.file_type().ok()?;
    let target = match kind.is_symlink() {
        true => resolve(folder, entry.file_name(), enters)?,
        false => kind.is_dir().then(|| entry.path())?,
    };
    let guarded = enters(&target) || access(&target, AccessFlags::R_OK | AccessFlags::X_OK).is_err();
    Some(Subfolder { name: entry.file_name().to_string_lossy().into_owned(), guarded })
}

/// Where the symlink `link` inside `folder` leads: the folder it resolves to, or the first path on its way that
/// `enters` a guarded place, which is never statted. Resolves one path component at a time, as the kernel would, so no
/// symlink on the way leads the kernel into a guarded place either. None when it leads to no folder.
fn resolve(folder: &Path, link: OsString, enters: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let mut resolved = folder.to_path_buf();
    let mut ahead = vec![link];
    let mut links = 0;
    let mut is_folder = true;
    while let Some(part) = ahead.pop() {
        if part == ".." {
            resolved.pop();
            is_folder = true;
            continue;
        }
        let next = resolved.join(&part);
        if enters(&next) {
            return Some(next);
        }
        let kind = std::fs::symlink_metadata(&next).ok()?.file_type();
        if !kind.is_symlink() {
            (resolved, is_folder) = (next, kind.is_dir());
            continue;
        }
        links += 1;
        if links > SYMLINK_LOOP {
            return None;
        }
        let target = std::fs::read_link(&next).ok()?;
        if target.is_absolute() {
            resolved = PathBuf::from("/");
        }
        ahead.extend(target.components().rev().filter_map(|part| match part {
            Component::Normal(part) => Some(part.to_os_string()),
            Component::ParentDir => Some("..".into()),
            Component::RootDir | Component::CurDir | Component::Prefix(_) => None,
        }));
    }
    is_folder.then_some(resolved)
}

/// Past this many symlinks a chain counts as a loop, as Linux counts.
const SYMLINK_LOOP: usize = 40;

type Setting = (&'static str, &'static str);

const FULL_DISK_ACCESS: Setting = ("Full Disk Access", "Privacy_AllFiles");

/// The place macOS asks the user about before letting the server into `path`, when `path` is or lies in one, and the
/// setting that lets the server in: Files & Folders for the Desktop, Documents and Downloads folders, iCloud Drive,
/// each cloud storage provider's folder and each volume, and Full Disk Access for each other app's data. Matches as
/// macOS's filesystem does, ignoring case, and reads the data volume's path to a folder as the folder.
#[cfg(target_os = "macos")]
fn guarded_place(home: &Path, path: &Path) -> Option<(PathBuf, Setting)> {
    const FILES_AND_FOLDERS: Setting = ("Files & Folders", "Privacy_FilesAndFolders");
    let lowered = |path: &Path| PathBuf::from(path.to_string_lossy().to_lowercase());
    let (home, path) = (lowered(home), lowered(path));
    let path =
        path.strip_prefix("/system/volumes/data").map_or_else(|_| path.clone(), |rest| Path::new("/").join(rest));
    let folders = ["desktop", "documents", "downloads", "library/mobile documents"].map(|folder| home.join(folder));
    let each_inside = [
        (home.join("library/cloudstorage"), FILES_AND_FOLDERS),
        (PathBuf::from("/volumes"), FILES_AND_FOLDERS),
        (home.join("library/containers"), FULL_DISK_ACCESS),
        (home.join("library/group containers"), FULL_DISK_ACCESS),
    ];
    let folder = folders.into_iter().find(|folder| path.starts_with(folder)).map(|folder| (folder, FILES_AND_FOLDERS));
    folder.or_else(|| {
        each_inside
            .into_iter()
            .find_map(|(parent, setting)| Some((parent.join(path.strip_prefix(&parent).ok()?.iter().next()?), setting)))
    })
}

/// Linux asks the user about no place; its folders are guarded by their permissions alone.
#[cfg(not(target_os = "macos"))]
fn guarded_place(_home: &Path, _path: &Path) -> Option<(PathBuf, Setting)> {
    None
}

/// The macOS privacy setting that lets the server into `folder`, which the OS kept it out of with `error`, or None when
/// the folder's permissions did.
fn kept_out_by(home: &Path, folder: &Path, error: &io::Error) -> Option<OsSetting> {
    (cfg!(target_os = "macos") && error.raw_os_error() == Some(nix::libc::EPERM)).then(|| {
        let (pane, anchor) = guarded_place(home, folder).map_or(FULL_DISK_ACCESS, |(_, setting)| setting);
        OsSetting {
            name: format!("System Settings › Privacy & Security › {pane}"),
            url: format!("x-apple.systempreferences:com.apple.preference.security?{anchor}"),
        }
    })
}
