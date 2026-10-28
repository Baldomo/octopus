mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{Running, SERVER, connect, create, echo_agent, eventually, identity, read_until, start, task, workspace};
use octopus_client::servers::{self, Address};
use octopus_client::tunnel::Tunnel;
use octopus_client::{CallError, Connection, Identity};
use octopus_proto::device::DeviceKey;
use octopus_proto::ops::{BrowserOpen, HarnessSet, PairingForget, PairingList, PairingOffer, TaskFocus, TermSend};
use octopus_proto::pairing::Invite;
use octopus_proto::types::{BrowserId, BrowserPanel, BrowserTarget, Checkout, Stream, TaskId};
use octopus_proto::{Error, Event};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn offer(running: &Running) -> Invite {
    connect(running, "owner").await.call(&PairingOffer {}).await.expect("pairing.offer").invite
}

/// The server as a client reaches it through its relay alone.
fn remote(running: &Running) -> Address {
    Address::Remote { key: running.server.device().public(), relay: running.server.relay().clone(), addresses: vec![] }
}

async fn paired(running: &Running, me: &Identity) -> Connection {
    let invite = offer(running).await;
    assert!(!invite.addresses.is_empty(), "an invite carries the server's direct addresses");
    let listed = servers::pair(&invite, me).await.expect("pair");
    let (key, relay, addresses) = (invite.server, running.server.relay().clone(), invite.addresses.clone());
    let address = Address::Remote { key, relay, addresses };
    assert_eq!(listed, servers::Server { name: SERVER.into(), address });
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
    assert!(format!("{:#}", refused.expect_err("a namesake")).contains("another device's key"));
    connect(&running, "owner").await.call(&PairingForget { client: "laptop".into() }).await.expect("forget");
    servers::pair(&fresh, &namesake).await.expect("the refusal left the code unspent");
    let laptop = namesake;

    let connection = Connection::connect(&remote(&running), &laptop).await.expect("a paired client is accepted");
    eventually("a heartbeat round trip", async || connection.rtt().is_some()).await;
    let listed = connection.call(&PairingList {}).await.expect("pairing.list").clients;
    let [client] = listed.as_slice() else { panic!("one paired client, got {listed:?}") };
    assert!(client.name == "laptop" && client.connected && client.seen_at.is_some(), "{client:?}");
    let unknown = connection.call(&PairingForget { client: "desk".into() }).await;
    assert!(matches!(unknown, Err(CallError::Server(Error::NotFound { .. }))), "{unknown:?}");
    let impostor = Connection::connect(&remote(&running), &twin(&laptop, "desk")).await.err().expect("impostor");
    assert!(format!("{impostor:#}").contains("paired as laptop"), "{impostor:#}");

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
        hooks: false,
    };
    laptop.call(&harness).await.expect("harness.set");
    let id = create(&laptop, &workspace, Checkout::Root, "echo").await;
    laptop.call(&TaskFocus { task: id.clone() }).await.expect("task.focus");
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
    let id = create(&desk, &workspace, Checkout::Root, "none").await;

    let shown = laptop.call(&open(&id, "http://localhost:5173/", BrowserTarget::New)).await.expect("browser.open");
    let settings = "http://localhost:5173/settings";
    let again = laptop.call(&open(&id, settings, BrowserTarget::Panel(shown.panel.clone()))).await.expect("again");
    assert_eq!(again.panel, shown.panel);
    let expected = vec![BrowserPanel { panel: shown.panel, url: settings.into() }];
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
    for (url, panel) in
        [("javascript:alert(1)", BrowserTarget::New), (settings, BrowserTarget::Panel(BrowserId::random()))]
    {
        match laptop.call(&open(&id, url, panel)).await {
            Err(CallError::Server(Error::InvalidArgs { .. } | Error::NotFound { .. })) => {}
            other => panic!("{url} must be refused, got {other:?}"),
        }
    }

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

#[tokio::test(flavor = "multi_thread")]
async fn a_server_on_the_same_network_pairs_and_connects_with_no_relay_reachable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let running = common::start_unrelayed(dir.path()).await;
    let laptop = paired(&running, &identity("laptop")).await;
    laptop.call(&PairingList {}).await.expect("a call over the direct path");
    assert!(!laptop.server_addresses().is_empty(), "each hello names where the server is reachable now");
}
