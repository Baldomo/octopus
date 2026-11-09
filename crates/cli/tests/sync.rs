mod common;

use std::time::Duration;

use common::{connect, create, server, workspace};
use octopus_client::cache::{Cache, PushEffect};
use octopus_proto::ops::{SessionSync, TaskMove, WorkspaceRemove};
use octopus_proto::types::{CheckoutKind, TaskState};
use octopus_proto::{Event, Push};

#[tokio::test(flavor = "multi_thread")]
async fn clients_moving_tasks_at_once_converge_on_one_server_state() {
    let (dir, running) = server().await;
    let laptop = connect(&running.socket, "laptop").await;
    let desk = connect(&running.socket, "desk").await;
    let mut pushes = laptop.pushes();
    let mut cache = Cache::from_snapshot(laptop.call(&SessionSync { since_rev: None }).await.expect("sync"));

    let workspace = workspace(&desk, dir.path()).await;
    let task = create(&desk, &workspace, CheckoutKind::Root, "none").await;
    let moves = TaskState::ALL.into_iter().cycle().take(40).enumerate().map(|(index, state)| {
        let (writer, task) = (if index % 2 == 0 { &laptop } else { &desk }, task.clone());
        async move { writer.call(&TaskMove { task, state }).await }
    });
    let revs: Vec<u64> =
        futures::future::join_all(moves).await.into_iter().map(|moved| moved.expect("task.move").rev).collect();
    let last = *revs.iter().max().expect("revs");

    let caught_up = tokio::time::timeout(Duration::from_secs(5), async {
        while cache.rev < last {
            let push = pushes.recv().await.expect("push");
            assert_ne!(cache.apply(&push), PushEffect::Stale, "a push arrived after a newer one");
        }
    })
    .await;
    assert!(caught_up.is_ok(), "laptop's cache stalled at rev {}", cache.rev);
    let fresh = Cache::from_snapshot(desk.call(&SessionSync { since_rev: Some(cache.rev) }).await.expect("resync"));
    assert_eq!(cache, fresh);

    let stale = Push { rev: 1, event: Event::HarnessUpdated { harness: "h".into() } };
    assert_eq!(cache.apply(&stale), PushEffect::Stale);

    laptop.call(&TaskMove { task, state: TaskState::Completed }).await.expect("end the task");
    let removed = laptop.call(&WorkspaceRemove { workspace }).await.expect("workspace.remove").rev;
    let emptied = tokio::time::timeout(Duration::from_secs(5), async {
        while cache.rev < removed {
            cache.apply(&pushes.recv().await.expect("push"));
        }
    })
    .await;
    assert!(emptied.is_ok(), "laptop's cache stalled at rev {}", cache.rev);
    assert!(cache.workspaces.is_empty() && cache.tasks.is_empty(), "{cache:?}");
}
