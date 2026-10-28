//! Pairing from the client menu: the phone takes its name on its first pairing and keeps it for every later one.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context;
use octopus_client::servers::{Config, create_parent};
use octopus_proto::pairing::Invite;
use octopus_shell::hub::Hub;
use tauri::State;

const NAME_FILE: &str = "name";

pub struct Phone {
    config: Config,
    named: AtomicBool,
}

impl Phone {
    /// The phone and the name it is known by: the one kept at its first pairing, else the one the OS reports.
    pub fn load(config: Config, reported: impl FnOnce() -> anyhow::Result<String>) -> anyhow::Result<(Self, String)> {
        let path = config.file(NAME_FILE);
        let (named, name) = match std::fs::read_to_string(&path) {
            Ok(name) => (true, name),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (false, reported()?),
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        Ok((Self { config, named: AtomicBool::new(named) }, name))
    }

    fn keep_name(&self, name: &str) -> anyhow::Result<()> {
        let path = self.config.file(NAME_FILE);
        create_parent(&path)?;
        std::fs::write(&path, name).with_context(|| format!("write {}", path.display()))?;
        self.named.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn drop_name(&self) -> anyhow::Result<()> {
        let path = self.config.file(NAME_FILE);
        std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
        self.named.store(false, Ordering::Relaxed);
        Ok(())
    }
}

/// Whether the phone has its name, which it takes on its first pairing.
#[tauri::command]
pub fn phone_named(phone: State<Phone>) -> bool {
    phone.named.load(Ordering::Relaxed)
}

/// Reads the key, in hex, of the server an invite pairs, shown before pairing spends it, naming the part of the
/// invite that is wrong.
#[tauri::command]
pub fn phone_invite_key(invite: String) -> Result<String, String> {
    let invite: Invite = invite.trim().parse()?;
    Ok(invite.server.hex())
}

/// Pairs with the server `invite` names and resolves to its name. A phone that has never paired names itself `name`
/// first; a named phone passes none.
#[tauri::command]
pub async fn phone_pair(
    hub: State<'_, Arc<Hub>>,
    phone: State<'_, Phone>,
    invite: String,
    name: Option<String>,
) -> Result<String, String> {
    let invite: Invite = invite.trim().parse()?;
    if let Some(listed) = hub.listed_as(&invite.server) {
        return Err(format!("Already paired as {listed}."));
    }
    let naming = !phone.named.load(Ordering::Relaxed);
    match (naming, name.as_deref().map(str::trim)) {
        (true, Some("") | None) => return Err("name this phone before its first pairing".into()),
        (true, Some(name)) => {
            phone.keep_name(name).map_err(|error| format!("{error:#}"))?;
            hub.me.rename(name.to_string());
        }
        (false, Some(_)) => return Err("this phone keeps the name it first paired with".into()),
        (false, None) => {}
    }
    match hub.pair(&invite).await {
        Ok(paired) => Ok(paired),
        Err(error) if naming => Err(match phone.drop_name() {
            Ok(()) => format!("{error:#}"),
            Err(dropping) => format!("{error:#}; and {dropping:#}"),
        }),
        Err(error) => Err(format!("{error:#}")),
    }
}
