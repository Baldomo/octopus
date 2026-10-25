pub mod cache;
pub mod servers;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, anyhow};
use octopus_proto::local;
use octopus_proto::ops::SessionHello;
use octopus_proto::types::Stream;
use octopus_proto::{Error, Op, Outcome, Push, Request, ServerFrame, StreamHeader, line};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, WriteHalf};
use tokio::sync::{broadcast, oneshot};

const PUSH_BACKLOG: usize = 4096;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Outcome>>>>;

/// One data stream: reads carry its bytes, writes are its input.
pub type DataStream = local::Stream;

pub struct Connection {
    socket: PathBuf,
    client: String,
    write: tokio::sync::Mutex<WriteHalf<local::Stream>>,
    pending: Pending,
    next_id: AtomicU64,
    pushes: broadcast::Sender<Push>,
}

#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error("{0}")]
    Server(Error),
    #[error(transparent)]
    Transport(#[from] anyhow::Error),
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

impl Connection {
    pub async fn connect(socket: &Path, client: &str) -> anyhow::Result<Self> {
        let stream = local::connect(socket).await.with_context(|| format!("connect {}", socket.display()))?;
        let (read, write) = tokio::io::split(stream);
        let pending: Pending = Arc::default();
        let (pushes, _) = broadcast::channel(PUSH_BACKLOG);
        let (reader_pending, reader_pushes) = (Arc::clone(&pending), pushes.clone());
        tokio::spawn(async move {
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(frame)) = lines.next_line().await {
                match serde_json::from_str::<ServerFrame>(&frame) {
                    Ok(ServerFrame::Response(response)) => {
                        if let Some(waiter) = lock(&reader_pending).remove(&response.id) {
                            let _caller_gone = waiter.send(response.outcome);
                        }
                    }
                    Ok(ServerFrame::Push(push)) => {
                        let _no_subscribers = reader_pushes.send(push);
                    }
                    Err(error) => {
                        eprintln!("octopus: closing a connection on an unreadable frame: {error}");
                        break;
                    }
                }
            }
            lock(&reader_pending).clear();
        });
        let connection = Self {
            socket: socket.to_path_buf(),
            client: client.to_string(),
            write: tokio::sync::Mutex::new(write),
            pending,
            next_id: AtomicU64::new(1),
            pushes,
        };
        connection.call(&SessionHello { client: client.to_string() }).await.map_err(|error| anyhow!("{error}"))?;
        Ok(connection)
    }

    pub fn pushes(&self) -> broadcast::Receiver<Push> {
        self.pushes.subscribe()
    }

    pub async fn call<O: Op>(&self, op: &O) -> Result<O::Ok, CallError> {
        let args = serde_json::to_value(op).context("encode args")?;
        match self.call_raw(O::NAME, args).await? {
            Outcome::Ok(value) => Ok(serde_json::from_value(value).context("decode result")?),
            Outcome::Err(error) => Err(CallError::Server(error)),
        }
    }

    pub async fn call_raw(&self, op: &str, args: Value) -> anyhow::Result<Outcome> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (waiter, outcome) = oneshot::channel();
        lock(&self.pending).insert(id, waiter);
        let request = line(&Request { id, op: op.to_string(), args });
        self.write.lock().await.write_all(request.as_bytes()).await.context("send request")?;
        outcome.await.context("connection closed before the response")
    }

    /// Each data stream rides its own connection.
    pub async fn open_data_stream(&self, stream: &Stream) -> Result<DataStream, CallError> {
        let header = StreamHeader { open: stream.name(), client: self.client.clone() };
        let (connection, outcome) = local::open(&self.socket, &header).await.context("open the stream")?;
        match outcome {
            Outcome::Ok(_) => Ok(connection),
            Outcome::Err(error) => Err(CallError::Server(error)),
        }
    }
}
