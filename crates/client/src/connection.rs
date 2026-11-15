//! One connection from this client to one server, over the local socket or iroh with the same protocol on both.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use octopus_proto::account::Change;
use octopus_proto::device::DeviceKey;
use octopus_proto::ops::{AccountSync, Hosted, SessionHello, SessionPing};
use octopus_proto::remote::{Connection as RemoteConnection, Dialer};
use octopus_proto::types::{LockHolder, ReadDir, Stream, TaskId};
use octopus_proto::{Error, FileSize, Op, Outcome, Push, Request, ServerFrame, StreamHeader, line, local, lock};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, WriteHalf};
use tokio::sync::{broadcast, oneshot, watch};

use crate::Task;
use crate::account::Book;
use crate::servers::Address;

const PUSH_BACKLOG: usize = 4096;
const HEARTBEAT: Duration = Duration::from_secs(2);
/// A server that has not answered a ping by then is treated as gone, so the client reconnects.
const HEARTBEAT_PATIENCE: Duration = Duration::from_secs(5);

/// This client as servers know it: its name, the key it pairs with, its account, and the iroh endpoint new dials go
/// out from, bound on the first dial after each `forget_dialer`.
pub struct Identity {
    name: Mutex<String>,
    pub key: DeviceKey,
    pub account: Book,
    dialer: tokio::sync::Mutex<Option<Dialer>>,
}

impl Identity {
    /// Names this device in its account under `name` when the account names it otherwise.
    pub fn new(name: String, key: DeviceKey, account: Book) -> anyhow::Result<Self> {
        let me = Self { name: Mutex::new(String::new()), key, account, dialer: tokio::sync::Mutex::default() };
        me.rename(name)?;
        Ok(me)
    }

    pub fn name(&self) -> String {
        lock(&self.name).clone()
    }

    /// Names this client on every connection and pairing it opens from now on, and in its account.
    pub fn rename(&self, name: String) -> anyhow::Result<()> {
        let state = self.account.state();
        let named = state.members.iter().find(|member| member.key == self.key.public());
        if named.is_some_and(|member| member.name != name) {
            self.account.append(&self.key, Change::Name { name: name.clone() })?;
        }
        *lock(&self.name) = name;
        Ok(())
    }

    /// Whether this device's account still holds it.
    pub fn in_account(&self) -> bool {
        self.account.state().is_member(&self.key.public())
    }

    /// Lists the server on this machine in the account, as this device's, unless it is listed so already under
    /// whatever name the account gives it.
    fn host(&self, server: &Hosted) -> anyhow::Result<()> {
        let listed = self.account.state().servers.into_iter().any(|listed| {
            listed.key == server.key && listed.relay == server.relay && listed.host == Some(self.key.public())
        });
        if !listed && self.in_account() {
            let (relay, name) = (server.relay.clone(), server.name.clone());
            let change = Change::ListServer { server: server.key, relay, name, host: Some(self.key.public()) };
            self.account.append(&self.key, change)?;
        }
        Ok(())
    }

    /// Whether this client holds the lock `holder` holds on the server at `address`: a server it is paired with
    /// knows it by its key, and the server on its local socket by its name.
    pub fn holds(&self, holder: &LockHolder, address: &Address) -> bool {
        match (&holder.key, address) {
            (Some(key), _) => *key == self.key.public(),
            (None, Address::Local { .. }) => holder.name == self.name(),
            (None, Address::Remote { .. }) => false,
        }
    }

    pub async fn dialer(&self) -> std::io::Result<Dialer> {
        let mut dialer = self.dialer.lock().await;
        if let Some(bound) = &*dialer {
            return Ok(bound.clone());
        }
        let bound = Dialer::bind().await?;
        *dialer = Some(bound.clone());
        Ok(bound)
    }

    pub async fn forget_dialer(&self) {
        *self.dialer.lock().await = None;
    }
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

/// Where each stream is opened: a fresh local socket connection, or a fresh stream of the one iroh connection.
/// Either way each stream flows on its own, so a large file transfer never blocks a terminal's bytes. A remote link
/// holds its endpoint, whose last drop ends every connection dialled from it.
enum Link {
    Local(PathBuf),
    Remote { connection: RemoteConnection, _dialer: Dialer },
}

impl Link {
    async fn open(&self) -> std::io::Result<DataStream> {
        match self {
            Self::Local(socket) => Ok(Box::new(local::connect(socket).await?)),
            Self::Remote { connection, .. } => {
                let (send, receive) = connection.open_bi().await?;
                Ok(Box::new(tokio::io::join(receive, send)))
            }
        }
    }
}

struct Control {
    write: tokio::sync::Mutex<WriteHalf<DataStream>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Outcome>>>,
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

    /// Records the round trip of a ping the server answers within `patience`, and closes the connection otherwise;
    /// resolves to whether it answered.
    async fn ping(&self, patience: Duration) -> bool {
        let sent = Instant::now();
        match tokio::time::timeout(patience, self.call(&SessionPing {})).await {
            Ok(Ok(_)) => {
                *lock(&self.rtt) = Some(sent.elapsed());
                true
            }
            Ok(Err(error)) => {
                self.close(format!("a ping failed: {error}"));
                false
            }
            Err(_) => {
                self.close(format!("no ping answer within {patience:?}"));
                false
            }
        }
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
        outcome.await.map_err(|_| {
            let reason = self.closed.borrow().clone().expect("close records its reason before dropping the waiters");
            anyhow!("the connection closed before the response: {reason}")
        })
    }
}

/// Sends the account and the accounts this device left, and keeps what the server merged; the server on this
/// machine is listed in the account as this device's, which takes a second round.
async fn sync(control: &Control, me: &Identity, local: bool) -> anyhow::Result<Hosted> {
    let server = round(control, me).await?;
    if local {
        let before = me.account.account();
        me.host(&server)?;
        if me.account.account() != before {
            round(control, me).await?;
        }
    }
    Ok(server)
}

/// One `account.sync`: sends what this device keeps and keeps what the server merged.
async fn round(control: &Control, me: &Identity) -> anyhow::Result<Hosted> {
    let kept = me.account.kept();
    let settings = me.account.settings();
    let synced = control.call(&AccountSync { account: kept.account, former: kept.former, settings }).await?;
    me.account.absorb(&synced.account, &synced.settings)?;
    Ok(synced.server)
}

/// Data streams already open outlive it while its endpoint stays `Identity`'s or another connection's.
pub struct Connection {
    link: Link,
    client: String,
    local: bool,
    server: Hosted,
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
        if !control.ping(HEARTBEAT_PATIENCE).await {
            return;
        }
    }
}

async fn read_frames(read: impl AsyncRead + Unpin, control: Weak<Control>, pushes: broadcast::Sender<Push>) {
    let mut lines = BufReader::new(read).lines();
    let reason = loop {
        let frame = match lines.next_line().await {
            Ok(Some(frame)) => frame,
            Ok(None) => break "the server closed the connection".to_string(),
            Err(error) => break format!("reading from the server failed: {:#}", anyhow::Error::from(error)),
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
    /// Opens the control stream and says hello. Over iroh, only the holder of the key `address` pins answers.
    pub async fn connect(address: &Address, me: &Identity) -> anyhow::Result<Self> {
        let link = match address {
            Address::Local { socket } => Link::Local(socket.clone()),
            Address::Remote { key, relay } => {
                let dialer = me.dialer().await?;
                let connection = dialer.connect(&me.key, *key, relay).await?;
                Link::Remote { connection, _dialer: dialer }
            }
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
        let client = me.name();
        let hello = SessionHello { client: client.clone() };
        control.call(&hello).await.map_err(|error| anyhow!("{address}: {error}"))?;
        let local = matches!(address, Address::Local { .. });
        let server = sync(&control, me, local).await.map_err(|error| anyhow!("{address}: {error:#}"))?;
        let heartbeat = Task(tokio::spawn(heartbeat(Arc::downgrade(&control))));
        Ok(Self { link, client, local, server, control, pushes, _reader: reader, _heartbeat: heartbeat })
    }

    /// The server as it names itself, with its key and relay.
    pub fn server(&self) -> &Hosted {
        &self.server
    }

    /// Merges this device's account with the server's log of it, both ways.
    pub async fn sync_account(&self, me: &Identity) -> anyhow::Result<()> {
        sync(&self.control, me, self.local).await.map(drop)
    }

    pub fn pushes(&self) -> broadcast::Receiver<Push> {
        self.pushes.subscribe()
    }

    /// Closes the connection unless the server answers a ping within `patience`.
    pub async fn check(&self, patience: Duration) {
        self.control.ping(patience).await;
    }

    pub fn is_closed(&self) -> bool {
        self.control.closed.borrow().is_some()
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
        self.open(stream).await.map(|(_, opened)| opened)
    }

    /// Up to `len` bytes of the file `path` names under `dir` from byte `start`, beside the file's size.
    pub async fn read(&self, dir: ReadDir, path: String, start: u64, len: u64) -> Result<(u64, DataStream), CallError> {
        let (reply, opened) = self.open(&Stream::Read { dir, path, start, len }).await?;
        let size: FileSize = serde_json::from_value(reply).context("hear the file's size")?;
        Ok((size.size, opened))
    }

    async fn open(&self, stream: &Stream) -> Result<(Value, DataStream), CallError> {
        let header = StreamHeader { open: stream.clone(), client: self.client.clone() };
        let mut opened = self.link.open().await.context("open the stream")?;
        match octopus_proto::frames::open(&mut opened, &header).await.context("open the stream")? {
            Outcome::Ok(reply) => Ok((reply, opened)),
            Outcome::Err(error) => Err(CallError::Server(error)),
        }
    }

    /// Sends `content` as a new file of a task's directory, resolving once the server has landed it.
    pub async fn upload(
        &self,
        task: TaskId,
        path: String,
        bytes: u64,
        mut content: impl AsyncRead + Unpin,
    ) -> Result<(), CallError> {
        let mut opened = self.open_data_stream(&Stream::Upload { task, bytes, path }).await?;
        tokio::io::copy(&mut content, &mut opened).await.context("send the upload")?;
        opened.shutdown().await.context("finish the upload")?;
        match octopus_proto::frames::read_outcome(&mut opened).await.context("hear the upload land")? {
            Outcome::Ok(_) => Ok(()),
            Outcome::Err(error) => Err(CallError::Server(error)),
        }
    }
}
