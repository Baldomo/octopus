//! Where a task's files live: its worktree's branch, creation and removal, and the watch on its checkout.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use octopus_proto::types::TaskId;
use octopus_proto::{Error, Event};

use crate::store::TaskRow;
use crate::{Server, State, files, lock, report};

pub const WORKTREE_LIFETIME_MS: i64 = 7 * 24 * 60 * 60 * 1000;

pub(crate) fn slug(title: &str) -> String {
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

fn git(repo: &Path, args: &[&str]) -> Result<(), Error> {
    let output = run_git(repo, args)?;
    if output.status.success() {
        return Ok(());
    }
    Err(Error::conflict(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim())))
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

pub fn create(repo: &Path, directory: &Path, branch: &str) -> Result<(), Error> {
    let directory = directory.to_string_lossy();
    if branch_exists(repo, branch)? {
        git(repo, &["worktree", "add", &directory, branch])
    } else {
        git(repo, &["worktree", "add", "-b", branch, &directory])
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

#[cfg(test)]
mod tests {
    use super::slug;

    #[test]
    fn a_slug_keeps_words_and_joins_them_with_dashes() {
        assert_eq!(slug("Strict serde: envelope!"), "strict-serde-envelope");
        assert_eq!(slug("  ...  "), "task");
    }
}
