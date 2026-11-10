//! The servers this client knows, the only server data it persists, and pairing, which adds a remote one.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use octopus_proto::account::{Account, Change, Link};
use octopus_proto::device::{DeviceKey, PublicKey};
use octopus_proto::ops::{AccountJoin, PairingPair};
use octopus_proto::pairing::{Invite, Side};
use octopus_proto::remote::{self, RelayUrl};
use octopus_proto::{Op, Outcome, Request, Response, line};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use crate::account::{self, Book};
use crate::connection::Identity;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "path", rename_all = "snake_case", deny_unknown_fields)]
pub enum Address {
    /// A server on this machine, which admits this OS user with no pairing.
    Local { socket: PathBuf },
    /// A paired server on another machine, dialled by its pinned `key` through its `relay`.
    Remote { key: PublicKey, relay: RelayUrl },
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
/// A reply to a device the server does not know yet carries at most an account.
const PAIRING_REPLY_BYTES: u64 = 4 << 20;
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

/// The file's text, or none when there is no such file.
pub fn read_text(path: &std::path::Path) -> anyhow::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

pub fn write_text(path: &std::path::Path, text: &str) -> anyhow::Result<()> {
    create_parent(path)?;
    std::fs::write(path, text).with_context(|| format!("write {}", path.display()))
}

pub fn create_parent(path: &std::path::Path) -> anyhow::Result<()> {
    let parent = path.parent().expect("a config file lives in a directory");
    std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))
}

/// Takes each `remote` entry out of the list, which kept paired servers before accounts did: its name, key and relay.
pub(crate) fn take_remote(listed: &mut serde_json::Value) -> anyhow::Result<Vec<(String, PublicKey, RelayUrl)>> {
    let Some(servers) = listed.get_mut("servers").and_then(serde_json::Value::as_array_mut) else { return Ok(vec![]) };
    let is_remote = |server: &serde_json::Value| server["address"]["path"].as_str() == Some("remote");
    let taken = servers
        .iter()
        .filter(|server| is_remote(server))
        .map(|server| {
            let name = server["name"].as_str().context("a listed server has a name")?.to_string();
            let key = serde_json::from_value(server["address"]["key"].clone()).context("a remote server's key")?;
            let relay =
                serde_json::from_value(server["address"]["relay"].clone()).context("a remote server's relay")?;
            Ok((name, key, relay))
        })
        .collect::<anyhow::Result<_>>()?;
    servers.retain(|server| !is_remote(server));
    Ok(taken)
}

impl Servers {
    /// The servers on this machine.
    pub fn load(config: &Config) -> anyhow::Result<Self> {
        let path = config.file(SERVERS_FILE);
        let Some(text) = read_text(&path)? else { return Ok(Self::default()) };
        let servers: Self = serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        servers.servers.iter().try_for_each(Server::validate).with_context(|| format!("check {}", path.display()))?;
        Ok(servers)
    }

    /// Every server this client reaches: the ones on this machine, then every one `me`'s account lists but those on
    /// this machine, each under a name no earlier one takes.
    pub fn reachable(&self, me: &Identity) -> Vec<Server> {
        let mut reachable = self.servers.clone();
        let mine = me.key.public();
        for listed in me.account.state().servers.into_iter().filter(|listed| listed.host != Some(mine)) {
            let mut name = listed.name.replace('/', "-");
            if name.is_empty() || reachable.iter().any(|known| known.name == name) {
                name = format!("{name}-{}", &listed.key.hex()[..4]);
            }
            reachable.push(Server { name, address: Address::Remote { key: listed.key, relay: listed.relay } });
        }
        reachable
    }

    /// The server this client reaches by `name`.
    pub fn reach(&self, me: &Identity, name: &str) -> anyhow::Result<Server> {
        self.reachable(me).into_iter().find(|known| known.name == name).ok_or_else(|| no_server(name))
    }

    pub fn save(&self, config: &Config) -> anyhow::Result<()> {
        write_text(&config.file(SERVERS_FILE), &serde_json::to_string_pretty(self)?)
    }

    /// Refuses a name already listed at another address.
    pub fn add(&mut self, server: Server) -> anyhow::Result<()> {
        server.validate()?;
        if let Some(known) = self.servers.iter().find(|known| known.name == server.name)
            && known.address != server.address
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
        Self::open(machine_name()?, key, config)
    }

    /// This device under `name`, with the account kept in `config`, to which a server list that still holds paired
    /// servers gives them.
    pub fn open(name: String, key: DeviceKey, config: &Config) -> anyhow::Result<Self> {
        let account = Book::load(config, &key)?;
        let me = Self::new(name, key, account)?;
        let path = config.file(SERVERS_FILE);
        let Some(text) = read_text(&path)? else { return Ok(me) };
        let mut listed: serde_json::Value =
            serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        let paired = take_remote(&mut listed).with_context(|| format!("parse {}", path.display()))?;
        if !paired.is_empty() {
            for (name, server, relay) in paired {
                me.account.append(&me.key, Change::ListServer { server, relay, name, host: None })?;
            }
            write_text(&path, &serde_json::to_string_pretty(&listed)?)?;
        }
        Ok(me)
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

/// Pairs with the server `invite` names and lists it in the account. Both sides prove the code over their own
/// handshake's keys, so a peer in the middle gets pinned by neither.
pub async fn pair(invite: &Invite, me: &Identity) -> anyhow::Result<Server> {
    let Invite { code, server, relay } = invite;
    let client = me.key.public();
    let op = PairingPair {
        client: me.name(),
        proof: code.prove(Side::Client, server, &client),
        account: me.account.account(),
    };
    let paired = unpaired_request(me, *server, relay, op)
        .await
        .map_err(|error| anyhow!("{server} refused to pair: {error:#}"))?;
    if !code.proves(&paired.proof, Side::Server, server, &client) {
        bail!("{server} did not prove it holds the code, so it was not pinned");
    }
    let listed =
        Server { name: paired.server.clone(), address: Address::Remote { key: *server, relay: relay.clone() } };
    listed.validate()?;
    let change = Change::ListServer { server: *server, relay: relay.clone(), name: paired.server, host: None };
    me.account.append(&me.key, change)?;
    Ok(listed)
}

/// Joins the account `link` names through the server holding its invite, and moves this device to it with the
/// servers its old account listed.
pub async fn join(link: &Link, me: &Identity) -> anyhow::Result<()> {
    if me.account.account().id() == link.account {
        bail!("Already in this account.");
    }
    let join = Account::join(link.account, link.invite, &link.secret, &me.key, me.name());
    let op = AccountJoin { account: link.account, join };
    let synced = unpaired_request(me, link.server, &link.relay, op)
        .await
        .map_err(|error| anyhow!("{} refused the link: {error:#}", link.server))?;
    me.account.move_to(&me.key, synced.account, &synced.settings, &me.name(), true)
}

/// Sends `op` as the one request of a connection to a server that does not know this device yet, and reads its one
/// reply.
async fn unpaired_request<O: Op>(me: &Identity, server: PublicKey, relay: &RelayUrl, op: O) -> anyhow::Result<O::Ok> {
    tokio::time::timeout(PAIRING_PATIENCE, async {
        // A redial can forget this endpoint while the reply is on its way, and its last drop ends the connection.
        let dialer = me.dialer().await?;
        let connection = dialer.connect(&me.key, server, relay).await?;
        let request = Request { id: 1, op: O::NAME.to_string(), args: serde_json::to_value(op)? };
        let (mut send, receive) = connection.open_bi().await.context("open the stream")?;
        send.write_all(line(&request).as_bytes()).await.context("send the request")?;
        let mut reply = String::new();
        BufReader::new(receive.take(PAIRING_REPLY_BYTES)).read_line(&mut reply).await.context("read the reply")?;
        connection.close(remote::DONE, b"done");
        let response: Response = serde_json::from_str(&reply).with_context(|| format!("{server} did not answer"))?;
        if response.id != request.id {
            bail!("{server} answered request {}, not {}", response.id, request.id);
        }
        match response.outcome {
            Outcome::Ok(value) => serde_json::from_value(value).context("decode the reply"),
            Outcome::Err(error) => bail!("{error}"),
        }
    })
    .await
    .map_err(|_| anyhow!("{server} did not answer within {PAIRING_PATIENCE:?}"))?
}

/// Renames a server this client reaches: in this machine's list for one on it, else in the account.
pub fn rename(config: &Config, me: &Identity, from: &str, to: &str) -> anyhow::Result<()> {
    let mut locals = Servers::load(config)?;
    if from != to && locals.reach(me, to).is_ok() {
        bail!("a server named {to} is already listed");
    }
    let server = locals.reach(me, from)?;
    Server { name: to.to_string(), address: server.address.clone() }.validate()?;
    match server.address {
        Address::Local { .. } => {
            let known = locals.servers.iter_mut().find(|known| known.name == from).expect("reach found it here");
            known.name = to.to_string();
            locals.save(config)
        }
        Address::Remote { key, .. } => {
            let listed = me
                .account
                .state()
                .servers
                .into_iter()
                .find(|listed| listed.key == key)
                .ok_or_else(|| no_server(from))?;
            me.account.append(&me.key, account::relisting(&listed, to))
        }
    }
}

/// Forgets a server this client reaches: from this machine's list for one on it, else from the account.
pub fn forget(config: &Config, me: &Identity, name: &str) -> anyhow::Result<()> {
    let mut locals = Servers::load(config)?;
    let server = locals.reach(me, name)?;
    match server.address {
        Address::Local { .. } => {
            locals.servers.retain(|known| known.name != name);
            locals.save(config)
        }
        Address::Remote { key, .. } => me.account.append(&me.key, Change::ForgetServer { server: key }),
    }
}

fn no_server(name: &str) -> anyhow::Error {
    anyhow!("no server named {name}; `octopus account show` lists the ones this client reaches")
}
