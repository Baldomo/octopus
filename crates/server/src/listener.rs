use std::sync::Arc;
use std::time::Duration;

use octopus_proto::device::PublicKey;
use octopus_proto::local::{self, Listener};
use octopus_proto::ops::{AccountJoin, Done, PairingPair, SessionHello, SessionPing};
use octopus_proto::remote::{self, Connection, Endpoint};
use octopus_proto::types::Stream;
use octopus_proto::{
    Error, FileSize, Op, Outcome, Request, Response, ServerFrame, StreamHeader, line, parse_args, parse_request,
    request_from,
};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Semaphore, broadcast, mpsc};

use crate::client::Client;
use crate::terminals::{Refusal, Terminal};
use crate::{Server, report};

const WORKTREE_SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);
const RECLAIM_SWEEP_INTERVAL: Duration = Duration::from_secs(60);
const INPUT_BYTES: usize = 4096;
/// How long each step of pairing may take: the request arriving, and the reply being acknowledged.
const PAIRING_PATIENCE: Duration = Duration::from_secs(10);
/// A stream's first line carries at most an account, in a `pairing.pair`; an unpaired peer gets no more of the
/// server's memory, and a longer line fails to parse.
const FIRST_LINE_BYTES: u64 = 4 << 20;
/// Requests of one control connection waiting their turn; a client that sends faster waits on its stream.
const QUEUED_REQUESTS: usize = 64;
/// Connections at once whose client is not yet known to be paired.
const UNPAIRED_CONNECTIONS: usize = 16;
/// Connections at once whose device proof has not arrived yet.
const UNPROVEN_CONNECTIONS: usize = 64;

const NO_PROOF: &[u8] = b"no valid device proof";
const NOTHING_ON_OFFER: &[u8] =
    b"unpaired, and no pairing code or link is on offer: each expires or is spent, so make a fresh one";
const UNPAIRED: &[u8] = b"unpaired";
const TOO_MANY_UNPROVEN: &[u8] = b"too many connections awaiting their device proof";
const TOO_MANY_UNPAIRED: &[u8] = b"too many unpaired connections";

/// Who a stream comes from: the local socket admits only the server's own OS user, and a paired device is the key
/// it proved.
#[derive(Clone, Copy)]
enum Peer {
    Local,
    Paired(PublicKey),
}

impl Peer {
    /// The client a `session.hello` names; a paired device's hello records the name it gives.
    async fn hello(self, server: &Arc<Server>, name: String) -> Result<Client, Error> {
        match self {
            Self::Local => Ok(Client::Local(name)),
            Self::Paired(key) => blocking(server, move |server| server.hello_paired(key, &name)).await,
        }
    }

    /// The client a data stream's header names; a paired device is its key, whatever name the header gives.
    fn stream(self, name: String) -> Client {
        match self {
            Self::Local => Client::Local(name),
            Self::Paired(key) => Client::Paired(key),
        }
    }
}

/// Serves the local socket and iroh at once, with one protocol on both.
pub async fn serve(server: Arc<Server>, local: Listener, remote: Endpoint) {
    tokio::spawn(every(WORKTREE_SWEEP_INTERVAL, Arc::clone(&server), "worktree sweep", remove_expired_worktrees));
    tokio::spawn(every(RECLAIM_SWEEP_INTERVAL, Arc::clone(&server), "agent reclaim", reclaim_agents));
    tokio::spawn(serve_remote(Arc::clone(&server), remote));
    local::accept_forever(local, |stream| drop(tokio::spawn(serve_stream(Arc::clone(&server), stream, Peer::Local))))
        .await
}

async fn serve_remote(server: Arc<Server>, endpoint: Endpoint) {
    let unproven = Arc::new(Semaphore::new(UNPROVEN_CONNECTIONS));
    let unpaired = Arc::new(Semaphore::new(UNPAIRED_CONNECTIONS));
    while let Some(incoming) = endpoint.accept().await {
        let (server, unproven, unpaired) = (Arc::clone(&server), Arc::clone(&unproven), Arc::clone(&unpaired));
        tokio::spawn(async move {
            match incoming.await {
                Ok(connection) => serve_remote_connection(server, connection, &unproven, &unpaired).await,
                Err(error) => report!("an iroh handshake failed: {error}"),
            }
        });
    }
}

/// A device the server has not pinned is closed before any stream but its device proof is read, unless a code is on
/// offer. Every stream of a paired connection carries the control frames or one data stream, as a local connection
/// does.
async fn serve_remote_connection(
    server: Arc<Server>,
    connection: Connection,
    unproven: &Semaphore,
    unpaired: &Semaphore,
) {
    let Ok(proving) = unproven.try_acquire() else {
        return connection.close(remote::REFUSED, TOO_MANY_UNPROVEN);
    };
    let Some(key) = remote::proven_device(&connection, server.device().public()).await else {
        return connection.close(remote::REFUSED, NO_PROOF);
    };
    drop(proving);
    if !track(&server, key, &connection).await {
        if !blocking(&server, Server::offering).await {
            return connection.close(remote::REFUSED, NOTHING_ON_OFFER);
        }
        let Ok(_pairing) = unpaired.try_acquire() else {
            return connection.close(remote::REFUSED, TOO_MANY_UNPAIRED);
        };
        if !pair(&server, &connection, key).await || !track(&server, key, &connection).await {
            return connection.close(remote::REFUSED, UNPAIRED);
        }
    }
    remote::admit(&connection);
    while let Ok((send, receive)) = connection.accept_bi().await {
        tokio::spawn(serve_stream(Arc::clone(&server), tokio::io::join(receive, send), Peer::Paired(key)));
    }
    blocking(&server, move |server| server.untrack(&key, &connection)).await;
}

/// Whether a trusted account holds `key`, keeping `connection` among its open ones if one does, so a removal or a
/// forget closes it.
async fn track(server: &Arc<Server>, key: PublicKey, connection: &Connection) -> bool {
    let connection = connection.clone();
    blocking(server, move |server| server.track(key, &connection)).await
}

/// An unpaired device's first stream carries one `pairing.pair` or `account.join`; it is in once either succeeds.
async fn pair(server: &Arc<Server>, connection: &Connection, key: PublicKey) -> bool {
    let Ok(Ok((mut send, receive))) = tokio::time::timeout(PAIRING_PATIENCE, connection.accept_bi()).await else {
        return false;
    };
    let mut first = String::new();
    let mut bounded = BufReader::new(receive.take(FIRST_LINE_BYTES));
    let Ok(Ok(_)) = tokio::time::timeout(PAIRING_PATIENCE, bounded.read_line(&mut first)).await else {
        return false;
    };
    let (id, claimed) = match parse_request(&first) {
        Ok(request) => (request.id, claim(server, key, &request).await),
        Err((id, error)) => (id, Err(error)),
    };
    let paired = claimed.is_ok();
    if send.write_all(respond(id, Outcome::from(claimed)).as_bytes()).await.is_ok() && send.finish().is_ok() {
        // Closing the connection drops unacknowledged bytes, and a refusal must still reach the client.
        let _acknowledged_or_gone = tokio::time::timeout(PAIRING_PATIENCE, send.stopped()).await;
    }
    paired
}

/// Trusts the account of the device proving `key` once its request proves a code, or takes its join on a link. A
/// paired device may pair again, to pin the server's key anew.
async fn claim(server: &Arc<Server>, key: PublicKey, request: &Request) -> Result<Value, Error> {
    fn encode(value: impl serde::Serialize) -> Result<Value, Error> {
        serde_json::to_value(value).map_err(Error::internal)
    }
    match request.op.as_str() {
        PairingPair::NAME => {
            let op = parse_args::<PairingPair>(&request.args)?;
            encode(blocking(server, move |server| server.pair(&key, op)).await?)
        }
        AccountJoin::NAME => {
            let op = parse_args::<AccountJoin>(&request.args)?;
            encode(blocking(server, move |server| server.account_join(&key, op)).await?)
        }
        _ => Err(Error::Unpaired),
    }
}

/// Runs `work` off the async threads once every `period`, starting now; a run that panics is reported as `name`'s and
/// the next one runs on time.
async fn every(period: Duration, server: Arc<Server>, name: &'static str, work: fn(&Server)) {
    let mut interval = tokio::time::interval(period);
    loop {
        interval.tick().await;
        let server = Arc::clone(&server);
        if let Err(error) = tokio::task::spawn_blocking(move || work(&server)).await {
            report!("{name} panicked: {error}");
        }
    }
}

fn remove_expired_worktrees(server: &Server) {
    match server.remove_expired_worktrees(crate::now()) {
        Ok(sweep) => {
            for (task, error) in sweep.failed {
                report!("removing the worktree of {task}: {error}");
            }
        }
        Err(error) => report!("worktree sweep: {error}"),
    }
}

fn reclaim_agents(server: &Server) {
    for task in server.reclaim_agents(crate::now()) {
        report!("stopped the idle agent of {task} to reclaim its memory");
    }
}

fn respond(id: u64, outcome: Outcome) -> String {
    line(&ServerFrame::Response(Response { id, outcome }))
}

/// A stream's first line opens a data stream; otherwise it is a request that pairs again or, as a `session.hello`,
/// opens the control frames.
async fn serve_stream<S: AsyncRead + AsyncWrite + Send + 'static>(server: Arc<Server>, stream: S, peer: Peer) {
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read);
    let mut first = String::new();
    if (&mut lines).take(FIRST_LINE_BYTES).read_line(&mut first).await.unwrap_or(0) == 0 {
        return;
    }
    let reply = match serde_json::from_str::<Value>(&first) {
        Ok(value) if value.get("open").is_some() => match serde_json::from_value::<StreamHeader>(value) {
            Ok(header) => {
                return serve_data_stream(server, header.open, peer.stream(header.client), lines, write).await;
            }
            Err(error) => line(&Outcome::Err(Error::invalid(error))),
        },
        Ok(value) => match (request_from(value), peer) {
            (Ok(request), Peer::Paired(key))
                if [PairingPair::NAME, AccountJoin::NAME].contains(&request.op.as_str()) =>
            {
                respond(request.id, Outcome::from(claim(&server, key, &request).await))
            }
            (Ok(request), _) => match hello(&server, &request, peer).await {
                Ok(client) => return serve_control(server, request, client, lines, write).await,
                Err(error) => respond(request.id, Outcome::Err(error)),
            },
            (Err((id, error)), _) => respond(id, Outcome::Err(error)),
        },
        Err(error) => respond(0, Outcome::Err(Error::invalid(error))),
    };
    let _peer_gone = write.write_all(reply.as_bytes()).await;
}

async fn hello(server: &Arc<Server>, request: &Request, peer: Peer) -> Result<Client, Error> {
    if request.op != SessionHello::NAME {
        return Err(Error::invalid(format!("the first request must be {}", SessionHello::NAME)));
    }
    peer.hello(server, parse_args::<SessionHello>(&request.args)?.client).await
}

async fn serve_control<R, W>(server: Arc<Server>, hello: Request, client: Client, mut lines: BufReader<R>, mut write: W)
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send + 'static,
{
    server.client_connected(&client);
    let (out, mut outbox) = mpsc::unbounded_channel::<String>();
    let mut pushes = server.subscribe();
    let _client_gone = out.send(respond(hello.id, server.handle(&client, &hello)));
    let writer = tokio::spawn(async move {
        while let Some(frame) = outbox.recv().await {
            if write.write_all(frame.as_bytes()).await.is_err() {
                break;
            }
        }
    });
    let push_out = out.clone();
    let mut forwarder = tokio::spawn(async move {
        loop {
            match pushes.recv().await {
                Ok(push) => {
                    if push_out.send(line(&ServerFrame::Push(push))).is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    report!("a client fell {missed} pushes behind; closing it so it resyncs");
                    break;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    let (queue, mut queued) = mpsc::channel::<Request>(QUEUED_REQUESTS);
    let (executor_server, executor_client, answers) = (Arc::clone(&server), client.clone(), out.clone());
    let executor = tokio::spawn(async move {
        while let Some(request) = queued.recv().await {
            let (server, client, id) = (Arc::clone(&executor_server), executor_client.clone(), request.id);
            let outcome = tokio::task::spawn_blocking(move || server.handle(&client, &request))
                .await
                .unwrap_or_else(|error| Outcome::Err(Error::internal(error)));
            if answers.send(respond(id, outcome)).is_err() {
                break;
            }
        }
    });
    // Requests run one at a time in order; a ping is answered as it arrives, so it times the connection rather
    // than the op ahead of it.
    let mut request_line = String::new();
    loop {
        request_line.clear();
        tokio::select! {
            read = lines.read_line(&mut request_line) => if read.unwrap_or(0) == 0 { break },
            _ = &mut forwarder => break,
        }
        let sent = match parse_request(&request_line) {
            Ok(request) if request.op == SessionPing::NAME => {
                out.send(respond(request.id, server.handle(&client, &request))).is_ok()
            }
            Ok(request) => queue.send(request).await.is_ok(),
            Err((id, error)) => out.send(respond(id, Outcome::Err(error))).is_ok(),
        };
        if !sent {
            break;
        }
    }
    forwarder.abort();
    // A client that hung up gets no answers, so the requests it left queued do not run.
    executor.abort();
    drop(out);
    let _ = writer.await;
    blocking(&server, move |server| server.client_disconnected(&client)).await;
}

enum Opened {
    Read(u64, tokio::io::Take<tokio::fs::File>),
    Upload(crate::files::Upload),
    Pty(Arc<Terminal>, local::Stream),
    Tunnel(tokio::net::TcpStream),
    Mic(Arc<Terminal>),
}

impl Opened {
    /// What the open reply carries beside success: a read's file size, and nothing for any other stream.
    fn reply(&self) -> Option<FileSize> {
        match self {
            Self::Read(size, _) => Some(FileSize { size: *size }),
            Self::Upload(_) | Self::Pty(..) | Self::Tunnel(_) | Self::Mic(_) => None,
        }
    }
}

async fn open(server: &Arc<Server>, stream: Stream, client: &Client) -> Result<Opened, Error> {
    match stream {
        Stream::Read { dir, path, start, len } => {
            let opened =
                blocking(server, move |server| crate::files::open_read(&server.read_dir(&dir)?, &path, start, len));
            let (size, bytes) = opened.await?;
            Ok(Opened::Read(size, bytes))
        }
        Stream::Upload { task, bytes, path } => {
            let upload =
                blocking(server, move |server| crate::files::upload(&server.checkout_dir(&task)?, &path, bytes));
            Ok(Opened::Upload(upload.await?))
        }
        Stream::Pty(id) => {
            let terminal = blocking(server, move |server| server.terminal(&id)).await?;
            let output = terminal.attach().await?;
            Ok(Opened::Pty(terminal, output))
        }
        Stream::Mic(id) => {
            let client = client.clone();
            let terminal = blocking(server, move |server| {
                let terminal = server.terminal(&id)?;
                server.check_holder(&client, &terminal).map(|()| terminal)
            })
            .await?;
            Ok(Opened::Mic(terminal))
        }
        Stream::Tunnel(port) => Ok(Opened::Tunnel(crate::browser::dial(port).await?)),
    }
}

async fn serve_data_stream<R, W>(server: Arc<Server>, stream: Stream, client: Client, input: BufReader<R>, mut write: W)
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let opened = open(&server, stream, &client).await;
    // The client counts as showing a terminal before it hears the stream opened, so a claim it sends next always
    // finds it showing.
    if let Ok(Opened::Pty(terminal, _)) = &opened {
        terminal.show(&client);
    }
    let reply = line(&Outcome::from(opened.as_ref().map(Opened::reply).map_err(Error::clone)));
    if write.write_all(reply.as_bytes()).await.is_err() {
        if let Ok(Opened::Pty(terminal, _)) = opened {
            blocking(&server, move |server| server.stopped_showing(&client, terminal, true)).await;
        }
        return;
    }
    let Ok(opened) = opened else { return };
    match opened {
        Opened::Read(_, mut bytes) => {
            if tokio::io::copy(&mut bytes, &mut write).await.is_ok() {
                let _peer_gone = write.shutdown().await;
            }
        }
        Opened::Upload(upload) => {
            let landed = Outcome::from(upload.receive(input).await.map(|()| Done {}));
            if write.write_all(line(&landed).as_bytes()).await.is_ok() {
                let _peer_gone = write.shutdown().await;
            }
        }
        Opened::Pty(terminal, output) => {
            relay_terminal(server, client, terminal, output, input, write).await;
        }
        Opened::Mic(terminal) => hear(&server, &client, &terminal, input).await,
        Opened::Tunnel(mut port) => {
            let _either_side_closed =
                tokio::io::copy_bidirectional(&mut tokio::io::join(input, write), &mut port).await;
        }
    }
}

/// Input from a client without the lock is dropped: only the holder types. A stream the client finishes stops it
/// showing the terminal; one that breaks went with its connection. Input is read first, so a finish that arrives
/// with the client's hang-up still reads as a finish.
async fn relay_terminal<R, W>(
    server: Arc<Server>,
    client: Client,
    terminal: Arc<Terminal>,
    mut terminal_output: local::Stream,
    mut input: BufReader<R>,
    mut write: W,
) where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send + 'static,
{
    server.client_connected(&client);
    let mut output = tokio::spawn(async move {
        tokio::io::copy(&mut terminal_output, &mut write).await?;
        write.shutdown().await
    });
    let mut buffer = [0u8; INPUT_BYTES];
    let closed_cleanly = loop {
        tokio::select! {
            biased;
            read = input.read(&mut buffer) => match read {
                Ok(0) => break true,
                Err(_) => break false,
                Ok(read) => {
                    let (from, to, bytes) = (client.clone(), Arc::clone(&terminal), buffer[..read].to_vec());
                    match tokio::task::spawn_blocking(move || to.write_as(&from, &bytes)).await {
                        Ok(Ok(()) | Err(Refusal::HeldBy(_) | Refusal::Free)) => {}
                        Ok(Err(Refusal::Failed(error))) => report!("input from {client} to {}: {error}", terminal.id),
                        Err(error) => report!("input from {client} to {} panicked: {error}", terminal.id),
                    }
                }
            },
            _ = &mut output => break false,
        }
    };
    output.abort();
    blocking(&server, move |server| {
        server.stopped_showing(&client, terminal, closed_cleanly);
        server.client_disconnected(&client);
    })
    .await;
}

/// Takes the holder's audio until it closes the stream or the lock leaves it, which closes the stream.
async fn hear<R: AsyncRead + Unpin>(server: &Server, client: &Client, terminal: &Terminal, mut input: BufReader<R>) {
    server.mic.streaming(&terminal.id, true);
    let mut buffer = [0u8; INPUT_BYTES];
    while let Ok(read) = input.read(&mut buffer).await
        && read > 0
        && server.hear(client, terminal, &buffer[..read]).is_ok()
    {}
    server.mic.streaming(&terminal.id, false);
}

async fn blocking<T: Send + 'static>(server: &Arc<Server>, work: impl FnOnce(&Server) -> T + Send + 'static) -> T {
    let server = Arc::clone(server);
    tokio::task::spawn_blocking(move || work(&server)).await.expect("server work does not panic")
}
