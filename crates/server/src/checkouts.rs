use std::path::Path;
use std::process::{Command, Output};

use octopus_proto::Error;
use octopus_proto::types::TaskId;

pub const WORKTREE_LIFETIME_MS: i64 = 7 * 24 * 60 * 60 * 1000;

pub fn slug(title: &str) -> String {
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

pub fn branch_for(repo: &Path, title: &str, task: &TaskId) -> Result<String, Error> {
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

#[cfg(test)]
mod tests {
    use super::slug;

    #[test]
    fn a_slug_keeps_words_and_joins_them_with_dashes() {
        assert_eq!(slug("Strict serde: envelope!"), "strict-serde-envelope");
        assert_eq!(slug("  ...  "), "task");
    }
}
