mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::{
    Running, SERVER, connect, create, echo_agent, eventually, holder_name, identity, read_until, start, task, workspace,
};
use octopus_client::servers::{self, Address};
use octopus_client::tunnel::Tunnel;
use octopus_client::{CallError, Connection, DataStream, Identity};
use octopus_proto::device::DeviceKey;
use octopus_proto::ops::{
    BrowserClose, BrowserMove, BrowserOpen, BrowserReport, HarnessSet, PairingForget, PairingList, PairingOffer,
    TaskFocus, TaskGet, TermClaim, TermSend,
};
use octopus_proto::pairing::Invite;
use octopus_proto::types::{BrowserId, BrowserPanel, BrowserTarget, CheckoutKind, Hooks, Stream, Task, TaskId};
use octopus_proto::{Error, Event};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn offer(running: &Running) -> Invite {
    connect(running, "owner").await.call(&PairingOffer {}).await.expect("pairing.offer").invite
}

/// The server as a client reaches it through its relay alone.
fn remote(running: &Running) -> Address {
    Address::Remote { key: running.server.device().public(), relay: running.server.relay().clone() }
}

async fn paired(running: &Running, me: &Identity) -> Connection {
    let listed = servers::pair(&offer(running).await, me).await.expect("pair");
    assert_eq!(listed, servers::Server { name: SERVER.into(), address: remote(running) });
    Connection::connect(&listed.address, me).await.expect("connect over iroh")
}

fn twin(of: &Identity, name: &str) -> Identity {
    Identity::new(name.into(), DeviceKey::from_pkcs8(of.key.pkcs8()).expect("same key"))
}

#[tokio::test(flavor = "multi_thread")]
async fn only_a_client_that_spent_a_code_reaches_a_remote_server() {
    let dir = tempfile::tempdir().expect("tempdir");
    let running = start(dir.path(), "server.sock", "host.sock").await;
    let laptop = identity("laptop");

    let closed = Connection::connect(&remote(&running), &laptop).await.err().expect("no code is on offer");
    assert!(format!("{closed:#}").contains("no pairing code is on offer"), "{closed:#}");

    let invite = offer(&running).await;
    let refused = Connection::connect(&remote(&running), &laptop).await.err().expect("an unpaired client is refused");
    assert!(format!("{refused:#}").contains("not paired"), "{refused:#}");
    servers::pair(&invite, &laptop).await.expect("pair");
    let spent = servers::pair(&invite, &identity("desk")).await.expect_err("spent");
    assert!(format!("{spent:#}").contains("no pairing code is on offer"), "{spent:#}");
    let (fresh, namesake) = (offer(&running).await, identity("laptop"));
    let refused = servers::pair(&fresh, &namesake).await;
    assert!(format!("{:#}", refused.expect_err("a namesake")).contains("paired to another device"));
    connect(&running, "owner").await.call(&PairingForget { client: "laptop".into() }).await.expect("forget");
    servers::pair(&fresh, &namesake).await.expect("the refusal left the code unspent");
    let laptop = namesake;

    let connection = Connection::connect(&remote(&running), &laptop).await.expect("a paired client is accepted");
    eventually("a heartbeat round trip", async || connection.rtt().is_some()).await;
    let listed = connection.call(&PairingList {}).await.expect("pairing.list").clients;
    let [client] = listed.as_slice() else { panic!("one paired client, got {listed:?}") };
    assert!(client.name == "laptop" && client.connected && client.seen_at.is_some(), "{client:?}");
    assert_eq!(client.key, laptop.key.public(), "the list carries the key the client is pinned by");
    let unknown = connection.call(&PairingForget { client: "desk".into() }).await;
    assert!(matches!(unknown, Err(CallError::Server(Error::NotFound { .. }))), "{unknown:?}");
    let desk = twin(&laptop, "desk");
    let renamed = Connection::connect(&remote(&running), &desk).await.expect("a renamed client");
    let listed = renamed.call(&PairingList {}).await.expect("pairing.list").clients;
    let [client] = listed.as_slice() else { panic!("one paired client, got {listed:?}") };
    assert_eq!(client.name, "desk", "a paired client goes by the name its hello gives");
    let seen_at = client.seen_at;
    tokio::time::sleep(Duration::from_millis(5)).await;
    drop(renamed);
    let list = || async { connection.call(&PairingList {}).await.expect("pairing.list").clients.remove(0) };
    eventually("the renamed connection closes", async || list().await.seen_at > seen_at).await;
    assert!(list().await.connected, "the device stays connected while its connection under the old name is open");
    servers::pair(&offer(&running).await, &identity("phone")).await.expect("pair a phone");
    let namesake = Connection::connect(&remote(&running), &twin(&laptop, "phone")).await.err().expect("a namesake");
    assert!(format!("{namesake:#}").contains("paired to another device"), "{namesake:#}");

    let second_process = Connection::connect(&remote(&running), &twin(&laptop, "laptop")).await.expect("a second");
    for open in [&connection, &second_process] {
        open.call(&PairingList {}).await.expect("both processes of one device stay connected");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_runs_over_iroh_and_a_stalled_file_transfer_never_blocks_its_terminal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let running = start(dir.path(), "server.sock", "host.sock").await;
    let laptop = paired(&running, &identity("laptop")).await;
    let workspace = workspace(&laptop, dir.path()).await;
    let harness = HarnessSet {
        id: "echo".into(),
        argv: echo_agent("agent:up"),
        install: "true".into(),
        start: vec![],
        resume: vec![],
        hooks: Hooks::None,
    };
    laptop.call(&harness).await.expect("harness.set");
    let id = create(&laptop, &workspace, CheckoutKind::Root, "echo").await;
    laptop.call(&TaskFocus { task: id.clone(), args: vec![] }).await.expect("task.focus");
    let agent = Stream::Pty(task(&laptop, &id).await.panels[0].id.clone());

    let large = vec![b'x'; 8 << 20];
    std::fs::write(dir.path().join("large.bin"), &large).expect("write a large file");
    let mut transfer = laptop.open_data_stream(&Stream::File { task: id.clone(), path: "large.bin".into() }).await;
    let mut terminal = laptop.open_data_stream(&agent).await.expect("attach");
    read_until(&mut terminal, "agent:up").await;
    let send = TermSend { task: id, panel: "agent".into(), text: "over-iroh".into(), enter: true, take_lock: true };
    laptop.call(&send).await.expect("term.send");
    read_until(&mut terminal, "over-iroh").await;

    let mut received = Vec::new();
    transfer.as_mut().expect("open the transfer").read_to_end(&mut received).await.expect("read the transfer");
    assert_eq!(received.len(), large.len());
}

/// A task whose agent terminal `holder` shows and holds the lock of.
async fn held_terminal(dir: &Path, holder: &Connection) -> (TaskId, DataStream) {
    let workspace = workspace(holder, dir).await;
    let harness = HarnessSet {
        id: "echo".into(),
        argv: echo_agent("agent:up"),
        install: "true".into(),
        start: vec![],
        resume: vec![],
        hooks: Hooks::None,
    };
    holder.call(&harness).await.expect("harness.set");
    let id = create(holder, &workspace, CheckoutKind::Root, "echo").await;
    holder.call(&TaskFocus { task: id.clone(), args: vec![] }).await.expect("task.focus");
    let agent = Stream::Pty(task(holder, &id).await.panels[0].id.clone());
    let mut shown = holder.open_data_stream(&agent).await.expect("attach");
    read_until(&mut shown, "agent:up").await;
    holder.call(&send(&id, true)).await.expect("take the lock");
    (id, shown)
}

fn send(task: &TaskId, take_lock: bool) -> TermSend {
    TermSend { task: task.clone(), panel: "agent".into(), text: "x".into(), enter: true, take_lock }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_lock_follows_its_holders_key_and_not_a_name_it_gave_up() {
    let dir = tempfile::tempdir().expect("tempdir");
    let running = start(dir.path(), "server.sock", "host.sock").await;
    let (laptop, phone) = (identity("laptop"), identity("phone"));
    let holder = paired(&running, &laptop).await;
    paired(&running, &phone).await;
    let (id, _shown) = held_terminal(dir.path(), &holder).await;

    let renamed = Connection::connect(&remote(&running), &twin(&laptop, "desk")).await.expect("rename the laptop");
    let panel = task(&renamed, &id).await.panels.remove(0);
    assert_eq!(holder_name(&panel), Some("desk"), "the lock is labelled with its holder's new name");
    let impostor = Connection::connect(&remote(&running), &twin(&phone, "laptop")).await.expect("a freed name");
    let written = impostor.call(&send(&id, false)).await;
    assert!(matches!(&written, Err(CallError::Server(Error::PtyLocked { holder })) if holder == "desk"), "{written:?}");
    holder.call(&send(&id, false)).await.expect("the holder still writes under its old name");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_claim_under_a_connections_old_name_keeps_the_lock_labelled_with_the_new_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let running = start(dir.path(), "server.sock", "host.sock").await;
    let laptop = identity("laptop");
    let holder = paired(&running, &laptop).await;
    let (id, _shown) = held_terminal(dir.path(), &holder).await;
    let _renamed = Connection::connect(&remote(&running), &twin(&laptop, "desk")).await.expect("rename the laptop");

    let panel = task(&holder, &id).await.panels.remove(0);
    let claim = TermClaim { task: id.clone(), panel: "agent".into(), cols: panel.cols, rows: panel.rows };
    holder.call(&claim).await.expect("claim under the old name");
    holder.call(&send(&id, true)).await.expect("take under the old name");
    assert_eq!(holder_name(&task(&holder, &id).await.panels[0]), Some("desk"));
}

#[tokio::test(flavor = "multi_thread")]
async fn forgetting_a_device_closes_its_connections_and_frees_every_lock_it_holds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let running = start(dir.path(), "server.sock", "host.sock").await;
    let holder = paired(&running, &identity("laptop")).await;
    let (id, _shown) = held_terminal(dir.path(), &holder).await;

    let owner = connect(&running, "owner").await;
    owner.call(&PairingForget { client: "laptop".into() }).await.expect("forget the laptop");
    assert_eq!(holder_name(&task(&owner, &id).await.panels[0]), None);
    let closed = tokio::time::timeout(Duration::from_secs(5), holder.closed()).await.expect("the connection closes");
    assert!(closed.contains("forgotten"), "{closed}");
    let claim = TermClaim { task: id.clone(), panel: "agent".into(), cols: 80, rows: 24 };
    assert!(holder.call(&claim).await.is_err(), "a forgotten device claims nothing");
    assert_eq!(holder_name(&task(&owner, &id).await.panels[0]), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_local_client_and_a_paired_one_of_one_name_each_hold_only_their_own_lock() {
    let dir = tempfile::tempdir().expect("tempdir");
    let running = start(dir.path(), "server.sock", "host.sock").await;
    let (local_me, paired_me) = (identity("laptop"), identity("laptop"));
    let local = connect(&running, "laptop").await;
    let paired = paired(&running, &paired_me).await;
    let (id, _shown) = held_terminal(dir.path(), &local).await;
    let (on_socket, over_iroh) = (Address::Local { socket: running.socket.clone() }, remote(&running));

    let local_lock = task(&local, &id).await.panels.remove(0).lock_holder.expect("the local client holds it");
    assert!(local_me.holds(&local_lock, &on_socket) && !paired_me.holds(&local_lock, &over_iroh));
    paired.call(&send(&id, true)).await.expect("the paired client takes the lock");
    let paired_lock = task(&local, &id).await.panels.remove(0).lock_holder.expect("the paired client holds it");
    assert_ne!(local_lock, paired_lock, "two clients of one name hold two different locks");
    assert!(paired_me.holds(&paired_lock, &over_iroh) && !local_me.holds(&paired_lock, &on_socket));
}

fn open(task: &TaskId, url: &str, panel: BrowserTarget) -> BrowserOpen {
    BrowserOpen { task: task.clone(), url: url.into(), panel }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_browser_panel_is_task_state_and_its_localhost_page_tunnels_to_the_servers_port() {
    let dir = tempfile::tempdir().expect("tempdir");
    let running = start(dir.path(), "server.sock", "host.sock").await;
    let laptop = Arc::new(paired(&running, &identity("laptop")).await);
    let desk = connect(&running, "desk").await;
    let mut pushes = desk.pushes();
    let workspace = workspace(&desk, dir.path()).await;
    let id = create(&desk, &workspace, CheckoutKind::Root, "none").await;

    let shown = laptop.call(&open(&id, "http://localhost:5173/", BrowserTarget::New)).await.expect("browser.open");
    let settings = "http://localhost:5173/settings";
    let again = laptop.call(&open(&id, settings, BrowserTarget::Panel(shown.panel.clone()))).await.expect("again");
    assert_eq!(again.panel, shown.panel);
    let reported =
        BrowserReport { task: id.clone(), panel: shown.panel.clone(), url: settings.into(), title: "S".into() };
    let titled = laptop.call(&reported).await.expect("browser.report");
    assert_eq!(
        laptop.call(&reported).await.expect("the same report").rev,
        titled.rev,
        "an idle report revises nothing"
    );
    let expected = vec![BrowserPanel { panel: shown.panel, url: settings.into(), title: "S".into(), load: 2 }];
    let pushed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Event::TaskUpdated { task } = pushes.recv().await.expect("push").event
                && task.browsers == expected
            {
                return;
            }
        }
    });
    assert!(pushed.await.is_ok(), "desk never saw the browser panel");
    for (url, panel) in [
        ("javascript:alert(1)", BrowserTarget::New),
        ("file://mac-mini/etc/hosts", BrowserTarget::New),
        (settings, BrowserTarget::Panel(BrowserId::random())),
    ] {
        match laptop.call(&open(&id, url, panel)).await {
            Err(CallError::Server(Error::InvalidArgs { .. } | Error::NotFound { .. })) => {}
            other => panic!("{url} must be refused, got {other:?}"),
        }
    }

    let site = dir.path().join("site");
    std::fs::create_dir_all(site.join("css")).expect("make the site");
    std::fs::write(site.join("css/style.css"), "h1{}").expect("write the style");
    std::fs::write(dir.path().join("secret"), "key").expect("write a secret beside the site");
    std::os::unix::fs::symlink(dir.path().join("secret"), site.join("leak")).expect("link out of the site");
    let file = laptop.call(&open(&id, &format!("file://{}/index.html", site.display()), BrowserTarget::New)).await;
    let read = |path: &str| {
        let stream = Stream::Page { root: site.display().to_string(), path: path.into() };
        let laptop = Arc::clone(&laptop);
        async move {
            let mut bytes = Vec::new();
            laptop.open_data_stream(&stream).await?.read_to_end(&mut bytes).await.expect("read the file");
            Ok::<_, CallError>(bytes)
        }
    };
    assert_eq!(read("css/style.css").await.expect("a file of the page's root"), b"h1{}");
    let secret = dir.path().join("secret").display().to_string();
    for outside in ["../secret", "css/../../secret", "leak", secret.as_str()] {
        let refused = read(outside).await;
        assert!(matches!(refused, Err(CallError::Server(Error::InvalidArgs { .. }))), "{outside}: {refused:?}");
    }
    let missing = read("css").await;
    assert!(matches!(missing, Err(CallError::Server(Error::NotFound { .. }))), "a directory is no page");
    let file = file.expect("a file page").panel;
    let order = |task: Task| task.browsers.into_iter().map(|panel| panel.panel).collect::<Vec<_>>();
    let moved = laptop.call(&BrowserMove { task: id.clone(), panel: file.clone(), to: 0 }).await.expect("browser.move");
    let task = laptop.call(&TaskGet { task: id.clone() }).await.expect("task.get");
    assert_eq!(order(task), [file.clone(), expected[0].panel.clone()], "a moved browser panel takes its index");
    let still = laptop.call(&BrowserMove { task: id.clone(), panel: file.clone(), to: 0 }).await.expect("idle move");
    assert_eq!(still.rev, moved.rev, "a move to the index the panel holds revises nothing");
    let past = laptop.call(&BrowserMove { task: id.clone(), panel: file.clone(), to: 2 }).await;
    assert!(matches!(past, Err(CallError::Server(Error::InvalidArgs { .. }))), "{past:?}");
    let behind = laptop.call(&open(&id, settings, BrowserTarget::Background)).await.expect("a background panel");
    let task = laptop.call(&TaskGet { task: id.clone() }).await.expect("task.get");
    let last = task.browsers.last().expect("the background panel");
    assert_eq!((&last.panel, last.load), (&behind.panel, 0), "a background panel joins the end, never shown");
    laptop.call(&BrowserClose { task: id.clone(), panel: behind.panel }).await.expect("close the background panel");
    let closed = BrowserClose { task: id.clone(), panel: file };
    laptop.call(&closed).await.expect("browser.close");
    let task = laptop.call(&TaskGet { task: id.clone() }).await.expect("task.get");
    assert_eq!(task.browsers, expected, "a closed browser panel is gone");

    let dev_server = TcpListener::bind("127.0.0.1:0").await.expect("bind the dev server");
    let port = dev_server.local_addr().expect("its port").port();
    tokio::spawn(async move {
        let (mut socket, _) = dev_server.accept().await.expect("accept");
        let mut request = Vec::new();
        socket.read_to_end(&mut request).await.expect("read the request");
        socket.write_all(&[b"echo:".as_slice(), &request].concat()).await.expect("reply");
    });
    let client_side = TcpListener::bind("127.0.0.1:0").await.expect("bind the client side");
    let browser_port = client_side.local_addr().expect("its port");
    let _tunnel = Tunnel::new(port, vec![client_side], Arc::new(move || Ok(Arc::clone(&laptop))));
    let mut browser = TcpStream::connect(browser_port).await.expect("connect through the tunnel");
    browser.write_all(b"GET / HTTP/1.1").await.expect("send");
    browser.shutdown().await.expect("end the request");
    let mut reply = Vec::new();
    browser.read_to_end(&mut reply).await.expect("read the reply");
    assert_eq!(reply, b"echo:GET / HTTP/1.1");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_connection_whose_device_proof_fails_is_closed_unread() {
    let dir = tempfile::tempdir().expect("tempdir");
    let running = start(dir.path(), "server.sock", "host.sock").await;
    let relay = running.server.relay().clone();
    let forger = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .relay_mode(iroh::RelayMode::Custom(iroh::RelayMap::from(relay.clone())))
        .bind()
        .await
        .expect("bind");
    let server = running.server.device().public().endpoint();
    let connection = forger.connect(iroh::EndpointAddr::new(server).with_relay_url(relay), b"octopus/1").await;
    let connection = connection.expect("the handshake completes");
    let mut proof = connection.open_uni().await.expect("open the proof stream");
    let unsigned = [identity("laptop").key.public().as_bytes().as_slice(), &[0u8; 64]].concat();
    proof.write_all(&unsigned).await.expect("send the proof");
    proof.finish().expect("finish the proof");
    let closed = tokio::time::timeout(Duration::from_secs(5), connection.closed()).await.expect("closed");
    assert!(closed.to_string().contains("no valid device proof"), "{closed}");
}
