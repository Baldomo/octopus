mod common;

use std::time::Duration;

use common::{connect, create, start, workspace};
use octopus_client::CallError;
use octopus_proto::ops::{
    Entry, EntryKind, FileCopy, FileCreate, FileDelete, FileList, FileMove, FileSearch, FileWrite, Match, TaskFocus,
};
use octopus_proto::types::{Checkout, Stream};
use octopus_proto::{Error, Event, Outcome};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test(flavor = "multi_thread")]
async fn a_client_works_on_a_tasks_files_through_its_server() {
    let dir = tempfile::tempdir().expect("tempdir");
    let checkout = dir.path().join("repo");
    std::fs::create_dir(&checkout).expect("mkdir");
    let running = start(dir.path(), "server.sock", "host.sock").await;
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, &checkout).await;
    let task = create(&laptop, &workspace, Checkout::Root, "none").await;
    let mut pushes = laptop.pushes();
    laptop.call(&TaskFocus { task: task.clone(), args: vec![] }).await.expect("focus");

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

#[tokio::test(flavor = "multi_thread")]
async fn a_client_rearranges_a_checkout_without_reaching_past_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let checkout = dir.path().join("repo");
    std::fs::create_dir(&checkout).expect("mkdir");
    std::fs::create_dir(dir.path().join("secret")).expect("mkdir");
    std::os::unix::fs::symlink(dir.path().join("secret"), checkout.join("out")).expect("symlink");
    let running = start(dir.path(), "server.sock", "host.sock").await;
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, &checkout).await;
    let task = create(&laptop, &workspace, Checkout::Root, "none").await;
    let (create, rename, copy) = (
        |path: &str, kind| FileCreate { task: task.clone(), path: path.into(), kind },
        |from: &str, to: &str| FileMove { task: task.clone(), from: from.into(), to: to.into() },
        |from: &str, to: &str| FileCopy { task: task.clone(), from: from.into(), to: to.into() },
    );

    laptop.call(&create("src/lib.rs", EntryKind::File)).await.expect("file.create a file");
    laptop.call(&create("docs", EntryKind::Directory)).await.expect("file.create a directory");
    let taken = laptop.call(&create("src/lib.rs", EntryKind::File)).await;
    assert!(matches!(taken, Err(CallError::Server(Error::Conflict { .. }))), "{taken:?}");
    laptop.call(&rename("src/lib.rs", "docs/lib.rs")).await.expect("file.move");
    laptop.call(&copy("docs", "docs copy")).await.expect("file.copy a directory");
    assert!(checkout.join("docs copy/lib.rs").is_file() && checkout.join("docs/lib.rs").is_file());
    let onto = laptop.call(&rename("docs copy", "docs")).await;
    assert!(matches!(onto, Err(CallError::Server(Error::Conflict { .. }))), "{onto:?}");
    let inside = laptop.call(&rename("docs", "docs/deeper")).await;
    assert!(matches!(inside, Err(CallError::Server(Error::InvalidArgs { .. }))), "{inside:?}");
    let gone = FileDelete { task: task.clone(), path: "docs copy".into(), trash: false };
    laptop.call(&gone).await.expect("file.delete");
    assert!(!checkout.join("docs copy").exists());

    laptop.upload(task.clone(), "logo.png".into(), &[0, 159, 146, 150]).await.expect("upload");
    assert_eq!(std::fs::read(checkout.join("logo.png")).expect("uploaded"), [0u8, 159, 146, 150]);
    let again = laptop.upload(task.clone(), "logo.png".into(), b"x").await;
    assert!(matches!(again, Err(CallError::Server(Error::Conflict { .. }))), "{again:?}");
    let mut short = laptop
        .open_data_stream(&Stream::Upload { task: task.clone(), bytes: 10, path: "short".into() })
        .await
        .expect("open");
    short.write_all(b"abc").await.expect("send");
    short.shutdown().await.expect("finish");
    let landed = octopus_proto::frames::read_outcome(&mut short).await.expect("outcome");
    assert!(matches!(landed, Outcome::Err(Error::InvalidArgs { .. })), "{landed:?}");
    assert!(!checkout.join("short").exists());

    std::os::unix::fs::symlink(dir.path().join("secret/planted"), checkout.join("dangling")).expect("symlink");
    std::os::unix::fs::symlink(checkout.join("docs"), checkout.join("docs link")).expect("symlink");
    for escape in [
        laptop.call(&FileWrite { task: task.clone(), path: "dangling".into(), content: "x".into() }).await.map(drop),
        laptop.call(&copy("docs", "docs link/again")).await.map(drop),
        laptop.call(&create("out/planted", EntryKind::File)).await.map(drop),
        laptop.call(&FileList { task: task.clone(), path: "out".into() }).await.map(drop),
        laptop.call(&copy("logo.png", "out/logo.png")).await.map(drop),
        laptop.call(&FileDelete { task: task.clone(), path: ".".into(), trash: false }).await.map(drop),
    ] {
        assert!(matches!(escape, Err(CallError::Server(Error::InvalidArgs { .. }))), "{escape:?}");
    }
    assert_eq!(std::fs::read_dir(dir.path().join("secret")).expect("secret").count(), 0);
}
