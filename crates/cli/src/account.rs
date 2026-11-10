//! The account verbs: what this device's account holds, linking and joining devices, and leaving.

use anyhow::{anyhow, bail};
use clap::Subcommand;
use octopus_client::servers::{self, Config, Servers};
use octopus_client::{Connection, Identity};
use octopus_proto::account::{Account, Change, Link};
use octopus_proto::settings::Settings;
use serde::Serialize;

use crate::commands::SERVER_HELP;
use crate::{connect_as, print};

#[derive(Subcommand)]
pub enum AccountVerb {
    /// Show this device's account: its id, its devices and the servers it lists.
    Show,
    /// Make a link another device joins this account with, held by the named server for 10 minutes.
    Link {
        #[arg(long, help = SERVER_HELP)]
        server: String,
    },
    /// Join the account a link names, bringing this device's servers along.
    Join {
        /// The link `account link` printed: octopus://<server key>?join=…&account=…&invite=…&relay=….
        #[arg(long)]
        link: String,
    },
    /// Remove another device from this account; it loses every server of the account.
    Remove {
        /// The device's name, as `account show` lists it.
        #[arg(long)]
        device: String,
    },
    /// Leave this account for a fresh one of this device's own, keeping its servers.
    Leave,
    /// Start a fresh account with no servers, as a device its account removed does.
    StartOver,
}

#[derive(Serialize)]
struct Shown {
    account: String,
    member: bool,
    devices: Vec<Device>,
    servers: Vec<servers::Server>,
}

#[derive(Serialize)]
struct Device {
    name: String,
    key: String,
    this: bool,
}

#[derive(Serialize)]
struct Linked {
    link: Link,
    expires_at: i64,
}

#[derive(Serialize)]
struct Synced {
    synced: Vec<String>,
    unreachable: Vec<String>,
}

fn shown(config: &Config, me: &Identity) -> anyhow::Result<Shown> {
    let state = me.account.state();
    let devices = state
        .members
        .iter()
        .map(|member| Device {
            name: member.name.clone(),
            key: member.key.to_string(),
            this: member.key == me.key.public(),
        })
        .collect();
    Ok(Shown {
        account: me.account.account().id().to_string(),
        member: me.in_account(),
        devices,
        servers: Servers::load(config)?.reachable(me),
    })
}

/// Syncs the account with every server this client reaches, naming the ones it could not reach.
pub(crate) async fn sync_everywhere(config: &Config, me: &Identity) -> anyhow::Result<()> {
    let (mut synced, mut unreachable) = (Vec::new(), Vec::new());
    for server in Servers::load(config)?.reachable(me) {
        match Connection::connect(&server.address, me).await {
            Ok(_) => synced.push(server.name),
            Err(error) => unreachable.push(format!("{}: {error:#}", server.name)),
        }
    }
    print(&Synced { synced, unreachable })
}

impl AccountVerb {
    pub async fn run(self) -> anyhow::Result<()> {
        let config = Config::user()?;
        let me = Identity::load(&config)?;
        match self {
            Self::Show => print(&shown(&config, &me)?),
            Self::Link { server } => {
                let connection = connect_as(&config, &me, &server).await?;
                let (invite, secret, expires_at) = me.account.update(|kept| Ok(kept.account.invite(&me.key)))?;
                connection.sync_account(&me).await?;
                let hosted = connection.server();
                let link = Link {
                    server: hosted.key,
                    relay: hosted.relay.clone(),
                    account: me.account.account().id(),
                    invite,
                    secret,
                };
                eprintln!("{}", octopus_client::qr::text(&link)?);
                print(&Linked { link, expires_at })
            }
            Self::Join { link } => {
                let link: Link = link.parse().map_err(|error: String| anyhow!(error))?;
                servers::join(&link, &me).await?;
                sync_everywhere(&config, &me).await
            }
            Self::Remove { device } => {
                let state = me.account.state();
                let named: Vec<_> = state.members.iter().filter(|member| member.name == device).collect();
                let member = match named.as_slice() {
                    [member] => member,
                    [] => bail!("no device named {device} in this account"),
                    _ => bail!("{} devices are named {device}; rename one first", named.len()),
                };
                if member.key == me.key.public() {
                    bail!("this is this device; `account leave` instead");
                }
                me.account.append(&me.key, Change::Remove { device: member.key })?;
                sync_everywhere(&config, &me).await
            }
            Self::Leave => {
                let fresh = Account::fresh(me.key.public());
                me.account.move_to(&me.key, fresh.clone(), &Settings::new(fresh.id()), &me.name(), true)?;
                sync_everywhere(&config, &me).await
            }
            Self::StartOver => {
                let fresh = Account::fresh(me.key.public());
                me.account.move_to(&me.key, fresh.clone(), &Settings::new(fresh.id()), &me.name(), false)?;
                print(&shown(&config, &me)?)
            }
        }
    }
}
