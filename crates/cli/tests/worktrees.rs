mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{connect, create, refused, server, task, task_create, workspace, workspace_add};
use octopus_proto::ops::{
    Folder, FolderGet, Inside, SessionSync, Subfolder, TaskArchive, TaskCreate, TaskFocus, TaskMove, TaskRename,
    TaskSetParent, TaskSetWorkspace, TermOpen, WorkspaceAdd, WorkspaceList, WorkspaceRemove, WorkspaceRename,
    WorkspaceSetColor,
};
use octopus_proto::types::{Color, Hue, Mode, Swatch, TaskId, TaskState, Tone, WorkspaceId};
use octopus_proto::{Error, Event, Op, Outcome};
use octopus_server::WORKTREE_LIFETIME_MS;
use serde_json::json;

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git").arg("-C").arg(repo).args(args).output().expect("run git");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn folders(listed: &[(&str, bool)]) -> Vec<Subfolder> {
    listed.iter().map(|&(name, guarded)| Subfolder { name: name.into(), guarded }).collect()
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
async fn a_worktree_holds_the_folder_on_its_branch_and_comes_back_until_its_lifetime_after_the_task_ends() {
    let (dir, running) = server().await;
    let repo = repo(dir.path());
    let app = repo.join("app");
    std::fs::create_dir(&app).expect("mkdir app");
    let laptop = connect(&running.socket, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    let make = async |mode| laptop.call(&task_create(&workspace, &app, mode, "none")).await;

    let id = make(Mode::Worktree).await.expect("task.create").task;
    let view = task(&laptop, &id).await;
    let worktree = dir.path().join("worktrees").join("repo").join("strict-serde");
    assert_eq!(Path::new(&view.directory), worktree.join("app"));
    assert_eq!(Path::new(&view.folder), app.canonicalize().expect("canonical"));
    assert_eq!((view.mode, view.branch.as_deref()), (Mode::Worktree, Some("task/strict-serde")));
    assert_eq!(git(&worktree, &["branch", "--show-current"]), "task/strict-serde");
    let again = task(&laptop, &make(Mode::Worktree).await.expect("task.create").task).await;
    assert_eq!(Path::new(&again.directory), worktree.with_file_name("strict-serde-2").join("app"));
    assert_eq!(again.branch.as_deref(), Some("task/strict-serde-2"));
    let outside = laptop.call(&task_create(&workspace, dir.path(), Mode::Copy, "none")).await;
    assert!(matches!(refused(outside), Error::InvalidArgs { .. }));

    std::fs::remove_dir_all(&worktree).expect("delete the worktree by hand");
    laptop.call(&TermOpen { task: id.clone(), path: ".".into() }).await.expect("term.open brings it back");
    assert_eq!(git(&worktree.join("app"), &["branch", "--show-current"]), "task/strict-serde");

    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Completed }).await.expect("complete");
    let ended_at = task(&laptop, &id).await.ended_at.expect("ended_at");
    let early = running.server.remove_expired_worktrees(ended_at + WORKTREE_LIFETIME_MS - 1).expect("sweep");
    assert!(early.removed.is_empty() && early.failed.is_empty());
    assert!(worktree.exists());
    let due = running.server.remove_expired_worktrees(ended_at + WORKTREE_LIFETIME_MS).expect("sweep");
    assert_eq!((due.removed, due.failed.len()), (vec![id.clone()], 0));
    assert!(!worktree.exists());
    assert_eq!(git(&repo, &["branch", "--list", "task/strict-serde"]), "task/strict-serde");
    let other = dir.path().join("other");
    std::fs::create_dir(&other).expect("mkdir other");
    let other = self::repo(&other);
    let elsewhere = laptop.call(&task_create(&workspace, &other, Mode::Worktree, "none")).await.expect("task.create");
    let elsewhere = task(&laptop, &elsewhere.task).await;
    assert_eq!(Path::new(&elsewhere.directory), worktree.with_file_name("strict-serde-3"));

    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Pinned }).await.expect("reopen");
    assert_eq!(git(&worktree.join("app"), &["branch", "--show-current"]), "task/strict-serde");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_shows_its_repo_branch_and_folders_locking_the_guarded_and_an_in_place_task_needs_its_folder() {
    let (dir, running) = server().await;
    let repo = repo(dir.path());
    std::fs::create_dir(repo.join("src")).expect("mkdir src");
    std::fs::write(repo.join("README.md"), "").expect("write a file");
    let laptop = connect(&running.socket, "laptop").await;
    let get = |folder: &Path| FolderGet { folder: folder.to_string_lossy().into_owned() };

    let shown = laptop.call(&get(&repo.join("src/.."))).await.expect("folder.get");
    let canonical = repo.canonicalize().expect("canonical").to_string_lossy().into_owned();
    assert_eq!(
        shown,
        Folder {
            folder: canonical.clone(),
            repo: Some(canonical),
            branch: Some("main".into()),
            inside: Inside::Listed { folders: folders(&[(".git", false), ("src", false)]) },
        }
    );
    let shelf = dir.path().join("shelf");
    let locked = shelf.join("locked");
    std::fs::create_dir_all(shelf.join("open")).expect("mkdir open");
    std::fs::create_dir(&locked).expect("mkdir locked");
    std::os::unix::fs::symlink(&locked, shelf.join("link")).expect("symlink link");
    std::os::unix::fs::symlink("link", shelf.join("chain")).expect("symlink chain");
    std::os::unix::fs::symlink("loop", shelf.join("loop")).expect("symlink loop");
    std::os::unix::fs::symlink("../shelf/./open", shelf.join("up")).expect("symlink up");
    std::os::unix::fs::symlink(repo.join("README.md"), shelf.join("file")).expect("symlink file");
    std::fs::set_permissions(&locked, std::os::unix::fs::PermissionsExt::from_mode(0o000)).expect("chmod locked");
    let outside = laptop.call(&get(&shelf)).await.expect("folder.get");
    let kept_out = laptop.call(&get(&locked)).await.expect("folder.get");
    std::fs::set_permissions(&locked, std::os::unix::fs::PermissionsExt::from_mode(0o755)).expect("chmod locked");
    let listed = Inside::Listed {
        folders: folders(&[("chain", true), ("link", true), ("locked", true), ("open", false), ("up", false)]),
    };
    assert_eq!((outside.repo, outside.branch, outside.inside), (None, None, listed));
    assert!(matches!(kept_out.inside, Inside::KeptOut { setting: None, .. }), "{kept_out:?}");
    for folder in [Path::new("repo"), &repo.join("README.md")] {
        assert!(matches!(refused(laptop.call(&get(folder)).await), Error::InvalidArgs { .. }));
    }

    let gone = dir.path().join("gone");
    std::fs::create_dir(&gone).expect("mkdir gone");
    let workspace = workspace(&laptop, dir.path()).await;
    let id = laptop.call(&task_create(&workspace, &gone, Mode::InPlace, "none")).await.expect("task.create").task;
    std::fs::remove_dir(&gone).expect("rmdir gone");
    let opened = laptop.call(&TermOpen { task: id.clone(), path: ".".into() }).await;
    assert!(matches!(refused(opened), Error::NotFound { .. }));
    let focused = laptop.call(&TaskFocus { task: id.clone(), args: vec![] }).await;
    assert!(matches!(refused(focused), Error::NotFound { .. }));
    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Completed }).await.expect("complete");
    let reopened = laptop.call(&TaskMove { task: id, state: TaskState::Pinned }).await;
    assert!(matches!(refused(reopened), Error::NotFound { .. }));
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

    let id = create(&laptop, &workspace, Mode::Copy, "none").await;
    let view = task(&laptop, &id).await;
    let copy = PathBuf::from(&view.directory);
    assert_eq!((view.mode, view.branch.as_deref()), (Mode::Copy, Some("task/strict-serde")));
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
    laptop.call(&add("same root", &repo)).await.expect("a second workspace at the same root");

    let id = create(&laptop, &doomed, Mode::Worktree, "none").await;
    laptop.call(&TaskRename { task: id.clone(), title: "Renamed".into() }).await.expect("task.rename");
    let view = task(&laptop, &id).await;
    assert_eq!(
        (view.title.as_str(), view.mode, view.branch.as_deref()),
        ("Renamed", Mode::Worktree, Some("task/strict-serde"))
    );
    let child = TaskCreate { parent: Some(id.clone()), ..task_create(&other, &repo, Mode::InPlace, "none") };
    assert!(matches!(refused(laptop.call(&child).await), Error::Conflict { .. }));

    let remove = WorkspaceRemove { workspace: doomed.clone() };
    assert!(matches!(refused(laptop.call(&remove).await), Error::Conflict { .. }));
    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Cancelled }).await.expect("cancel");
    laptop.call(&remove).await.expect("workspace.remove");
    let snapshot = laptop.call(&SessionSync { since_rev: None }).await.expect("sync");
    assert!(snapshot.workspaces.iter().all(|workspace| workspace.id != doomed));
    assert!(snapshot.tasks.iter().all(|task| task.id != id));
    assert!(!Path::new(&view.directory).exists());
    assert_eq!(git(&repo, &["branch", "--list", "task/strict-serde"]), "task/strict-serde");
}

#[tokio::test(flavor = "multi_thread")]
async fn only_an_ended_task_archives_and_a_move_unarchives_it() {
    let (dir, running) = server().await;
    let laptop = connect(&running.socket, "laptop").await;
    let id = create(&laptop, &workspace(&laptop, dir.path()).await, Mode::InPlace, "none").await;
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
async fn a_task_nests_in_its_workspace_outside_its_own_subtree_and_refiles_with_its_subtasks() {
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
            TaskCreate { parent: parent.cloned(), state, ..task_create(workspace, dir.path(), Mode::InPlace, "none") };
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
    let before = task(&laptop, &child).await;
    assert_eq!(before.parent, None);

    let refile =
        |task: &TaskId, workspace: &WorkspaceId| TaskSetWorkspace { task: task.clone(), workspace: workspace.clone() };
    assert!(matches!(refused(laptop.call(&refile(&grandchild, &dotfiles)).await), Error::Conflict { .. }));
    let nowhere = WorkspaceId("wsp_missing".into());
    assert!(matches!(refused(laptop.call(&refile(&child, &nowhere)).await), Error::NotFound { .. }));
    let rev = laptop.call(&refile(&child, &dotfiles)).await.expect("task.set_workspace").rev;
    let (moved, below) = (task(&laptop, &child).await, task(&laptop, &grandchild).await);
    assert_eq!((&moved.workspace, &moved.folder, &moved.directory), (&dotfiles, &before.folder, &before.directory));
    assert_eq!((&below.workspace, below.parent.as_ref(), below.rev), (&dotfiles, Some(&child), rev));
    assert_eq!(task(&laptop, &root).await.workspace, octopus);
}
