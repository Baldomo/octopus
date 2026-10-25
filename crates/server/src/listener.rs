use std::sync::Arc;
use std::time::Duration;

use octopus_proto::device::PublicKey;
use octopus_proto::local::{self, Listener};
use octopus_proto::ops::{Paired, PairingPair, SessionHello, SessionPing};
use octopus_proto::types::{PtyId, Stream};
use octopus_proto::{
    Error, Op, Outcome, Request, Response, ServerFrame, StreamHeader, line, parse_args, parse_request, quic,
    request_from,
};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Semaphore, broadcast, mpsc};

use crate::{Server, report};

const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);
const INPUT_BYTES: usize = 4096;
/// How long each step of pairing may take: the request arriving, and the reply being acknowledged.
const PAIRING_PATIENCE: Duration = Duration::from_secs(10);
/// A `pairing.pair` request line is far shorter; an unpaired peer gets no more of the server's memory.
const PAIRING_LINE_BYTES: u64 = 4096;
/// Requests of one control connection waiting their turn; a client that sends faster waits on its stream.
const QUEUED_REQUESTS: usize = 64;
/// Connections at once whose client is not yet known to be paired.
const UNPAIRED_CONNECTIONS: usize = 16;

/// Who a stream comes from. The local socket admits only the server's own OS user, who names any client; a paired
/// client is the name it paired under.
#[derive(Clone)]
enum Peer {
    Local,
    Paired { name: String, key: PublicKey },
}

impl Peer {
    fn admit(&self, client: &str) -> Result<(), Error> {
        match self {
            Self::Paired { name, .. } if name != client => {
                Err(Error::invalid(format!("this connection is paired as {name}, not {client}")))
            }
            _ => Ok(()),
        }
    }
}

/// Serves the local socket and QUIC at once, with one protocol on both.
pub async fn serve(server: Arc<Server>, local: Listener, quic: quinn::Endpoint) {
    tokio::spawn(remove_expired_worktrees(Arc::clone(&server)));
    tokio::spawn(serve_quic(Arc::clone(&server), quic));
    local::accept_forever(local, |stream| drop(tokio::spawn(serve_stream(Arc::clone(&server), stream, Peer::Local))))
        .await
}

async fn serve_quic(server: Arc<Server>, endpoint: quinn::Endpoint) {
    let unpaired = Arc::new(Semaphore::new(UNPAIRED_CONNECTIONS));
    while let Some(incoming) = endpoint.accept().await {
        let (server, unpaired) = (Arc::clone(&server), Arc::clone(&unpaired));
        tokio::spawn(async move {
            match incoming.await {
                Ok(connection) => serve_quic_connection(server, connection, &unpaired).await,
                Err(error) => report!("a QUIC handshake failed: {error}"),
            }
        });
    }
}

/// Every stream of a paired connection carries the control frames or one data stream, as a local connection does.
async fn serve_quic_connection(server: Arc<Server>, connection: quinn::Connection, unpaired: &Semaphore) {
    let key = quic::peer_key(&connection);
    let pinned = blocking(&server, move |server| server.paired_name(&key)).await;
    let name = match pinned {
        Ok(Some(name)) => name,
        Ok(None) => {
            let Ok(_pairing) = unpaired.try_acquire() else {
                return connection.close(quic::REFUSED, b"too many unpaired connections");
            };
            match pair(&server, &connection, key).await {
                Some(name) => name,
                None => return connection.close(quic::REFUSED, b"unpaired"),
            }
        }
        Err(error) => {
            report!("look up the pin of {key}: {error}");
            return connection.close(quic::REFUSED, b"internal error");
        }
    };
    connection.set_max_concurrent_bi_streams(quic::PAIRED_STREAMS);
    while let Ok((send, receive)) = connection.accept_bi().await {
        let peer = Peer::Paired { name: name.clone(), key };
        tokio::spawn(serve_stream(Arc::clone(&server), tokio::io::join(receive, send), peer));
    }
}

/// An unpaired client's first stream carries one `pairing.pair`; the client is paired once it proves a code.
async fn pair(server: &Arc<Server>, connection: &quinn::Connection, key: PublicKey) -> Option<String> {
    let (mut send, receive) = tokio::time::timeout(PAIRING_PATIENCE, connection.accept_bi()).await.ok()?.ok()?;
    let mut first = String::new();
    let mut bounded = BufReader::new(receive.take(PAIRING_LINE_BYTES));
    tokio::time::timeout(PAIRING_PATIENCE, bounded.read_line(&mut first)).await.ok()?.ok()?;
    let (id, claimed) = match parse_request(&first) {
        Ok(request) => (request.id, claim(server, key, &request).await),
        Err((id, error)) => (id, Err(error)),
    };
    let name = claimed.as_ref().ok().map(|(name, _)| name.clone());
    let reply = respond(id, Outcome::from(claimed.map(|(_, paired)| paired)));
    if send.write_all(reply.as_bytes()).await.is_ok() && send.finish().is_ok() {
        // Closing the connection drops unacknowledged bytes, and a refusal must still reach the client.
        let _acknowledged_or_gone = tokio::time::timeout(PAIRING_PATIENCE, send.stopped()).await;
    }
    name
}

/// Pins the client proving `key` once its request proves a code. A paired client may pair again, to pin the
/// server's key anew.
async fn claim(server: &Arc<Server>, key: PublicKey, request: &Request) -> Result<(String, Paired), Error> {
    if request.op != PairingPair::NAME {
        return Err(Error::Unpaired);
    }
    let op = parse_args::<PairingPair>(&request.args)?;
    let name = op.client.clone();
    let paired = blocking(server, move |server| server.pair(&key, op)).await?;
    Ok((name, paired))
}

async fn remove_expired_worktrees(server: Arc<Server>) {
    let mut interval = tokio::time::interval(SWEEP_INTERVAL);
    loop {
        interval.tick().await;
        let server = Arc::clone(&server);
        match tokio::task::spawn_blocking(move || server.remove_expired_worktrees(crate::now())).await {
            Ok(Ok(sweep)) => {
                for (task, error) in sweep.failed {
                    report!("removing the worktree of {task}: {error}");
                }
            }
            Ok(Err(error)) => report!("worktree sweep: {error}"),
            Err(error) => report!("worktree sweep panicked: {error}"),
        }
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
    if lines.read_line(&mut first).await.unwrap_or(0) == 0 {
        return;
    }
    let reply = match serde_json::from_str::<Value>(&first) {
        Ok(value) if value.get("open").is_some() => {
            let header =
                parse_args::<StreamHeader>(&value).and_then(|header| peer.admit(&header.client).map(|()| header));
            match header {
                Ok(header) => return serve_data_stream(server, header, lines, write).await,
                Err(error) => line(&Outcome::Err(error)),
            }
        }
        Ok(value) => match (request_from(value), &peer) {
            (Ok(request), Peer::Paired { key, .. }) if request.op == PairingPair::NAME => {
                let claimed = claim(&server, *key, &request).await;
                respond(request.id, Outcome::from(claimed.map(|(_, paired)| paired)))
            }
            (Ok(request), _) => match hello(&request, &peer) {
                Ok(client) => return serve_control(server, request, client, lines, write).await,
                Err(error) => respond(request.id, Outcome::Err(error)),
            },
            (Err((id, error)), _) => respond(id, Outcome::Err(error)),
        },
        Err(error) => respond(0, Outcome::Err(Error::invalid(error))),
    };
    let _peer_gone = write.write_all(reply.as_bytes()).await;
}

fn hello(request: &Request, peer: &Peer) -> Result<String, Error> {
    if request.op != SessionHello::NAME {
        return Err(Error::invalid(format!("the first request must be {}", SessionHello::NAME)));
    }
    let hello = parse_args::<SessionHello>(&request.args)?;
    peer.admit(&hello.client)?;
    Ok(hello.client)
}

async fn serve_control<R, W>(server: Arc<Server>, hello: Request, client: String, mut lines: BufReader<R>, mut write: W)
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
    File(Vec<u8>),
    Pty(PtyId, local::Stream),
    Tunnel(tokio::net::TcpStream),
}

async fn open(server: &Arc<Server>, name: &str) -> Result<Opened, Error> {
    match Stream::parse(name).ok_or_else(|| Error::invalid(format!("no stream named {name}")))? {
        Stream::File { task, path } => {
            Ok(Opened::File(blocking(server, move |server| server.read_file(&task, &path)).await?))
        }
        Stream::Pty(id) => {
            let lookup = id.clone();
            let terminal =
                blocking(server, move |server| server.terminal(&lookup).ok_or_else(|| Error::not_found(&lookup)))
                    .await?;
            Ok(Opened::Pty(id, terminal.attach().await?))
        }
        Stream::Tunnel(port) => Ok(Opened::Tunnel(crate::browser::dial(port).await?)),
    }
}

async fn serve_data_stream<R, W>(server: Arc<Server>, header: StreamHeader, input: BufReader<R>, mut write: W)
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let opened = open(&server, &header.open).await;
    let reply = line(&Outcome::from(opened.as_ref().map(drop).map_err(Error::clone)));
    if write.write_all(reply.as_bytes()).await.is_err() {
        return;
    }
    let Ok(opened) = opened else { return };
    match opened {
        Opened::File(bytes) => {
            if write.write_all(&bytes).await.is_ok() {
                let _peer_gone = write.shutdown().await;
            }
        }
        Opened::Pty(id, output) => relay_terminal(server, header.client, id, output, input, write).await,
        Opened::Tunnel(mut port) => {
            let _either_side_closed =
                tokio::io::copy_bidirectional(&mut tokio::io::join(input, write), &mut port).await;
        }
    }
}

async fn relay_terminal<R, W>(
    server: Arc<Server>,
    client: String,
    id: PtyId,
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
    loop {
        tokio::select! {
            read = input.read(&mut buffer) => match read {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    let (from, to, bytes) = (client.clone(), id.clone(), buffer[..read].to_vec());
                    match blocking(&server, move |server| server.pty_input(&from, &to, &bytes)).await {
                        Ok(()) | Err(Error::PtyLocked { .. }) => {}
                        Err(error) => report!("input from {client} to {id}: {error}"),
                    }
                }
            },
            _ = &mut output => break,
        }
    }
    output.abort();
    blocking(&server, move |server| server.client_disconnected(&client)).await;
}

async fn blocking<T: Send + 'static>(server: &Arc<Server>, work: impl FnOnce(&Server) -> T + Send + 'static) -> T {
    let server = Arc::clone(server);
    tokio::task::spawn_blocking(move || work(&server)).await.expect("server work does not panic")
}
