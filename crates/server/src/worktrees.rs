//! A task's mode and directory: its folder, or the same folder inside a worktree of the folder's repo that the task
//! holds on a branch of its own, which a copy fills with the repo's ignored files; and the watch on the directory.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use octopus_proto::types::{Mode, TaskId};
use octopus_proto::{Error, Event};

use crate::folders::repo;
use crate::git::{git, git_output, run_git};
use crate::store::{TaskRow, Worktree};
use crate::{Server, State, files, lock, report};

pub const WORKTREE_LIFETIME_MS: i64 = 7 * 24 * 60 * 60 * 1000;

const SLUG_CHARS: usize = 40;

/// The title lowercased, its runs of letters and digits joined by `-`, at most `SLUG_CHARS` long, or `task`.
fn slug(title: &str) -> String {
    let words: Vec<String> =
        title.split(|c: char| !c.is_alphanumeric()).filter(|word| !word.is_empty()).map(str::to_lowercase).collect();
    let cut: String = words.join("-").chars().take(SLUG_CHARS).collect();
    match cut.trim_end_matches('-') {
        "" => "task".to_string(),
        slug => slug.to_string(),
    }
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

/// Adds a worktree of the repo `folder` sits in at `<worktrees>/<repo folder name>/<slug>`, on a new branch from HEAD
/// named after the title, and returns it with the task's directory, the folder's place inside it. A path some task
/// `held` stays its own.
pub(crate) fn branch_off(
    folder: &Path,
    worktrees: &Path,
    title: &str,
    mode: Mode,
    held: impl Fn(&str) -> Result<bool, Error>,
) -> Result<(Worktree, PathBuf), Error> {
    let repo = repo(folder)?.ok_or_else(|| {
        Error::invalid(format!("{} sits in no git repo, so a task works in it only in place", folder.display()))
    })?;
    let repo_name = repo.file_name().ok_or_else(|| {
        Error::invalid(format!("the repo {} has no folder name to file its worktrees under", repo.display()))
    })?;
    let base = slug(title);
    let mut name = base.clone();
    for count in 2.. {
        let path = worktrees.join(repo_name).join(&name).to_string_lossy().into_owned();
        let branch = format!("task/{name}");
        if Path::new(&path).symlink_metadata().is_err() && !held(&path)? && !branch_exists(&repo, &branch)? {
            let worktree = Worktree { path, branch };
            let directory = add(&repo, folder, mode, &worktree)?;
            return Ok((worktree, directory));
        }
        name = format!("{base}-{count}");
    }
    unreachable!("an unbounded run of names holds one free")
}

/// The lock of each worktree path a restore runs on, so a caller finds a worktree another brings back whole, never
/// half made. A path's entry goes with the last restore waiting on it.
static RESTORING: LazyLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = LazyLock::new(Mutex::default);

fn one_restore_at_a_time<T>(path: &str, run: impl FnOnce() -> T) -> T {
    let one = Arc::clone(lock(&RESTORING).entry(path.to_string()).or_default());
    let ran = {
        let _one = lock(&*one);
        run()
    };
    let mut restoring = lock(&RESTORING);
    if Arc::strong_count(&one) == 2 {
        restoring.remove(path);
    }
    ran
}

/// Brings the task's missing worktree back at its path, on its branch. A present worktree runs on, its repo gone or
/// not.
fn restore(row: &TaskRow, worktree: &Worktree) -> Result<(), Error> {
    one_restore_at_a_time(&worktree.path, || {
        if Path::new(&worktree.path).exists() {
            return Ok(());
        }
        let folder = Path::new(&row.folder);
        let repo = repo(folder)?.ok_or_else(|| {
            Error::conflict(format!(
                "{} sits in no git repo now, so the worktree {} cannot come back",
                folder.display(),
                worktree.path
            ))
        })?;
        git(&repo, &["worktree", "prune"])?;
        add(&repo, folder, row.mode, worktree).map(drop)
    })
}

/// Removes a worktree `branch_off` made for a task that never came to be, with its branch.
pub(crate) fn discard(folder: &Path, worktree: &Worktree) -> Result<(), Error> {
    remove(&worktree.path)?;
    match repo(folder)? {
        Some(repo) => git(&repo, &["branch", "-D", &worktree.branch]),
        None => Ok(()),
    }
}

/// Adds the worktree on its branch, from HEAD when the repo lacks the branch, then the folder's subfolder inside it
/// and, for a copy, the repo's ignored files. A failure leaves neither the worktree nor a branch it made.
fn add(repo: &Path, folder: &Path, mode: Mode, worktree: &Worktree) -> Result<PathBuf, Error> {
    let subfolder = folder
        .strip_prefix(repo)
        .map_err(|_| Error::conflict(format!("{} is outside its repo {}", folder.display(), repo.display())))?;
    let (path, branch) = (Path::new(&worktree.path), worktree.branch.as_str());
    let made_branch = !branch_exists(repo, branch)?;
    if made_branch {
        git(repo, &["worktree", "add", "-b", branch, &worktree.path])?;
    } else {
        git(repo, &["worktree", "add", &worktree.path, branch])?;
    }
    let directory = path.join(subfolder);
    let filled = std::fs::create_dir_all(&directory)
        .map_err(|error| Error::conflict(format!("create {}: {error}", directory.display())))
        .and_then(|()| if mode == Mode::Copy { clone_ignored(repo, path) } else { Ok(()) });
    filled.map(|()| directory).map_err(|error| {
        let undone = remove(&worktree.path)
            .and_then(|()| if made_branch { git(repo, &["branch", "-D", branch]) } else { Ok(()) });
        match undone {
            Ok(()) => error,
            Err(undo) => Error::conflict(format!("{error}; undoing the worktree failed too: {undo}")),
        }
    })
}

/// Clones each file and folder git ignores in the repo into the worktree, where it shares the repo's blocks until
/// either side writes; an entry the worktree already holds stays.
fn clone_ignored(repo: &Path, worktree: &Path) -> Result<(), Error> {
    let listed = git_output(repo, &["ls-files", "-z", "--others", "--ignored", "--exclude-standard", "--directory"])?;
    for entry in listed.split(|byte| *byte == 0).filter(|entry| !entry.is_empty()) {
        let relative: PathBuf = Path::new(OsStr::from_bytes(entry)).components().collect();
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
    let output = std::process::Command::new("cp").args(["-a", "--reflink=always"]).arg(from).arg(to).output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(String::from_utf8_lossy(&output.stderr).trim().to_string()))
    }
}

/// Removes the worktree with any uncommitted change.
pub fn remove(worktree: &str) -> Result<(), Error> {
    git(Path::new(worktree), &["worktree", "remove", "--force", worktree])
}

#[derive(Debug, Default)]
pub struct Sweep {
    pub removed: Vec<TaskId>,
    pub failed: Vec<(TaskId, Error)>,
}

impl Server {
    /// The task's row under the state lock, with its directory there whenever `runs` holds for the row. A missing
    /// worktree comes back outside the lock, since a copy clones its files, and a directory still missing fails
    /// `not_found`.
    pub(crate) fn ready_task(
        &self,
        task: &TaskId,
        runs: impl Fn(&TaskRow) -> bool,
    ) -> Result<(MutexGuard<'_, State>, TaskRow), Error> {
        let row = lock(&self.state).store.task(task)?;
        if runs(&row)
            && let Some(worktree) = &row.worktree
        {
            restore(&row, worktree)?;
        }
        let state = lock(&self.state);
        let row = state.store.task(task)?;
        if runs(&row) && !Path::new(&row.directory).is_dir() {
            return Err(Error::not_found(&row.directory));
        }
        Ok((state, row))
    }

    pub(crate) fn watch_directory(&self, state: &mut State, row: &TaskRow) -> Result<(), Error> {
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
        for (task, worktree) in expired {
            let ended_at = lock(&self.state).store.task(&task)?.ended_at;
            if ended_at.is_none_or(|ended_at| ended_at > cutoff) || !Path::new(&worktree.path).exists() {
                continue;
            }
            match remove(&worktree.path) {
                Ok(()) => sweep.removed.push(task),
                Err(error) => sweep.failed.push((task, error)),
            }
        }
        Ok(sweep)
    }
}

#[cfg(test)]
mod tests {
    use super::slug;

    #[test]
    fn a_slug_joins_the_titles_words_in_lowercase_up_to_40_characters() {
        assert_eq!(slug("Strict serde: no defaults!"), "strict-serde-no-defaults");
        assert_eq!(slug("?!"), "task");
        assert_eq!(slug(&"word ".repeat(20)), "word-word-word-word-word-word-word-word");
    }
}
