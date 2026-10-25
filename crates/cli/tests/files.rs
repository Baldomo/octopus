mod common;

use std::time::Duration;

use common::{connect, create, start, workspace};
use octopus_client::CallError;
use octopus_proto::ops::{Entry, EntryKind, FileList, FileSearch, FileWrite, Match, TaskFocus};
use octopus_proto::types::{Checkout, Stream};
use octopus_proto::{Error, Event};
use tokio::io::AsyncReadExt;

#[tokio::test(flavor = "multi_thread")]
async fn a_client_works_on_a_tasks_files_through_its_server() {
    let dir = tempfile::tempdir().expect("tempdir");
    let checkout = dir.path().join("repo");
    std::fs::create_dir(&checkout).expect("mkdir");
    let running = start(dir.path(), "server.sock", "host.sock");
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, &checkout).await;
    let task = create(&laptop, &workspace, Checkout::Root, "none").await;
    let mut pushes = laptop.pushes();
    laptop.call(&TaskFocus { task: task.clone() }).await.expect("focus");

    let write = FileWrite { task: task.clone(), path: "src/main.rs".into(), content: "fn main() {}\n".into() };
    assert_eq!(laptop.call(&write).await.expect("file.write").bytes, 13);
    let main = Stream::File { task: task.clone(), path: "src/main.rs".into() };
    let mut bytes = Vec::new();
    laptop.open_data_stream(&main).await.expect("open").read_to_end(&mut bytes).await.expect("read");
    assert_eq!(bytes, b"fn main() {}\n");
    let absent = Stream::File { task: task.clone(), path: "absent.rs".into() };
    assert!(matches!(laptop.open_data_stream(&absent).await, Err(CallError::Server(Error::NotFound { .. }))));

    let entries = laptop.call(&FileList { task: task.clone(), path: ".".into() }).await.expect("file.list").entries;
    assert_eq!(entries, vec![Entry { name: "src".into(), kind: EntryKind::Directory }]);
    let matches =
        laptop.call(&FileSearch { task: task.clone(), pattern: "main".into() }).await.expect("search").matches;
    assert_eq!(matches, vec![Match { path: "src/main.rs".into(), line: 1, text: "fn main() {}".into() }]);

    std::fs::write(checkout.join("notes.md"), "by the agent").expect("external write");
    let pushed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Event::FileChanged { task, path } = pushes.recv().await.expect("push").event
                && path == "notes.md"
            {
                return task;
            }
        }
    })
    .await;
    assert_eq!(pushed.expect("no file.changed for notes.md"), task);

    match laptop.call(&FileList { task, path: "../".into() }).await {
        Err(CallError::Server(Error::InvalidArgs { .. })) => {}
        other => panic!("a path outside the checkout must fail, got {other:?}"),
    }
}
