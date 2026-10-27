//! The servers this client knows, the only server data it persists, and pairing, which adds a remote one.

use std::fmt;
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use octopus_proto::device::{DeviceKey, PublicKey};
use octopus_proto::ops::{Paired, PairingPair};
use octopus_proto::pairing::{Code, Side};
use octopus_proto::{Op, Outcome, Request, Response, line, quic};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use crate::connection::Identity;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "path", rename_all = "snake_case", deny_unknown_fields)]
pub enum Address {
    /// A server on this machine, which admits this OS user with no pairing.
    Local { socket: PathBuf },
    /// A paired server on another machine at `endpoint`, `host:port`, which must prove `key`.
    Quic { endpoint: String, key: PublicKey },
}

impl Address {
    /// Whether the server's ports are another machine's. A QUIC endpoint on a loopback host is this machine, and a
    /// tunnel to it would dial its own listener.
    pub fn is_another_machine(&self) -> bool {
        match self {
            Self::Local { .. } => false,
            Self::Quic { endpoint, .. } => {
                let host = endpoint.rsplit_once(':').map_or(endpoint.as_str(), |(host, _)| host);
                let host = host.trim_start_matches('[').trim_end_matches(']');
                !(host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()))
            }
        }
    }
}

impl Address {
    /// The same socket, or the same QUIC endpoint whatever key it pins.
    pub fn same_place(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Local { socket }, Self::Local { socket: other }) => socket == other,
            (Self::Quic { endpoint, .. }, Self::Quic { endpoint: other, .. }) => endpoint == other,
            _ => false,
        }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local { socket } => write!(f, "{}", socket.display()),
            Self::Quic { endpoint, .. } => f.write_str(endpoint),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Server {
    pub name: String,
    pub address: Address,
}

impl Server {
    /// A client keys a server's tasks as `<name>/<task>`, and a relative socket would depend on where it started.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.name.is_empty() || self.name.contains('/') {
            bail!("a server name is non-empty and has no '/': {:?}", self.name);
        }
        match &self.address {
            Address::Local { socket } if !socket.is_absolute() => {
                bail!("the socket of {} must be an absolute path: {}", self.name, socket.display())
            }
            Address::Quic { endpoint, .. }
                if endpoint.rsplit_once(':').is_none_or(|(_, port)| port.parse::<u16>().is_err()) =>
            {
                bail!("the endpoint of {} must be host:port: {endpoint}", self.name)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Servers {
    pub servers: Vec<Server>,
}

const SERVERS_FILE: &str = "servers.json";
const DEVICE_KEY_FILE: &str = "device.key";
/// A pairing reply is one short line, from a server not yet pinned.
const PAIRING_REPLY_BYTES: u64 = 4096;
const PAIRING_PATIENCE: Duration = Duration::from_secs(10);

/// Where this client keeps `file` among its config.
pub fn config(file: &str) -> anyhow::Result<PathBuf> {
    Ok(dirs::config_dir().ok_or_else(|| anyhow!("no config directory on this platform"))?.join("octopus").join(file))
}

pub fn create_parent(path: &std::path::Path) -> anyhow::Result<()> {
    let parent = path.parent().expect("a config file lives in a directory");
    std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))
}

impl Servers {
    pub fn load() -> anyhow::Result<Self> {
        let path = config(SERVERS_FILE)?;
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let servers: Self = serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
                servers
                    .servers
                    .iter()
                    .try_for_each(Server::validate)
                    .with_context(|| format!("check {}", path.display()))?;
                Ok(servers)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = config(SERVERS_FILE)?;
        create_parent(&path)?;
        std::fs::write(&path, serde_json::to_string_pretty(self)?).with_context(|| format!("write {}", path.display()))
    }

    /// Replaces a listed server only at the same place, as re-pairing does to pin a new key; a name listed for
    /// another place is refused.
    pub fn add(&mut self, server: Server) -> anyhow::Result<()> {
        server.validate()?;
        if let Some(known) = self.servers.iter().find(|known| known.name == server.name)
            && !known.address.same_place(&server.address)
        {
            bail!("a server named {} is already listed at {}", server.name, known.address);
        }
        self.servers.retain(|known| known.name != server.name);
        self.servers.push(server);
        Ok(())
    }

    /// Lists `server` in the saved list.
    pub fn remember(server: Server) -> anyhow::Result<Self> {
        let mut servers = Self::load()?;
        servers.add(server)?;
        servers.save()?;
        Ok(servers)
    }

    /// Lists the server named `from` as `to`, which no other listed server may be called.
    pub fn rename(&mut self, from: &str, to: &str) -> anyhow::Result<Server> {
        if from != to && self.servers.iter().any(|known| known.name == to) {
            bail!("a server named {to} is already listed");
        }
        let renamed = Server { name: to.to_string(), address: self.get(from)?.address.clone() };
        renamed.validate()?;
        let index = self.servers.iter().position(|known| known.name == from).expect("get found it");
        self.servers[index] = renamed.clone();
        Ok(renamed)
    }

    /// Drops the server and its pin from this client's list.
    pub fn forget(&mut self, name: &str) -> anyhow::Result<()> {
        self.get(name)?;
        self.servers.retain(|known| known.name != name);
        Ok(())
    }

    pub fn get(&self, name: &str) -> anyhow::Result<&Server> {
        self.servers.iter().find(|server| server.name == name).ok_or_else(|| {
            anyhow!("no server named {name}; pair it with `octopus pairing accept`, or add a local one with `octopus server add`")
        })
    }
}

impl Identity {
    /// This machine's name, and the device key made on the first load and kept beside the server list.
    pub fn load() -> anyhow::Result<Self> {
        let path = config(DEVICE_KEY_FILE)?;
        let key = match std::fs::read(&path) {
            Ok(pkcs8) => DeviceKey::from_pkcs8(&pkcs8).map_err(|error| anyhow!("{}: {error}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let key = DeviceKey::generate();
                create_parent(&path)?;
                write_private(&path, key.pkcs8()).with_context(|| format!("write {}", path.display()))?;
                key
            }
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        Ok(Self { name: hostname(), key })
    }
}

pub fn hostname() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

#[cfg(unix)]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?.write_all(bytes)
}

/// The config directory is the user's own on Windows.
#[cfg(windows)]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::OpenOptions::new().write(true).create_new(true).open(path)?.write_all(bytes)
}

/// Pairs with the server at `endpoint` using a code it offered, and returns the entry that pins its key. Both sides
/// prove the code over their own handshake's keys, so a peer in the middle gets pinned by neither.
pub async fn pair(endpoint: &str, code: &Code, me: &Identity) -> anyhow::Result<Server> {
    tokio::time::timeout(PAIRING_PATIENCE, exchange(endpoint, code, me))
        .await
        .map_err(|_| anyhow!("{endpoint} did not finish pairing within {PAIRING_PATIENCE:?}"))?
}

async fn exchange(endpoint: &str, code: &Code, me: &Identity) -> anyhow::Result<Server> {
    let connection = quic::connect(endpoint, &me.key, None).await?;
    let (server, client) = (quic::peer_key(&connection), me.key.public());
    let op = PairingPair { client: me.name.clone(), proof: code.prove(Side::Client, &server, &client) };
    let request = Request { id: 1, op: PairingPair::NAME.to_string(), args: serde_json::to_value(op)? };
    let (mut send, receive) = connection.open_bi().await.context("open the pairing stream")?;
    send.write_all(line(&request).as_bytes()).await.context("send the pairing request")?;
    let mut reply = String::new();
    BufReader::new(receive.take(PAIRING_REPLY_BYTES)).read_line(&mut reply).await.context("read the pairing reply")?;
    connection.close(quic::DONE, b"paired");
    let response: Response = serde_json::from_str(&reply).with_context(|| format!("{endpoint} did not answer"))?;
    if response.id != request.id {
        bail!("{endpoint} answered request {}, not {}", response.id, request.id);
    }
    let paired: Paired = match response.outcome {
        Outcome::Ok(value) => serde_json::from_value(value).context("decode the pairing reply")?,
        Outcome::Err(error) => bail!("{endpoint} refused to pair: {error}"),
    };
    if !code.proves(&paired.proof, Side::Server, &server, &client) {
        bail!("{endpoint} did not prove it holds the code, so its key {server} was not pinned");
    }
    let server = Server { name: paired.server, address: Address::Quic { endpoint: endpoint.to_string(), key: server } };
    server.validate()?;
    Ok(server)
}
