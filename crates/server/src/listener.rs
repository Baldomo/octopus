use std::sync::Arc;
use std::time::Duration;

use octopus_proto::local::{self, Listener};
use octopus_proto::ops::SessionHello;
use octopus_proto::types::{PtyId, Stream};
use octopus_proto::{
    Error, Op, Outcome, Request, Response, ServerFrame, StreamHeader, line, parse_args, parse_request,
};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, mpsc};

use crate::{Server, report};

const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);
const INPUT_BYTES: usize = 4096;

pub async fn serve(server: Arc<Server>, listener: Listener) {
    tokio::spawn(remove_expired_worktrees(Arc::clone(&server)));
    local::accept_forever(listener, |stream| drop(tokio::spawn(serve_connection(Arc::clone(&server), stream)))).await
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

async fn serve_connection<S: AsyncRead + AsyncWrite + Send + 'static>(server: Arc<Server>, stream: S) {
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read);
    let mut first = String::new();
    if lines.read_line(&mut first).await.unwrap_or(0) == 0 {
        return;
    }
    let opens_stream = serde_json::from_str::<serde_json::Value>(&first).is_ok_and(|value| value.get("open").is_some());
    if opens_stream {
        match serde_json::from_str::<StreamHeader>(&first) {
            Ok(header) => serve_data_stream(server, header, lines, write).await,
            Err(error) => {
                let _peer_gone = write.write_all(line(&Outcome::Err(Error::invalid(error))).as_bytes()).await;
            }
        }
        return;
    }
    match parse_hello(&first) {
        Ok((request, client)) => serve_control(server, request, client, lines, write).await,
        Err((id, error)) => {
            let _peer_gone = write.write_all(respond(id, Outcome::Err(error)).as_bytes()).await;
        }
    }
}

fn parse_hello(first: &str) -> Result<(Request, String), (u64, Error)> {
    let request = parse_request(first)?;
    if request.op != SessionHello::NAME {
        return Err((request.id, Error::invalid(format!("the first request must be {}", SessionHello::NAME))));
    }
    let hello = parse_args::<SessionHello>(&request.args).map_err(|error| (request.id, error))?;
    Ok((request, hello.client))
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
    let mut request_line = String::new();
    loop {
        request_line.clear();
        tokio::select! {
            read = lines.read_line(&mut request_line) => if read.unwrap_or(0) == 0 { break },
            _ = &mut forwarder => break,
        }
        let response = match parse_request(&request_line) {
            Ok(request) => {
                let id = request.id;
                let (server, client) = (Arc::clone(&server), client.clone());
                let outcome = tokio::task::spawn_blocking(move || server.handle(&client, &request))
                    .await
                    .unwrap_or_else(|error| Outcome::Err(Error::internal(error)));
                respond(id, outcome)
            }
            Err((id, error)) => respond(id, Outcome::Err(error)),
        };
        if out.send(response).is_err() {
            break;
        }
    }
    forwarder.abort();
    drop(out);
    let _ = writer.await;
    blocking(&server, move |server| server.client_disconnected(&client)).await;
}

enum Opened {
    File(Vec<u8>),
    Pty(PtyId, local::Stream),
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
