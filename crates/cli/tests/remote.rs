mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{Running, SERVER, connect, create, echo_agent, eventually, identity, read_until, start, task, workspace};
use octopus_client::servers::{self, Address};
use octopus_client::tunnel::Tunnel;
use octopus_client::{CallError, Connection, Identity};
use octopus_proto::device::DeviceKey;
use octopus_proto::ops::{BrowserOpen, HarnessSet, PairingForget, PairingList, PairingOffer, TaskFocus, TermSend};
use octopus_proto::pairing::Code;
use octopus_proto::types::{BrowserId, BrowserPanel, BrowserTarget, Checkout, Stream, TaskId};
use octopus_proto::{Error, Event};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn offer(running: &Running) -> Code {
    connect(running, "owner").await.call(&PairingOffer {}).await.expect("pairing.offer").code
}

fn remote(running: &Running) -> Address {
    Address::Quic { endpoint: running.quic.to_string(), key: running.server.device().public() }
}

async fn paired(running: &Running, me: &Identity) -> Connection {
    let listed = servers::pair(&running.quic.to_string(), &offer(running).await, me).await.expect("pair");
    assert_eq!(listed, servers::Server { name: SERVER.into(), address: remote(running) });
    Connection::connect(&listed.address, me).await.expect("connect over QUIC")
}

fn twin(of: &Identity, name: &str) -> Identity {
    Identity { name: name.into(), key: DeviceKey::from_pkcs8(of.key.pkcs8()).expect("same key") }
}

#[tokio::test(flavor = "multi_thread")]
async fn only_a_client_that_spent_a_code_reaches_a_remote_server() {
    let dir = tempfile::tempdir().expect("tempdir");
    let running = start(dir.path(), "server.sock", "host.sock");
    let laptop = identity("laptop");

    let refused = Connection::connect(&remote(&running), &laptop).await.err().expect("an unpaired client is refused");
    assert!(format!("{refused:#}").contains("not paired"), "{refused:#}");

    let code = offer(&running).await;
    servers::pair(&running.quic.to_string(), &code, &laptop).await.expect("pair");
    let spent = servers::pair(&running.quic.to_string(), &code, &identity("desk")).await.expect_err("spent");
    assert!(format!("{spent:#}").contains("code is spent"), "{spent:#}");
    let (fresh, namesake) = (offer(&running).await, identity("laptop"));
    let refused = servers::pair(&running.quic.to_string(), &fresh, &namesake).await;
    assert!(format!("{:#}", refused.expect_err("a namesake")).contains("another device's key"));
    connect(&running, "owner").await.call(&PairingForget { client: "laptop".into() }).await.expect("forget");
    servers::pair(&running.quic.to_string(), &fresh, &namesake).await.expect("the refusal left the code unspent");
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

    let moved = Address::Quic { endpoint: running.quic.to_string(), key: DeviceKey::generate().public() };
    let mismatch = Connection::connect(&moved, &laptop).await.err().expect("the client refuses a changed key");
    assert!(format!("{mismatch:#}").contains("not its pinned key"), "{mismatch:#}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_runs_over_quic_and_a_stalled_file_transfer_never_blocks_its_terminal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let running = start(dir.path(), "server.sock", "host.sock");
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
    let send = TermSend { task: id, panel: "agent".into(), text: "over-quic".into(), enter: true, take_lock: false };
    laptop.call(&send).await.expect("term.send");
    read_until(&mut terminal, "over-quic").await;

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
    let running = start(dir.path(), "server.sock", "host.sock");
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
