//! This device's account as it keeps it: the log of the account it belongs to and of every one it left, and the
//! account's settings, beside its device key. Each change reloads the files and merges them first, so the app and a
//! CLI on one device never undo each other's entries.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, anyhow};
use nix::fcntl::{Flock, FlockArg};
use octopus_proto::account::{self, Account, Change, Listed};
use octopus_proto::device::DeviceKey;
use octopus_proto::lock;
use octopus_proto::settings::Settings;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::servers::{Config, create_parent, read_text};

const ACCOUNT_FILE: &str = "account.json";
const SETTINGS_FILE: &str = "settings.json";
/// The account this device belongs to, and the ones it left, which a server that trusted one follows it from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Kept {
    pub account: Account,
    pub former: Vec<Account>,
}

impl Kept {
    fn first(key: &DeviceKey) -> Self {
        Self { account: Account::first(key.public()), former: Vec::new() }
    }

    /// Takes what `other` holds that this lacks, moving to `other`'s account when `other` left this one for it, and
    /// tells whether anything changed.
    fn merge(&mut self, other: &Kept) -> anyhow::Result<bool> {
        let id = self.account.id();
        let mut changed = false;
        if other.account.id() == id {
            changed |= self.account.merge(&other.account).map_err(|error| anyhow!(error))? > 0;
        } else if other.former.iter().any(|former| former.id() == id) {
            let left = std::mem::replace(&mut self.account, other.account.clone());
            self.former.push(left);
            changed = true;
        }
        for theirs in &other.former {
            match self.former.iter_mut().find(|mine| mine.id() == theirs.id()) {
                Some(mine) => changed |= mine.merge(theirs).map_err(|error| anyhow!(error))? > 0,
                None if theirs.id() != self.account.id() => {
                    self.former.push(theirs.clone());
                    changed = true;
                }
                None => {}
            }
        }
        Ok(changed)
    }
}

/// What a `Book` holds: the account and the ones it left, and the account's settings.
#[derive(Clone, PartialEq)]
struct Held {
    kept: Kept,
    settings: Settings,
}

impl Held {
    /// The settings of the account it is in, holding only its members' values.
    fn settle(&mut self) {
        let id = self.kept.account.id();
        if self.settings.account != id {
            self.settings = Settings::new(id);
        }
        self.settings.retain_members(&self.kept.account.state());
    }
}

/// This device's account, kept in `account.json` and its settings in `settings.json` when it has files.
pub struct Book {
    config: Option<Config>,
    held: Mutex<Held>,
    changed: watch::Sender<u64>,
}

impl Book {
    /// The account kept in `config`, a first one for `key` on the first load.
    pub fn load(config: &Config, key: &DeviceKey) -> anyhow::Result<Self> {
        let book = Self { config: Some(config.clone()), ..Self::memory(key) };
        let _held = book.hold()?;
        let mut held = lock(&book.held);
        if let Some(kept) = read(&config.file(ACCOUNT_FILE))? {
            held.kept = kept;
        }
        if let Some(settings) = read(&config.file(SETTINGS_FILE))? {
            held.settings = settings;
        }
        held.settle();
        book.save(&held)?;
        drop(held);
        Ok(book)
    }

    /// An account kept in memory alone.
    pub fn memory(key: &DeviceKey) -> Self {
        let kept = Kept::first(key);
        let settings = Settings::new(kept.account.id());
        Self { config: None, held: Mutex::new(Held { kept, settings }), changed: watch::Sender::new(0) }
    }

    fn file(&self, name: &str) -> Option<PathBuf> {
        self.config.as_ref().map(|config| config.file(name))
    }

    /// What this process holds, merged with what another process of this device saved since; ticks `changes` when
    /// the files held something new.
    fn merged(&self) -> anyhow::Result<MutexGuard<'_, Held>> {
        let mut held = lock(&self.held);
        let mut changed = false;
        if let Some(on_disk) = self.file(ACCOUNT_FILE).as_deref().map(read::<Kept>).transpose()?.flatten()
            && held.kept.merge(&on_disk)?
        {
            held.settle();
            changed = true;
        }
        if let Some(on_disk) = self.file(SETTINGS_FILE).as_deref().map(read::<Settings>).transpose()?.flatten()
            && on_disk.account == held.settings.account
            && on_disk != held.settings
        {
            let members = held.kept.account.state();
            changed |= held.settings.merge(&on_disk, &members).map_err(|error| anyhow!(error))?;
        }
        if changed {
            self.changed.send_modify(|version| *version += 1);
        }
        Ok(held)
    }

    /// As `merged`, keeping what this process holds when the files cannot be read.
    fn current(&self) -> MutexGuard<'_, Held> {
        self.merged().unwrap_or_else(|error| {
            eprintln!("octopus: read the account another process saved: {error:#}");
            lock(&self.held)
        })
    }

    pub fn kept(&self) -> Kept {
        self.current().kept.clone()
    }

    pub fn account(&self) -> Account {
        self.current().kept.account.clone()
    }

    pub fn state(&self) -> account::State {
        self.current().kept.account.state()
    }

    pub fn settings(&self) -> Settings {
        self.current().settings.clone()
    }

    /// Ticks whenever the account takes an entry or its settings change.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// Applies `change` to the kept account, after merging what the files hold, and saves it; ticks `changes` when
    /// the account took an entry.
    pub fn update<T>(&self, change: impl FnOnce(&mut Kept) -> anyhow::Result<T>) -> anyhow::Result<T> {
        self.update_held(|held| change(&mut held.kept))
    }

    fn update_held<T>(&self, change: impl FnOnce(&mut Held) -> anyhow::Result<T>) -> anyhow::Result<T> {
        let _held = self.hold()?;
        let mut held = self.merged()?;
        let mut next = held.clone();
        let changed = change(&mut next)?;
        next.settle();
        self.save(&next)?;
        if next != *held {
            *held = next;
            self.changed.send_modify(|version| *version += 1);
        }
        Ok(changed)
    }

    /// Moves this device to `next`, an account holding it, after leaving the one it is in for it; `servers` decides
    /// whether that account's servers come along. It takes `settings`, refused as a merge refuses them, and brings
    /// each setting they lack.
    pub fn move_to(
        &self,
        key: &DeviceKey,
        mut next: Account,
        settings: &Settings,
        name: &str,
        servers: bool,
    ) -> anyhow::Result<()> {
        self.update_held(|held| {
            let kept = &mut held.kept;
            if servers {
                for server in kept.account.state().servers {
                    next.append(key, relisting(&server, &server.name));
                    if server.host == Some(key.public()) {
                        kept.account.append(key, Change::ForgetServer { server: server.key });
                    }
                }
            }
            next.append(key, Change::Name { name: name.to_string() });
            kept.account.append(key, Change::Leave { to: next.id() });
            let left = std::mem::replace(&mut kept.account, next);
            kept.former.retain(|former| former.id() != left.id());
            kept.former.push(left);
            let mut checked = Settings::new(kept.account.id());
            checked.merge(settings, &kept.account.state()).map_err(|error| anyhow!(error))?;
            held.settings.carry_into(&mut checked, key);
            held.settings = checked;
            Ok(())
        })
    }

    pub fn append(&self, key: &DeviceKey, change: Change) -> anyhow::Result<()> {
        self.update(|kept| {
            kept.account.append(key, change);
            Ok(())
        })
    }

    /// Applies `change` to the account's settings, after merging what the files hold, and saves them.
    pub fn update_settings(&self, change: impl FnOnce(&mut Settings)) -> anyhow::Result<()> {
        self.update_held(|held| {
            change(&mut held.settings);
            Ok(())
        })
    }

    /// Merges the log and settings a server returned.
    pub fn absorb(&self, merged: &Account, settings: &Settings) -> anyhow::Result<()> {
        self.update_held(|held| {
            if merged.id() != held.kept.account.id() {
                return Ok(());
            }
            held.kept.account.merge(merged).map_err(|error| anyhow!(error))?;
            held.settle();
            held.settings.merge(settings, &held.kept.account.state()).map(drop).map_err(|error| anyhow!(error))
        })
    }

    /// Holds the account file against every other process of this device until dropped. The lock is `flock`, which
    /// std's `File::lock` does not implement on Android.
    fn hold(&self) -> anyhow::Result<Option<Flock<std::fs::File>>> {
        let Some(path) = self.file(ACCOUNT_FILE) else { return Ok(None) };
        create_parent(&path)?;
        let held = path.with_extension("lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&held)
            .with_context(|| format!("open {}", held.display()))?;
        let locked = Flock::lock(file, FlockArg::LockExclusive)
            .map_err(|(_, error)| error)
            .with_context(|| format!("lock {}", held.display()))?;
        Ok(Some(locked))
    }

    fn save(&self, held: &Held) -> anyhow::Result<()> {
        let Some(config) = &self.config else { return Ok(()) };
        write(&config.file(ACCOUNT_FILE), &held.kept)?;
        write(&config.file(SETTINGS_FILE), &held.settings)
    }
}

fn write(path: &std::path::Path, value: &impl Serialize) -> anyhow::Result<()> {
    let staged = path.with_extension(format!("json.{}.new", std::process::id()));
    std::fs::write(&staged, serde_json::to_string(value)?).with_context(|| format!("write {}", staged.display()))?;
    std::fs::rename(&staged, path).with_context(|| format!("replace {}", path.display()))
}

fn read<T: DeserializeOwned>(path: &std::path::Path) -> anyhow::Result<Option<T>> {
    let parsed = read_text(path)?.map(|text| serde_json::from_str(&text));
    parsed.transpose().with_context(|| format!("parse {}", path.display()))
}

/// The change that lists `server` again, under `name`.
pub fn relisting(server: &Listed, name: &str) -> Change {
    Change::ListServer { server: server.key, relay: server.relay.clone(), name: name.to_string(), host: server.host }
}
