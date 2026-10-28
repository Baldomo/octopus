//! The servers this client knows, the only server data it persists, and pairing, which adds a remote one.

use std::fmt;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use octopus_proto::device::{DeviceKey, PublicKey};
use octopus_proto::ops::{Paired, PairingPair};
use octopus_proto::pairing::{Invite, Side};
use octopus_proto::remote::{self, RelayUrl};
use octopus_proto::{Op, Outcome, Request, Response, line};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use crate::connection::Identity;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "path", rename_all = "snake_case", deny_unknown_fields)]
pub enum Address {
    /// A server on this machine, which admits this OS user with no pairing.
    Local { socket: PathBuf },
    /// A paired server on another machine, dialled by its pinned `key` at its direct `addresses` and through its
    /// `relay` at once.
    Remote { key: PublicKey, relay: RelayUrl, addresses: Vec<SocketAddr> },
}

impl Address {
    /// The same socket, or the same server key whatever relay it names.
    pub fn same_place(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Local { socket }, Self::Local { socket: other }) => socket == other,
            (Self::Remote { key, .. }, Self::Remote { key: other, .. }) => key == other,
            _ => false,
        }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local { socket } => write!(f, "{}", socket.display()),
            Self::Remote { key, relay, .. } => write!(f, "{key} at {relay}"),
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

/// The directory this client keeps its files in.
#[derive(Debug, Clone)]
pub struct Config(PathBuf);

impl Config {
    /// This OS user's octopus config directory, which every desktop client and the CLI share.
    pub fn user() -> anyhow::Result<Self> {
        Ok(Self(dirs::config_dir().ok_or_else(|| anyhow!("no config directory on this platform"))?.join("octopus")))
    }

    pub fn at(dir: PathBuf) -> Self {
        Self(dir)
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

pub fn create_parent(path: &std::path::Path) -> anyhow::Result<()> {
    let parent = path.parent().expect("a config file lives in a directory");
    std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))
}

/// Rewrites each entry paired over QUIC, `{"path":"quic","endpoint":"<host:port>","key"}`, to dial the same key at
/// the addresses the endpoint resolves to and through n0's relay, which every server installed then is homed on once
/// upgraded. The key stays, so neither side pairs again. Returns whether any entry changed.
pub(crate) fn migrate_quic_entries(listed: &mut serde_json::Value) -> anyhow::Result<bool> {
    let Some(servers) = listed.get_mut("servers").and_then(serde_json::Value::as_array_mut) else { return Ok(false) };
    let mut migrated = false;
    for server in servers.iter_mut() {
        let name = server.get("name").and_then(serde_json::Value::as_str).unwrap_or("a server").to_string();
        let Some(address) = server.get_mut("address") else { continue };
        if address.get("path").and_then(serde_json::Value::as_str) != Some("quic") {
            continue;
        }
        let (Some(endpoint), Some(key)) =
            (address.get("endpoint").and_then(serde_json::Value::as_str), address.get("key"))
        else {
            bail!("{name} is listed over QUIC without both an endpoint and a key; pair it again with an invite");
        };
        let addresses: Vec<String> = match endpoint.to_socket_addrs() {
            Ok(resolved) => resolved.map(|address| address.to_string()).collect(),
            Err(error) => {
                eprintln!(
                    "octopus: {name} keeps only n0's relay: its QUIC endpoint {endpoint} does not resolve: {error}"
                );
                Vec::new()
            }
        };
        *address = serde_json::json!({
            "path": "remote", "key": key, "relay": remote::N0_EU_RELAY, "addresses": addresses,
        });
        migrated = true;
    }
    Ok(migrated)
}

impl Servers {
    pub fn load(config: &Config) -> anyhow::Result<Self> {
        let path = config.file(SERVERS_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let mut listed: serde_json::Value =
                    serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
                let migrated =
                    migrate_quic_entries(&mut listed).with_context(|| format!("migrate {}", path.display()))?;
                let servers: Self =
                    serde_json::from_value(listed).with_context(|| format!("parse {}", path.display()))?;
                servers
                    .servers
                    .iter()
                    .try_for_each(Server::validate)
                    .with_context(|| format!("check {}", path.display()))?;
                if migrated {
                    servers.save(config)?;
                }
                Ok(servers)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
        }
    }

    pub fn save(&self, config: &Config) -> anyhow::Result<()> {
        let path = config.file(SERVERS_FILE);
        create_parent(&path)?;
        std::fs::write(&path, serde_json::to_string_pretty(self)?).with_context(|| format!("write {}", path.display()))
    }

    /// Points the listed remote server `name` at the direct `addresses` it reported, saving the list when they
    /// changed, so a server whose network moved stays reachable with no relay.
    pub fn refresh(config: &Config, name: &str, addresses: &[SocketAddr]) -> anyhow::Result<()> {
        let mut servers = Self::load(config)?;
        let Some(Address::Remote { addresses: listed, .. }) =
            servers.servers.iter_mut().find(|server| server.name == name).map(|server| &mut server.address)
        else {
            return Ok(());
        };
        if listed.as_slice() == addresses {
            return Ok(());
        }
        *listed = addresses.to_vec();
        servers.save(config)
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
    pub fn remember(config: &Config, server: Server) -> anyhow::Result<Self> {
        let mut servers = Self::load(config)?;
        servers.add(server)?;
        servers.save(config)?;
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
    pub fn load(config: &Config) -> anyhow::Result<Self> {
        let path = config.file(DEVICE_KEY_FILE);
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
        Ok(Self::new(hostname(), key))
    }
}

pub fn hostname() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?.write_all(bytes)
}

/// Pairs with the server `invite` names and returns the entry that pins its key. Both sides prove the code over
/// their own handshake's keys, so a peer in the middle gets pinned by neither.
pub async fn pair(invite: &Invite, me: &Identity) -> anyhow::Result<Server> {
    let server = invite.server;
    tokio::time::timeout(PAIRING_PATIENCE, exchange(invite, me))
        .await
        .map_err(|_| anyhow!("{server} did not finish pairing within {PAIRING_PATIENCE:?}"))?
}

async fn exchange(invite: &Invite, me: &Identity) -> anyhow::Result<Server> {
    let Invite { code, server, relay, addresses } = invite;
    let connection = me.dialer().await?.connect(&me.key, *server, relay, addresses).await?;
    let client = me.key.public();
    let op = PairingPair { client: me.name(), proof: code.prove(Side::Client, server, &client) };
    let request = Request { id: 1, op: PairingPair::NAME.to_string(), args: serde_json::to_value(op)? };
    let (mut send, receive) = connection.open_bi().await.context("open the pairing stream")?;
    send.write_all(line(&request).as_bytes()).await.context("send the pairing request")?;
    let mut reply = String::new();
    BufReader::new(receive.take(PAIRING_REPLY_BYTES)).read_line(&mut reply).await.context("read the pairing reply")?;
    connection.close(remote::DONE, b"paired");
    let response: Response = serde_json::from_str(&reply).with_context(|| format!("{server} did not answer"))?;
    if response.id != request.id {
        bail!("{server} answered request {}, not {}", response.id, request.id);
    }
    let paired: Paired = match response.outcome {
        Outcome::Ok(value) => serde_json::from_value(value).context("decode the pairing reply")?,
        Outcome::Err(error) => bail!("{server} refused to pair: {error}"),
    };
    if !code.proves(&paired.proof, Side::Server, server, &client) {
        bail!("{server} did not prove it holds the code, so it was not pinned");
    }
    let address = Address::Remote { key: *server, relay: relay.clone(), addresses: addresses.clone() };
    let server = Server { name: paired.server, address };
    server.validate()?;
    Ok(server)
}
