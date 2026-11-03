//! The servers this client knows, the only server data it persists, and pairing, which adds a remote one.

use std::fmt;
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
    /// A paired server on another machine, dialled by its pinned `key` through its `relay`.
    Remote { key: PublicKey, relay: RelayUrl },
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
            Self::Remote { key, relay } => write!(f, "{key} at {relay}"),
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

/// Strips `addresses` from each `remote` entry, keeping its pinned key and relay. Returns whether any entry changed.
pub(crate) fn strip_addresses(listed: &mut serde_json::Value) -> bool {
    let Some(servers) = listed.get_mut("servers").and_then(serde_json::Value::as_array_mut) else { return false };
    let mut stripped = false;
    for address in servers.iter_mut().filter_map(|server| server.get_mut("address")) {
        if address.get("path").and_then(serde_json::Value::as_str) == Some("remote") {
            stripped |= address.as_object_mut().and_then(|fields| fields.remove("addresses")).is_some();
        }
    }
    stripped
}

impl Servers {
    pub fn load(config: &Config) -> anyhow::Result<Self> {
        let path = config.file(SERVERS_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let mut listed: serde_json::Value =
                    serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
                let stripped = strip_addresses(&mut listed);
                let servers: Self =
                    serde_json::from_value(listed).with_context(|| format!("parse {}", path.display()))?;
                servers
                    .servers
                    .iter()
                    .try_for_each(Server::validate)
                    .with_context(|| format!("check {}", path.display()))?;
                if stripped {
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
        Ok(Self::new(machine_name()?, key))
    }
}

/// The name the user gave this computer, which names both its server and its client. macOS's hostname follows the
/// network it joins, so it names neither there.
pub fn machine_name() -> anyhow::Result<String> {
    #[cfg(target_os = "macos")]
    {
        const UNNAMED: &str = "this Mac has no name; set one in System Settings > General > Sharing";
        let output = std::process::Command::new("/usr/sbin/scutil").args(["--get", "ComputerName"]).output();
        let output = output.context("run scutil")?;
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !output.status.success() || name.is_empty() {
            bail!(UNNAMED);
        }
        Ok(name)
    }
    #[cfg(not(target_os = "macos"))]
    Ok(gethostname::gethostname().to_string_lossy().into_owned())
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
    let Invite { code, server, relay } = invite;
    let connection = me.dialer().await?.connect(&me.key, *server, relay).await?;
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
    let address = Address::Remote { key: *server, relay: relay.clone() };
    let server = Server { name: paired.server, address };
    server.validate()?;
    Ok(server)
}
