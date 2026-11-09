mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::{
    Running, SERVER, agent, connect, create, echo_agent, eventually, focus, holder_name, identity, read_until,
    relayed_server, send, set_harness, task, workspace,
};
use octopus_client::servers::{self, Address, Servers};
use octopus_client::tunnel::Tunnel;
use octopus_client::{CallError, Connection, DataStream, Identity};
use octopus_proto::account::{Change, Link};
use octopus_proto::device::DeviceKey;
use octopus_proto::ops::{
    AccountSync, BrowserClose, BrowserMove, BrowserOpen, BrowserReport, PairedClient, PairingForget, PairingList,
    PairingOffer, TaskGet, TermClaim, TermSend,
};
use octopus_proto::pairing::Invite;
use octopus_proto::types::{
    BrowserId, BrowserPanel, BrowserTarget, CheckoutKind, Hooks, ReadDir, Stream, Task, TaskId,
};
use octopus_proto::{Error, Event};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The device `of` as the server lists it among its trusted accounts' devices.
async fn device(connection: &Connection, of: &Identity) -> PairedClient {
    let accounts = connection.call(&PairingList {}).await.expect("pairing.list").accounts;
    let mut devices = accounts.into_iter().flat_map(|account| account.devices);
    devices.find(|device| device.key == of.key.public()).expect("the server lists the device")
}

async fn offer(running: &Running) -> Invite {
    connect(&running.socket, "owner").await.call(&PairingOffer {}).await.expect("pairing.offer").invite
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

/// A link through `running` that admits one device to `of`'s account.
async fn link(running: &Running, of: &Identity, on: &Connection) -> Link {
    let (invite, secret, _) = of.account.update(|kept| Ok(kept.account.invite(&of.key))).expect("invite");
    on.sync_account(of).await.expect("sync the invite");
    let (server, relay) = (running.server.device().public(), running.server.relay().clone());
    Link { server, relay, account: of.account.account().id(), invite, secret }
}

fn forget(of: &Identity) -> PairingForget {
    PairingForget { account: of.account.account().id() }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_connection_outlives_the_client_forgetting_its_endpoint_and_the_next_dial_works() {
    let (_dir, running) = relayed_server().await;
    let phone = identity("phone", &DeviceKey::generate());
    let before = paired(&running, &phone).await;
    phone.forget_dialer().await;
    let after = Connection::connect(&remote(&running), &phone).await.expect("dial from a new endpoint");
    after.call(&PairingList {}).await.expect("the new connection works");
    before.call(&PairingList {}).await.expect("the open connection keeps its endpoint");
}

#[tokio::test(flavor = "multi_thread")]
async fn only_a_client_that_spent_a_code_reaches_a_remote_server() {
    let (_dir, running) = relayed_server().await;
    let laptop = identity("laptop", &DeviceKey::generate());

    let closed = Connection::connect(&remote(&running), &laptop).await.err().expect("no code is on offer");
    assert!(format!("{closed:#}").contains("no pairing code or link is on offer"), "{closed:#}");

    let invite = offer(&running).await;
    let refused = Connection::connect(&remote(&running), &laptop).await.err().expect("an unpaired client is refused");
    assert!(format!("{refused:#}").contains("not paired"), "{refused:#}");
    servers::pair(&invite, &laptop).await.expect("pair");
    let spent = servers::pair(&invite, &identity("desk", &DeviceKey::generate())).await.expect_err("spent");
    assert!(format!("{spent:#}").contains("no pairing code or link is on offer"), "{spent:#}");
    let (fresh, namesake) = (offer(&running).await, identity("laptop", &DeviceKey::generate()));
    let refused = servers::pair(&fresh, &namesake).await;
    assert!(format!("{:#}", refused.expect_err("a namesake")).contains("belongs to another device"));
    connect(&running.socket, "owner").await.call(&forget(&laptop)).await.expect("forget");
    servers::pair(&fresh, &namesake).await.expect("the refusal left the code unspent");
    let laptop = namesake;

    let connection = Connection::connect(&remote(&running), &laptop).await.expect("a paired client is accepted");
    eventually("a heartbeat round trip", async || connection.rtt().is_some()).await;
    let client = device(&connection, &laptop).await;
    assert!(client.name == "laptop" && client.connected && client.seen_at.is_some(), "{client:?}");
    assert_eq!(client.key, laptop.key.public(), "the list carries the key the client is pinned by");
    let unknown = connection.call(&forget(&identity("desk", &DeviceKey::generate()))).await;
    assert!(matches!(unknown, Err(CallError::Server(Error::NotFound { .. }))), "{unknown:?}");
    let desk = identity("desk", &laptop.key);
    let renamed = Connection::connect(&remote(&running), &desk).await.expect("a renamed client");
    let client = device(&renamed, &laptop).await;
    assert_eq!(client.name, "desk", "a paired client goes by the name its hello gives");
    let seen_at = client.seen_at;
    tokio::time::sleep(Duration::from_millis(5)).await;
    drop(renamed);
    let list = || async { device(&connection, &laptop).await };
    eventually("the renamed connection closes", async || list().await.seen_at > seen_at).await;
    assert!(list().await.connected, "the device stays connected while its connection under the old name is open");
    servers::pair(&offer(&running).await, &identity("phone", &DeviceKey::generate())).await.expect("pair a phone");
    let namesake =
        Connection::connect(&remote(&running), &identity("phone", &laptop.key)).await.err().expect("a namesake");
    assert!(format!("{namesake:#}").contains("belongs to another device"), "{namesake:#}");

    let second_process =
        Connection::connect(&remote(&running), &identity("laptop", &laptop.key)).await.expect("a second");
    for open in [&connection, &second_process] {
        open.call(&PairingList {}).await.expect("both processes of one device stay connected");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_runs_over_iroh_and_a_stalled_file_transfer_never_blocks_its_terminal() {
    let (dir, running) = relayed_server().await;
    let laptop = paired(&running, &identity("laptop", &DeviceKey::generate())).await;
    let (id, _shown) = held_terminal(dir.path(), &laptop).await;

    let large = vec![b'x'; 8 << 20];
    std::fs::write(dir.path().join("large.bin"), &large).expect("write a large file");
    let mut transfer = laptop.read(ReadDir::Checkout(id.clone()), "large.bin".into(), 0, Stream::TO_END).await;
    let mut terminal = laptop.open_data_stream(&agent(&laptop, &id).await).await.expect("attach");
    laptop.call(&send(&id, "agent", "over-iroh")).await.expect("term.send");
    read_until(&mut terminal, "over-iroh").await;

    let mut received = Vec::new();
    transfer.as_mut().expect("open the transfer").1.read_to_end(&mut received).await.expect("read the transfer");
    assert_eq!(received.len(), large.len());
}

/// A task whose agent terminal `holder` shows and holds the lock of.
async fn held_terminal(dir: &Path, holder: &Connection) -> (TaskId, DataStream) {
    let workspace = workspace(holder, dir).await;
    set_harness(holder, "echo", echo_agent("agent:up"), Hooks::None).await;
    let id = create(holder, &workspace, CheckoutKind::Root, "echo").await;
    focus(holder, &id).await;
    let mut shown = holder.open_data_stream(&agent(holder, &id).await).await.expect("attach");
    read_until(&mut shown, "agent:up").await;
    holder.call(&send(&id, "agent", "x")).await.expect("take the lock");
    (id, shown)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_lock_follows_its_holders_key_and_not_a_name_it_gave_up() {
    let (dir, running) = relayed_server().await;
    let [laptop, phone, local_laptop] =
        ["laptop", "phone", "laptop"].map(|name| identity(name, &DeviceKey::generate()));
    let holder = paired(&running, &laptop).await;
    paired(&running, &phone).await;
    let (id, _shown) = held_terminal(dir.path(), &holder).await;
    let (on_socket, over_iroh) = (common::local(&running.socket), remote(&running));

    let renamed = Connection::connect(&over_iroh, &identity("desk", &laptop.key)).await.expect("rename the laptop");
    let panel = task(&renamed, &id).await.panels.remove(0);
    assert_eq!(holder_name(&panel), Some("desk"), "the lock is labelled with its holder's new name");
    let write = TermSend { take_lock: false, ..send(&id, "agent", "x") };
    let impostor = Connection::connect(&over_iroh, &identity("laptop", &phone.key)).await.expect("a freed name");
    let local = Connection::connect(&on_socket, &local_laptop).await.expect("connect on the socket");
    for other in [&impostor, &local] {
        let written = other.call(&write).await;
        let refused = matches!(&written, Err(CallError::Server(Error::PtyLocked { holder })) if holder == "desk");
        assert!(refused, "{written:?}");
    }
    let paired_lock = panel.lock_holder.expect("the paired laptop holds the lock");
    assert!(laptop.holds(&paired_lock, &over_iroh) && !local_laptop.holds(&paired_lock, &on_socket));
    holder.call(&write).await.expect("the holder still writes under its old name");

    let claim = TermClaim { task: id.clone(), panel: "agent".into(), cols: panel.cols, rows: panel.rows };
    holder.call(&claim).await.expect("claim under the old name");
    holder.call(&send(&id, "agent", "x")).await.expect("take under the old name");
    assert_eq!(holder_name(&task(&holder, &id).await.panels[0]), Some("desk"));

    local.call(&send(&id, "agent", "x")).await.expect("the local laptop takes the lock");
    let local_lock = task(&local, &id).await.panels.remove(0).lock_holder.expect("the local laptop holds it");
    assert!(local_laptop.holds(&local_lock, &on_socket) && !laptop.holds(&local_lock, &over_iroh));
}

#[tokio::test(flavor = "multi_thread")]
async fn forgetting_a_device_closes_its_connections_and_frees_every_lock_it_holds() {
    let (dir, running) = relayed_server().await;
    let laptop = identity("laptop", &DeviceKey::generate());
    let holder = paired(&running, &laptop).await;
    let (id, _shown) = held_terminal(dir.path(), &holder).await;

    let owner = connect(&running.socket, "owner").await;
    owner.call(&forget(&laptop)).await.expect("forget the laptop");
    assert_eq!(holder_name(&task(&owner, &id).await.panels[0]), None);
    let closed = tokio::time::timeout(Duration::from_secs(5), holder.closed()).await.expect("the connection closes");
    assert!(closed.contains("no longer a member"), "{closed}");
    let claim = TermClaim { task: id.clone(), panel: "agent".into(), cols: 80, rows: 24 };
    assert!(holder.call(&claim).await.is_err(), "a forgotten device claims nothing");
    assert_eq!(holder_name(&task(&owner, &id).await.panels[0]), None);
}

fn open(task: &TaskId, url: &str, panel: BrowserTarget) -> BrowserOpen {
    BrowserOpen { task: task.clone(), url: url.into(), panel }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_browser_panel_is_task_state_and_its_localhost_page_tunnels_to_the_servers_port() {
    let (dir, running) = relayed_server().await;
    let laptop = Arc::new(paired(&running, &identity("laptop", &DeviceKey::generate())).await);
    let desk = connect(&running.socket, "desk").await;
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
    let read = |path, start, len| common::read(&laptop, ReadDir::Page(site.display().to_string()), path, start, len);
    assert_eq!(
        read("css/style.css", 0, Stream::TO_END).await.expect("a file of the page's root"),
        (4, b"h1{}".to_vec())
    );
    assert_eq!(read("css/style.css", 1, 2).await.expect("a range of it"), (4, b"1{".to_vec()));
    let secret = dir.path().join("secret").display().to_string();
    for outside in ["../secret", "css/../../secret", "leak", secret.as_str()] {
        let refused = read(outside, 0, Stream::TO_END).await;
        assert!(matches!(refused, Err(CallError::Server(Error::InvalidArgs { .. }))), "{outside}: {refused:?}");
    }
    let missing = read("css", 0, Stream::TO_END).await;
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
    let (_dir, running) = relayed_server().await;
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
    let unsigned = [identity("laptop", &DeviceKey::generate()).key.public().as_bytes().as_slice(), &[0u8; 64]].concat();
    proof.write_all(&unsigned).await.expect("send the proof");
    proof.finish().expect("finish the proof");
    let closed = tokio::time::timeout(Duration::from_secs(5), connection.closed()).await.expect("closed");
    assert!(closed.to_string().contains("no valid device proof"), "{closed}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_linked_device_brings_its_servers_and_a_removal_shuts_it_out_of_every_one() {
    let (_laptop_dir, laptop_server) = relayed_server().await;
    let (_desk_dir, desk) = relayed_server().await;
    let (laptop, phone) = (identity("laptop", &DeviceKey::generate()), identity("phone", &DeviceKey::generate()));
    let on_laptop = Connection::connect(&common::local(&laptop_server.socket), &laptop).await.expect("connect");
    paired(&desk, &phone).await;

    let link = link(&laptop_server, &laptop, &on_laptop).await;
    let account = link.account;
    let spare = link.to_string().parse::<Link>().expect("a link");
    servers::join(&link, &phone).await.expect("join");
    let spent =
        servers::join(&spare, &identity("tablet", &DeviceKey::generate())).await.expect_err("a link joins one device");
    assert!(format!("{spent:#}").contains("expires or is spent"), "{spent:#}");
    assert_eq!(phone.account.account().id(), account);

    let reachable = Servers::default().reachable(&phone);
    let mut addresses: Vec<String> = reachable.iter().map(|server| server.address.to_string()).collect();
    let mut both = [remote(&laptop_server).to_string(), remote(&desk).to_string()];
    addresses.sort();
    both.sort();
    assert_eq!(addresses, both, "the phone reaches the laptop's server and its own, each under a name of its own");
    let mut over_iroh = Vec::new();
    for server in &reachable {
        over_iroh.push(Connection::connect(&server.address, &phone).await.expect("the phone reaches both servers"));
    }
    on_laptop.sync_account(&laptop).await.expect("sync");
    let desk_address = remote(&desk);
    let unpaired = Connection::connect(&desk_address, &laptop).await.expect("the desk follows the phone's account");

    laptop.account.append(&laptop.key, Change::Remove { device: phone.key.public() }).expect("remove");
    on_laptop.sync_account(&laptop).await.expect("sync the removal");
    unpaired.sync_account(&laptop).await.expect("sync the removal to the desk");
    for connection in &over_iroh {
        let closed = tokio::time::timeout(Duration::from_secs(5), connection.closed()).await.expect("closes");
        assert!(closed.contains("no longer a member"), "{closed}");
    }
    let refused = Connection::connect(&desk_address, &phone).await.err().expect("the phone is out");
    assert!(format!("{refused:#}").contains("no pairing code or link is on offer"), "{refused:#}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_paired_to_the_links_server_joins_through_it_and_no_other_account_writes_into_this_one() {
    let (_dir, running) = relayed_server().await;
    let (laptop, tablet, stranger) = (
        identity("laptop", &DeviceKey::generate()),
        identity("tablet", &DeviceKey::generate()),
        identity("stranger", &DeviceKey::generate()),
    );
    let on_laptop = Connection::connect(&common::local(&running.socket), &laptop).await.expect("connect");
    paired(&running, &tablet).await;
    let intruder = paired(&running, &stranger).await;

    let entries = laptop.account.account().entries.len();
    let mut forged = laptop.account.account();
    forged.append(&stranger.key, Change::Name { name: "laptop's".into() });
    let written = intruder.call(&AccountSync { account: forged, former: vec![] }).await;
    assert!(matches!(written, Err(CallError::Server(Error::Unpaired))), "{written:?}");
    on_laptop.sync_account(&laptop).await.expect("sync");
    assert_eq!(laptop.account.account().entries.len(), entries, "the stranger's entry never reached the log");

    let link = link(&running, &laptop, &on_laptop).await;
    servers::join(&link, &tablet).await.expect("a paired device joins through its server");
    on_laptop.sync_account(&laptop).await.expect("sync");
    assert!(laptop.account.state().is_member(&tablet.key.public()));
}
