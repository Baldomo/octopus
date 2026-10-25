//! One connection from this client to one server, over the local socket or QUIC with the same protocol on both.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use octopus_proto::device::DeviceKey;
use octopus_proto::ops::{SessionHello, SessionPing};
use octopus_proto::types::Stream;
use octopus_proto::{Error, Op, Outcome, Push, Request, ServerFrame, StreamHeader, line, local, quic};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, WriteHalf};
use tokio::sync::{broadcast, oneshot, watch};

use crate::Task;
use crate::servers::Address;

const PUSH_BACKLOG: usize = 4096;
const HEARTBEAT: Duration = Duration::from_secs(2);
/// A server that has not answered a ping by then is treated as gone, so the client reconnects.
const HEARTBEAT_PATIENCE: Duration = Duration::from_secs(5);

/// This client as servers know it: its name and the key it pairs with.
pub struct Identity {
    pub name: String,
    pub key: DeviceKey,
}

pub trait Duplex: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Duplex for T {}

/// One data stream: reads carry its bytes, writes are its input.
pub type DataStream = Box<dyn Duplex>;

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

/// Where each stream is opened: a fresh local socket connection, or a fresh stream of the one QUIC connection.
/// Either way each stream flows on its own, so a large file transfer never blocks a terminal's bytes.
enum Link {
    Local(PathBuf),
    Quic(quinn::Connection),
}

impl Link {
    async fn open(&self) -> std::io::Result<DataStream> {
        match self {
            Self::Local(socket) => Ok(Box::new(local::connect(socket).await?)),
            Self::Quic(connection) => {
                let (send, receive) = connection.open_bi().await?;
                Ok(Box::new(tokio::io::join(receive, send)))
            }
        }
    }
}

type Pending = Mutex<HashMap<u64, oneshot::Sender<Outcome>>>;

struct Control {
    write: tokio::sync::Mutex<WriteHalf<DataStream>>,
    pending: Pending,
    next_id: AtomicU64,
    /// Why the connection closed, once it has.
    closed: watch::Sender<Option<String>>,
    rtt: Mutex<Option<Duration>>,
}

impl Control {
    /// The first reason sticks.
    fn close(&self, reason: impl Into<String>) {
        self.closed.send_if_modified(|closed| closed.is_none() && closed.replace(reason.into()).is_none());
        lock(&self.pending).clear();
    }

    async fn call<O: Op>(&self, op: &O) -> Result<O::Ok, CallError> {
        let args = serde_json::to_value(op).context("encode args")?;
        match self.call_raw(O::NAME, args).await? {
            Outcome::Ok(value) => Ok(serde_json::from_value(value).context("decode result")?),
            Outcome::Err(error) => Err(CallError::Server(error)),
        }
    }

    async fn call_raw(&self, op: &str, args: Value) -> anyhow::Result<Outcome> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (waiter, outcome) = oneshot::channel();
        lock(&self.pending).insert(id, waiter);
        if let Some(reason) = self.closed.borrow().as_ref() {
            lock(&self.pending).remove(&id);
            return Err(anyhow!("the connection closed: {reason}"));
        }
        let request = line(&Request { id, op: op.to_string(), args });
        self.write.lock().await.write_all(request.as_bytes()).await.context("send request")?;
        outcome.await.context("connection closed before the response")
    }
}

/// Data streams already open outlive it.
pub struct Connection {
    link: Link,
    client: String,
    control: Arc<Control>,
    pushes: broadcast::Sender<Push>,
    _reader: Task,
    _heartbeat: Task,
}

/// Pings until the server stops answering, recording each round trip.
async fn heartbeat(control: Weak<Control>) {
    let mut ticks = tokio::time::interval(HEARTBEAT);
    loop {
        ticks.tick().await;
        let Some(control) = control.upgrade() else { return };
        let sent = Instant::now();
        match tokio::time::timeout(HEARTBEAT_PATIENCE, control.call(&SessionPing {})).await {
            Ok(Ok(_)) => *lock(&control.rtt) = Some(sent.elapsed()),
            Ok(Err(error)) => return control.close(format!("a heartbeat failed: {error}")),
            Err(_) => return control.close(format!("no heartbeat answer within {HEARTBEAT_PATIENCE:?}")),
        }
    }
}

async fn read_frames(read: impl AsyncRead + Unpin, control: Weak<Control>, pushes: broadcast::Sender<Push>) {
    let mut lines = BufReader::new(read).lines();
    let reason = loop {
        let frame = match lines.next_line().await {
            Ok(Some(frame)) => frame,
            Ok(None) => break "the server closed the connection".to_string(),
            Err(error) => break format!("reading from the server failed: {error}"),
        };
        let Some(control) = control.upgrade() else { return };
        match serde_json::from_str::<ServerFrame>(&frame) {
            Ok(ServerFrame::Response(response)) => {
                if let Some(waiter) = lock(&control.pending).remove(&response.id) {
                    let _caller_gone = waiter.send(response.outcome);
                }
            }
            Ok(ServerFrame::Push(push)) => {
                let _no_subscribers = pushes.send(push);
            }
            Err(error) => break format!("the server sent an unreadable frame: {error}"),
        }
    };
    if let Some(control) = control.upgrade() {
        control.close(reason);
    }
}

impl Connection {
    /// Opens the control stream and says hello. Over QUIC, the server must prove the key `address` pins.
    pub async fn connect(address: &Address, me: &Identity) -> anyhow::Result<Self> {
        let link = match address {
            Address::Local { socket } => Link::Local(socket.clone()),
            Address::Quic { endpoint, key } => Link::Quic(quic::connect(endpoint, &me.key, Some(*key)).await?),
        };
        let stream = link.open().await.with_context(|| format!("connect {address}"))?;
        let (read, write) = tokio::io::split(stream);
        let (closed, _) = watch::channel(None);
        let control = Arc::new(Control {
            write: tokio::sync::Mutex::new(write),
            pending: Mutex::default(),
            next_id: AtomicU64::new(1),
            closed,
            rtt: Mutex::default(),
        });
        let (pushes, _) = broadcast::channel(PUSH_BACKLOG);
        let reader = Task(tokio::spawn(read_frames(read, Arc::downgrade(&control), pushes.clone())));
        control.call(&SessionHello { client: me.name.clone() }).await.map_err(|error| anyhow!("{address}: {error}"))?;
        let heartbeat = Task(tokio::spawn(heartbeat(Arc::downgrade(&control))));
        Ok(Self { link, client: me.name.clone(), control, pushes, _reader: reader, _heartbeat: heartbeat })
    }

    pub fn pushes(&self) -> broadcast::Receiver<Push> {
        self.pushes.subscribe()
    }

    /// The round trip of the newest heartbeat.
    pub fn rtt(&self) -> Option<Duration> {
        *lock(&self.control.rtt)
    }

    /// Resolves to why the connection closed, once the server hangs up or stops answering heartbeats.
    pub async fn closed(&self) -> String {
        let mut closed = self.control.closed.subscribe();
        match closed.wait_for(Option::is_some).await {
            Ok(reason) => reason.clone().expect("waited for a reason"),
            Err(_) => "the connection is gone".to_string(),
        }
    }

    pub async fn call<O: Op>(&self, op: &O) -> Result<O::Ok, CallError> {
        self.control.call(op).await
    }

    pub async fn call_raw(&self, op: &str, args: Value) -> anyhow::Result<Outcome> {
        self.control.call_raw(op, args).await
    }

    pub async fn open_data_stream(&self, stream: &Stream) -> Result<DataStream, CallError> {
        let header = StreamHeader { open: stream.name(), client: self.client.clone() };
        let mut opened = self.link.open().await.context("open the stream")?;
        match octopus_proto::frames::open(&mut opened, &header).await.context("open the stream")? {
            Outcome::Ok(_) => Ok(opened),
            Outcome::Err(error) => Err(CallError::Server(error)),
        }
    }
}
