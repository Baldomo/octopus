mod common;

use std::time::Duration;

use common::{connect, create, focus, refused, server, workspace};
use octopus_proto::ops::{
    Entry, EntryKind, FileCopy, FileCreate, FileDelete, FileList, FileMove, FileSearch, FileWrite, Match,
};
use octopus_proto::types::{CheckoutKind, ReadDir, Stream};
use octopus_proto::{Error, Event, Outcome};
use tokio::io::AsyncWriteExt;

#[tokio::test(flavor = "multi_thread")]
async fn a_client_works_on_a_tasks_files_through_its_server() {
    let (dir, running) = server().await;
    let checkout = dir.path().join("repo");
    std::fs::create_dir(&checkout).expect("mkdir");
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, &checkout).await;
    let task = create(&laptop, &workspace, CheckoutKind::Root, "none").await;
    let mut pushes = laptop.pushes();
    focus(&laptop, &task).await;

    let write = FileWrite { task: task.clone(), path: "src/main.rs".into(), content: "fn main() {}\n".into() };
    assert_eq!(laptop.call(&write).await.expect("file.write").bytes, 13);
    let read = |path, start, len| common::read(&laptop, ReadDir::Checkout(task.clone()), path, start, len);
    assert_eq!(read("src/main.rs", 0, Stream::TO_END).await.expect("read"), (13, b"fn main() {}\n".to_vec()));
    assert_eq!(read("src/main.rs", 3, 4).await.expect("a range"), (13, b"main".to_vec()));
    assert_eq!(read("src/main.rs", 13, 4).await.expect("a range at the end"), (13, vec![]));
    assert!(matches!(refused(read("absent.rs", 0, 1).await), Error::NotFound { .. }));
    assert!(matches!(refused(read("src", 0, 1).await), Error::NotFound { .. }));

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

    let outside = laptop.call(&FileList { task, path: "../".into() }).await;
    assert!(matches!(refused(outside), Error::InvalidArgs { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_rearranges_a_checkout_without_reaching_past_it() {
    let (dir, running) = server().await;
    let checkout = dir.path().join("repo");
    std::fs::create_dir(&checkout).expect("mkdir");
    std::fs::create_dir(dir.path().join("secret")).expect("mkdir");
    std::os::unix::fs::symlink(dir.path().join("secret"), checkout.join("out")).expect("symlink");
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, &checkout).await;
    let task = create(&laptop, &workspace, CheckoutKind::Root, "none").await;
    let (create, rename, copy) = (
        |path: &str, kind| FileCreate { task: task.clone(), path: path.into(), kind },
        |from: &str, to: &str| FileMove { task: task.clone(), from: from.into(), to: to.into() },
        |from: &str, to: &str| FileCopy { task: task.clone(), from: from.into(), to: to.into() },
    );

    laptop.call(&create("src/lib.rs", EntryKind::File)).await.expect("file.create a file");
    laptop.call(&create("docs", EntryKind::Directory)).await.expect("file.create a directory");
    let taken = laptop.call(&create("src/lib.rs", EntryKind::File)).await;
    assert!(matches!(refused(taken), Error::Conflict { .. }));
    laptop.call(&rename("src/lib.rs", "docs/lib.rs")).await.expect("file.move");
    laptop.call(&copy("docs", "docs copy")).await.expect("file.copy a directory");
    assert!(checkout.join("docs copy/lib.rs").is_file() && checkout.join("docs/lib.rs").is_file());
    assert!(matches!(refused(laptop.call(&rename("docs copy", "docs")).await), Error::Conflict { .. }));
    assert!(matches!(refused(laptop.call(&rename("docs", "docs/deeper")).await), Error::InvalidArgs { .. }));
    let gone = FileDelete { task: task.clone(), path: "docs copy".into(), trash: false };
    laptop.call(&gone).await.expect("file.delete");
    assert!(!checkout.join("docs copy").exists());

    laptop.upload(task.clone(), "logo.png".into(), &[0, 159, 146, 150]).await.expect("upload");
    assert_eq!(std::fs::read(checkout.join("logo.png")).expect("uploaded"), [0u8, 159, 146, 150]);
    let again = laptop.upload(task.clone(), "logo.png".into(), b"x").await;
    assert!(matches!(refused(again), Error::Conflict { .. }));
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
        assert!(matches!(refused(escape), Error::InvalidArgs { .. }));
    }
    assert_eq!(std::fs::read_dir(dir.path().join("secret")).expect("secret").count(), 0);
}
