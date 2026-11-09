//! Where a task's files live: its worktree's branch, creation and removal, and the watch on its checkout.

use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use octopus_proto::types::{Checkout, TaskId};
use octopus_proto::{Error, Event};

use crate::store::TaskRow;
use crate::{Server, State, files, lock, report};

pub const WORKTREE_LIFETIME_MS: i64 = 7 * 24 * 60 * 60 * 1000;

fn slug(title: &str) -> String {
    let words: Vec<String> =
        title.split(|c: char| !c.is_alphanumeric()).filter(|word| !word.is_empty()).map(str::to_lowercase).collect();
    if words.is_empty() { "task".to_string() } else { words.join("-") }
}

fn run_git(repo: &Path, args: &[&str]) -> Result<Output, Error> {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|error| Error::internal(format!("run git: {error}")))
}

fn git_output(repo: &Path, args: &[&str]) -> Result<Vec<u8>, Error> {
    let output = run_git(repo, args)?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    Err(Error::conflict(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim())))
}

fn git(repo: &Path, args: &[&str]) -> Result<(), Error> {
    git_output(repo, args).map(drop)
}

fn branch_exists(repo: &Path, branch: &str) -> Result<bool, Error> {
    let output = run_git(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")])?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(Error::conflict(format!(
            "git rev-parse in {}: {}",
            repo.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

pub(crate) fn branch_for(repo: &Path, title: &str, task: &TaskId) -> Result<String, Error> {
    let branch = format!("task/{}", slug(title));
    if branch_exists(repo, &branch)? { Ok(format!("{branch}-{}", &task.0[TaskId::PREFIX.len()..])) } else { Ok(branch) }
}

/// A copy that fails to fill leaves no directory, and no branch it made.
pub fn create(repo: &Path, directory: &Path, checkout: &Checkout) -> Result<(), Error> {
    let (branch, copy) = match checkout {
        Checkout::Root => return Ok(()),
        Checkout::Worktree { branch } => (branch, false),
        Checkout::Copy { branch } => (branch, true),
    };
    let made_branch = !branch_exists(repo, branch)?;
    let target = directory.to_string_lossy();
    if made_branch {
        git(repo, &["worktree", "add", "-b", branch, &target])?;
    } else {
        git(repo, &["worktree", "add", &target, branch])?;
    }
    if !copy {
        return Ok(());
    }
    clone_ignored(repo, directory).map_err(|error| {
        let undone = remove(repo, directory)
            .and_then(|()| if made_branch { git(repo, &["branch", "-D", branch]) } else { Ok(()) });
        match undone {
            Ok(()) => error,
            Err(undo) => Error::conflict(format!("{error}; undoing the copy failed too: {undo}")),
        }
    })
}

/// Clones each file and folder git ignores in the root into the worktree, where it shares the root's blocks until
/// either side writes; an entry the worktree already holds stays.
fn clone_ignored(repo: &Path, worktree: &Path) -> Result<(), Error> {
    let listed = git_output(repo, &["ls-files", "-z", "--others", "--ignored", "--exclude-standard", "--directory"])?;
    for entry in listed.split(|byte| *byte == 0).filter(|entry| !entry.is_empty()) {
        let relative: PathBuf = Path::new(std::ffi::OsStr::from_bytes(entry)).components().collect();
        let target = worktree.join(&relative);
        if target.symlink_metadata().is_ok() {
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| Error::conflict(format!("create {}: {error}", parent.display())))?;
        }
        clone(&repo.join(&relative), &target).map_err(|error| {
            Error::conflict(format!(
                "clone {} into the copy: {error}; a copy needs {} and {} on one file system that clones files",
                relative.display(),
                repo.display(),
                worktree.display()
            ))
        })?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn clone(from: &Path, to: &Path) -> std::io::Result<()> {
    /// `<sys/clonefile.h>`: a link is cloned as the link, not as what it points to.
    const CLONE_NOFOLLOW: u32 = 0x0001;
    let path = |path: &Path| std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(std::io::Error::other);
    let (from, to) = (path(from)?, path(to)?);
    // SAFETY: both are NUL-terminated paths that outlive the call.
    if unsafe { libc::clonefile(from.as_ptr(), to.as_ptr(), CLONE_NOFOLLOW) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn clone(from: &Path, to: &Path) -> std::io::Result<()> {
    let output = Command::new("cp").args(["-a", "--reflink=always"]).arg(from).arg(to).output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(String::from_utf8_lossy(&output.stderr).trim().to_string()))
    }
}

pub fn remove(repo: &Path, directory: &Path) -> Result<(), Error> {
    git(repo, &["worktree", "remove", "--force", &directory.to_string_lossy()])
}

#[derive(Debug, Default)]
pub struct Sweep {
    pub removed: Vec<TaskId>,
    pub failed: Vec<(TaskId, Error)>,
}

impl Server {
    pub(crate) fn watch_checkout(&self, state: &mut State, row: &TaskRow) -> Result<(), Error> {
        use notify::Watcher;
        if state.watchers.contains_key(&row.id) {
            return Ok(());
        }
        let directory = PathBuf::from(&row.directory);
        let (changes, task, root) = (self.file_changes.clone(), row.id.clone(), directory.clone());
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
            Ok(event) => {
                for path in event.paths {
                    let Ok(relative) = path.strip_prefix(&root) else { continue };
                    if relative.components().all(|component| component.as_os_str() != ".git") {
                        let _server_gone = changes.send((task.clone(), files::wire_path(relative)));
                    }
                }
            }
            Err(error) => report!("watching {}: {error}", root.display()),
        })
        .map_err(Error::internal)?;
        watcher
            .watch(&directory, notify::RecursiveMode::Recursive)
            .map_err(|error| Error::internal(format!("watch {}: {error}", directory.display())))?;
        state.watchers.insert(row.id.clone(), watcher);
        Ok(())
    }

    pub(crate) fn file_changed(&self, task: &TaskId, path: &str) {
        let state = lock(&self.state);
        match state.store.rev() {
            Ok(rev) => self.publish(&state, rev, Event::FileChanged { task: task.clone(), path: path.to_string() }),
            Err(error) => report!("file change in {task}: {error}"),
        }
    }

    /// The branch of a removed worktree stays.
    pub fn remove_expired_worktrees(&self, now: i64) -> Result<Sweep, Error> {
        let cutoff = now - WORKTREE_LIFETIME_MS;
        let expired = lock(&self.state).store.worktrees_ended_by(cutoff)?;
        let mut sweep = Sweep::default();
        for row in expired {
            let directory = PathBuf::from(&row.directory);
            let root = {
                let state = lock(&self.state);
                let current = state.store.task(&row.id)?;
                if current.ended_at.is_none_or(|ended_at| ended_at > cutoff) || !directory.exists() {
                    continue;
                }
                state.store.workspace(&row.workspace).map(|workspace| PathBuf::from(workspace.root))
            };
            match root.and_then(|root| remove(&root, &directory)) {
                Ok(()) => sweep.removed.push(row.id),
                Err(error) => sweep.failed.push((row.id, error)),
            }
        }
        Ok(sweep)
    }
}
