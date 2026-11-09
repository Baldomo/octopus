mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{connect, create, refused, server, task, task_create, workspace, workspace_add};
use octopus_proto::ops::{
    SessionSync, TaskArchive, TaskCreate, TaskMove, TaskRename, TaskSetParent, WorkspaceAdd, WorkspaceList,
    WorkspaceRemove, WorkspaceRename, WorkspaceSetColor,
};
use octopus_proto::types::{Checkout, CheckoutKind, Color, Hue, Swatch, TaskId, TaskState, Tone, WorkspaceId};
use octopus_proto::{Error, Event, Op, Outcome};
use octopus_server::WORKTREE_LIFETIME_MS;
use serde_json::json;

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git").arg("-C").arg(repo).args(args).output().expect("run git");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// A git repository with one commit on `main`, under `dir`.
fn repo(dir: &Path) -> PathBuf {
    let repo = dir.join("repo");
    std::fs::create_dir(&repo).expect("mkdir repo");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "root"]);
    repo
}

#[tokio::test(flavor = "multi_thread")]
async fn a_worktree_lives_on_its_branch_until_its_lifetime_after_the_task_ends() {
    let (dir, running) = server().await;
    let repo = repo(dir.path());
    let laptop = connect(&running.socket, "laptop").await;
    let workspace = workspace(&laptop, &repo).await;

    let id = create(&laptop, &workspace, CheckoutKind::Worktree, "none").await;
    let view = task(&laptop, &id).await;
    let directory = dir.path().join("worktrees").join("octopus").join(&id.0);
    assert_eq!(Path::new(&view.directory), directory);
    assert_eq!(view.checkout, Checkout::Worktree { branch: "task/strict-serde".into() });
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

#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread")]
async fn a_copy_starts_with_the_roots_ignored_files_and_gets_them_again_when_it_reopens() {
    let (dir, running) = server().await;
    let repo = repo(dir.path());
    std::fs::write(repo.join(".gitignore"), "target/\n.env\n").expect("write .gitignore");
    git(&repo, &["add", ".gitignore"]);
    git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "ignore"]);
    std::fs::create_dir(repo.join("target")).expect("mkdir target");
    std::fs::write(repo.join("target").join("built"), "cache").expect("write build output");
    std::fs::write(repo.join(".env"), "KEY=1").expect("write .env");
    std::fs::write(repo.join("draft.md"), "untracked").expect("write untracked file");
    let laptop = connect(&running.socket, "laptop").await;
    let workspace = workspace(&laptop, &repo).await;

    let id = create(&laptop, &workspace, CheckoutKind::Copy, "none").await;
    let view = task(&laptop, &id).await;
    let copy = PathBuf::from(&view.directory);
    assert_eq!(view.checkout, Checkout::Copy { branch: "task/strict-serde".into() });
    assert_eq!(git(&copy, &["branch", "--show-current"]), "task/strict-serde");
    let read = |path: PathBuf| std::fs::read_to_string(path).expect("read cloned file");
    assert_eq!((read(copy.join("target/built")), read(copy.join(".env"))), ("cache".into(), "KEY=1".into()));
    assert!(!copy.join("draft.md").exists());
    std::fs::write(copy.join("target/built"), "rebuilt").expect("write in the copy");
    assert_eq!(read(repo.join("target/built")), "cache");

    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Completed }).await.expect("complete");
    let ended_at = task(&laptop, &id).await.ended_at.expect("ended_at");
    let due = running.server.remove_expired_worktrees(ended_at + WORKTREE_LIFETIME_MS).expect("sweep");
    assert_eq!((due.removed, due.failed.len()), (vec![id.clone()], 0));
    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Pinned }).await.expect("reopen");
    assert_eq!(read(copy.join("target/built")), "cache");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_renames_and_recolors_in_place_and_goes_with_its_tasks_once_none_is_live() {
    let (dir, running) = server().await;
    let repo = repo(dir.path());
    let laptop = connect(&running.socket, "laptop").await;
    let teal = Color::Picked(Swatch { hue: Hue::Teal, tone: Tone::Deep });
    let add = |name: &str, root: &Path| WorkspaceAdd { name: name.into(), color: teal, ..workspace_add(root) };
    assert!(matches!(refused(laptop.call(&add("x", Path::new("repo"))).await), Error::InvalidArgs { .. }));
    assert!(matches!(refused(laptop.call(&add(" ", &repo)).await), Error::InvalidArgs { .. }));
    let doomed = workspace(&laptop, &repo).await;
    let other = laptop.call(&add("dotfiles", dir.path())).await.expect("workspace.add").workspace;
    let spare = dir.path().join("spare");
    std::fs::create_dir(&spare).expect("mkdir spare");
    assert!(matches!(refused(laptop.call(&add("octopus", &spare)).await), Error::Conflict { .. }));

    let rename = |name: &str| WorkspaceRename { workspace: doomed.clone(), name: name.into() };
    assert!(matches!(refused(laptop.call(&rename("dotfiles")).await), Error::Conflict { .. }));
    laptop.call(&rename("acme")).await.expect("workspace.rename");
    let names: Vec<String> =
        laptop.call(&WorkspaceList {}).await.expect("list").workspaces.into_iter().map(|w| w.name).collect();
    assert_eq!(names, ["acme", "dotfiles"]);

    let violet = Color::Picked(Swatch { hue: Hue::Violet, tone: Tone::Vivid });
    laptop.call(&WorkspaceSetColor { workspace: doomed.clone(), color: violet }).await.expect("workspace.set_color");
    let colors: Vec<(String, Color)> =
        laptop.call(&WorkspaceList {}).await.expect("list").workspaces.into_iter().map(|w| (w.name, w.color)).collect();
    assert_eq!(colors, [("acme".into(), violet), ("dotfiles".into(), teal)]);
    let plaid = laptop.call_raw(WorkspaceSetColor::NAME, json!({"workspace": doomed, "color": "plaid"})).await;
    assert!(matches!(plaid.expect("call"), Outcome::Err(Error::InvalidArgs { .. })));

    let id = create(&laptop, &doomed, CheckoutKind::Worktree, "none").await;
    laptop.call(&TaskRename { task: id.clone(), title: "Renamed".into() }).await.expect("task.rename");
    let view = task(&laptop, &id).await;
    assert_eq!(
        (view.title.as_str(), view.checkout),
        ("Renamed", Checkout::Worktree { branch: "task/strict-serde".into() })
    );
    let child = TaskCreate { parent: Some(id.clone()), ..task_create(&other, CheckoutKind::Root, "none") };
    let child = laptop.call(&child).await.expect("task.create").task;

    let remove = WorkspaceRemove { workspace: doomed.clone() };
    assert!(matches!(refused(laptop.call(&remove).await), Error::Conflict { .. }));
    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Cancelled }).await.expect("cancel");
    laptop.call(&remove).await.expect("workspace.remove");
    let snapshot = laptop.call(&SessionSync { since_rev: None }).await.expect("sync");
    assert!(snapshot.workspaces.iter().all(|workspace| workspace.id != doomed));
    assert!(snapshot.tasks.iter().all(|task| task.id != id));
    assert_eq!(task(&laptop, &child).await.parent, None);
    assert!(!Path::new(&view.directory).exists());
    assert_eq!(git(&repo, &["branch", "--list", "task/strict-serde"]), "task/strict-serde");
}

#[tokio::test(flavor = "multi_thread")]
async fn only_an_ended_task_archives_and_a_move_unarchives_it() {
    let (dir, running) = server().await;
    let laptop = connect(&running.socket, "laptop").await;
    let id = create(&laptop, &workspace(&laptop, dir.path()).await, CheckoutKind::Root, "none").await;
    let archive = TaskArchive { task: id.clone() };
    assert!(matches!(refused(laptop.call(&archive).await), Error::Conflict { .. }));
    let empty = TaskRename { task: id.clone(), title: "".into() };
    assert!(matches!(refused(laptop.call(&empty).await), Error::InvalidArgs { .. }));

    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Completed }).await.expect("complete");
    laptop.call(&archive).await.expect("task.archive");
    assert!(task(&laptop, &id).await.archived_at.is_some());
    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Cancelled }).await.expect("cancel");
    assert_eq!(task(&laptop, &id).await.archived_at, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_nests_under_another_task_of_its_workspace_outside_its_own_subtree() {
    let (dir, running) = server().await;
    let laptop = connect(&running.socket, "laptop").await;
    let desk = connect(&running.socket, "desk").await;
    let octopus = workspace(&laptop, dir.path()).await;
    let elsewhere_root = dir.path().join("dotfiles");
    std::fs::create_dir(&elsewhere_root).expect("mkdir dotfiles");
    let dotfiles = WorkspaceAdd { name: "dotfiles".into(), ..workspace_add(&elsewhere_root) };
    let dotfiles = laptop.call(&dotfiles).await.expect("workspace.add").workspace;
    let make = async |workspace: &WorkspaceId, parent: Option<&TaskId>, state| {
        let create =
            TaskCreate { parent: parent.cloned(), state, ..task_create(workspace, CheckoutKind::Root, "none") };
        laptop.call(&create).await.expect("task.create").task
    };
    let root = make(&octopus, None, TaskState::Pinned).await;
    let child = make(&octopus, Some(&root), TaskState::Temporary).await;
    let grandchild = make(&octopus, Some(&child), TaskState::Pinned).await;
    let other = make(&octopus, None, TaskState::Completed).await;
    let elsewhere = make(&dotfiles, None, TaskState::Pinned).await;
    let set = |task: &TaskId, parent: Option<&TaskId>| TaskSetParent { task: task.clone(), parent: parent.cloned() };

    for (task, parent) in [(&root, &root), (&root, &grandchild), (&child, &elsewhere)] {
        assert!(matches!(refused(laptop.call(&set(task, Some(parent))).await), Error::Conflict { .. }));
    }
    let missing = TaskId("tsk_missing".into());
    assert!(matches!(refused(laptop.call(&set(&root, Some(&missing))).await), Error::NotFound { .. }));
    assert!(matches!(refused(laptop.call(&set(&missing, None)).await), Error::NotFound { .. }));

    let mut pushes = desk.pushes();
    let rev = laptop.call(&set(&child, Some(&other))).await.expect("task.set_parent").rev;
    let moved = task(&laptop, &child).await;
    assert_eq!((moved.parent.as_ref(), moved.state, moved.rev), (Some(&other), TaskState::Temporary, rev));
    assert_eq!(task(&laptop, &grandchild).await.parent.as_ref(), Some(&child));
    let push = pushes.recv().await.expect("push");
    assert!(matches!(&push.event, Event::TaskUpdated { task } if task.id == child && push.rev == rev), "{push:?}");

    laptop.call(&set(&child, None)).await.expect("task.set_parent");
    assert_eq!(task(&laptop, &child).await.parent, None);
}
