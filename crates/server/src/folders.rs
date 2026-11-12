//! A folder on the server's machine: the directory a path resolves to, the git repo it sits in, the branch that repo is
//! on, and the folders inside it.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use octopus_proto::Error;
use octopus_proto::ops::{Folder, FolderGet};

use crate::git::{git_command, run_git};

/// A root or folder, `what` naming which: an absolute path to a directory, its symlinks, `.` and `..` resolved.
pub(crate) fn directory(what: &str, path: &str) -> Result<String, Error> {
    if !Path::new(path).is_absolute() {
        return Err(Error::invalid(format!("{what} {path} is not an absolute path")));
    }
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
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
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

pub(crate) fn folder_get(op: FolderGet) -> Result<Folder, Error> {
    let folder = directory("folder", &op.folder)?;
    let repo = repo(Path::new(&folder))?;
    let branch = repo.as_deref().map(head_branch).transpose()?.flatten();
    let repo = repo.map(|repo| repo.to_string_lossy().into_owned());
    let listed = std::fs::read_dir(&folder).map_err(|error| Error::invalid(format!("folder {folder}: {error}")))?;
    let mut folders: Vec<String> = listed
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    folders.sort();
    Ok(Folder { folder, repo, branch, folders })
}
