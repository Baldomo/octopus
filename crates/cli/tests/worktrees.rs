mod common;

use std::path::Path;
use std::process::Command;

use common::{connect, create, start, task, workspace};
use octopus_proto::ops::TaskMove;
use octopus_proto::types::{Checkout, TaskState};
use octopus_server::WORKTREE_LIFETIME_MS;

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git").arg("-C").arg(repo).args(args).output().expect("run git");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_worktree_lives_on_its_branch_until_its_lifetime_after_the_task_ends() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).expect("mkdir repo");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "root"]);
    let running = start(dir.path(), "server.sock", "host.sock");
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, &repo).await;

    let id = create(&laptop, &workspace, Checkout::Worktree, "none").await;
    let view = task(&laptop, &id).await;
    let directory = dir.path().join("worktrees").join("octopus").join(&id.0);
    assert_eq!(Path::new(&view.directory), directory);
    assert_eq!(view.branch.as_deref(), Some("task/strict-serde"));
    assert_eq!(git(&directory, &["branch", "--show-current"]), "task/strict-serde");

    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Completed }).await.expect("complete");
    let ended_at = task(&laptop, &id).await.ended_at.expect("ended_at");
    let early = running.server.remove_expired_worktrees(ended_at + WORKTREE_LIFETIME_MS - 1).expect("sweep");
    assert!(early.removed.is_empty() && early.failed.is_empty());
    assert!(directory.exists());
    let due = running.server.remove_expired_worktrees(ended_at + WORKTREE_LIFETIME_MS).expect("sweep");
    assert_eq!((due.removed, due.failed.len()), (vec![id.clone()], 0));
    assert!(!directory.exists());
    assert_eq!(git(&repo, &["branch", "--list", "task/strict-serde"]), "task/strict-serde");

    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Pinned }).await.expect("reopen");
    assert_eq!(git(&directory, &["branch", "--show-current"]), "task/strict-serde");
}
