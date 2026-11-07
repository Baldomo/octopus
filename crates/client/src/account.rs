//! This device's account as it keeps it: the log of the account it belongs to and of every one it left, beside its
//! device key. Each change reloads the file and merges it first, so the app and a CLI on one device never undo each
//! other's entries.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, anyhow};
use nix::fcntl::{Flock, FlockArg};
use octopus_proto::account::{self, Account, Change, Listed};
use octopus_proto::device::DeviceKey;
use octopus_proto::lock;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::servers::{Config, create_parent, read_text};

const ACCOUNT_FILE: &str = "account.json";

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

/// This device's account, kept in `account.json` when it has a file.
pub struct Book {
    path: Option<PathBuf>,
    kept: Mutex<Kept>,
    changed: watch::Sender<u64>,
}

impl Book {
    /// The account kept in `config`, a first one for `key` on the first load.
    pub fn load(config: &Config, key: &DeviceKey) -> anyhow::Result<Self> {
        let path = config.file(ACCOUNT_FILE);
        let book = Self { path: Some(path.clone()), ..Self::memory(key) };
        let _held = book.hold()?;
        match read(&path)? {
            Some(kept) => *lock(&book.kept) = kept,
            None => book.save(&lock(&book.kept))?,
        }
        Ok(book)
    }

    /// An account kept in memory alone.
    pub fn memory(key: &DeviceKey) -> Self {
        Self { path: None, kept: Mutex::new(Kept::first(key)), changed: watch::Sender::new(0) }
    }

    /// What this process holds, merged with what another process of this device saved since; ticks `changes` when
    /// the file held something new.
    fn merged(&self) -> anyhow::Result<MutexGuard<'_, Kept>> {
        let mut kept = lock(&self.kept);
        if let Some(on_disk) = self.path.as_deref().map(read).transpose()?.flatten()
            && kept.merge(&on_disk)?
        {
            self.changed.send_modify(|version| *version += 1);
        }
        Ok(kept)
    }

    /// As `merged`, keeping what this process holds when the file cannot be read.
    fn current(&self) -> MutexGuard<'_, Kept> {
        self.merged().unwrap_or_else(|error| {
            eprintln!("octopus: read the account another process saved: {error:#}");
            lock(&self.kept)
        })
    }

    pub fn kept(&self) -> Kept {
        self.current().clone()
    }

    pub fn account(&self) -> Account {
        self.current().account.clone()
    }

    pub fn state(&self) -> account::State {
        self.current().account.state()
    }

    /// Ticks whenever the account takes an entry.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// Applies `change` to the kept account, after merging what the file holds, and saves it; ticks `changes` when
    /// the account took an entry.
    pub fn update<T>(&self, change: impl FnOnce(&mut Kept) -> anyhow::Result<T>) -> anyhow::Result<T> {
        let _held = self.hold()?;
        let mut kept = self.merged()?;
        let before = kept.clone();
        let changed = change(&mut kept)?;
        self.save(&kept)?;
        if *kept != before {
            self.changed.send_modify(|version| *version += 1);
        }
        Ok(changed)
    }

    /// Moves this device to `next`, an account holding it, after leaving the one it is in for it; `servers` decides
    /// whether that account's servers come along.
    pub fn move_to(&self, key: &DeviceKey, mut next: Account, name: &str, servers: bool) -> anyhow::Result<()> {
        self.update(|kept| {
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
            Ok(())
        })
    }

    pub fn append(&self, key: &DeviceKey, change: Change) -> anyhow::Result<()> {
        self.update(|kept| {
            kept.account.append(key, change);
            Ok(())
        })
    }

    /// Merges the log a server returned.
    pub fn absorb(&self, merged: &Account) -> anyhow::Result<()> {
        self.update(|kept| match merged.id() == kept.account.id() {
            true => kept.account.merge(merged).map(drop).map_err(|error| anyhow!(error)),
            false => Ok(()),
        })
    }

    /// Holds the account file against every other process of this device until dropped. The lock is `flock`, which
    /// std's `File::lock` does not implement on Android.
    fn hold(&self) -> anyhow::Result<Option<Flock<std::fs::File>>> {
        let Some(path) = &self.path else { return Ok(None) };
        create_parent(path)?;
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

    fn save(&self, kept: &Kept) -> anyhow::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        create_parent(path)?;
        let staged = path.with_extension(format!("json.{}.new", std::process::id()));
        std::fs::write(&staged, serde_json::to_string(kept)?).with_context(|| format!("write {}", staged.display()))?;
        std::fs::rename(&staged, path).with_context(|| format!("replace {}", path.display()))
    }
}

fn read(path: &std::path::Path) -> anyhow::Result<Option<Kept>> {
    let parsed = read_text(path)?.map(|text| serde_json::from_str(&text));
    parsed.transpose().with_context(|| format!("parse {}", path.display()))
}

/// The change that lists `server` again, under `name`.
pub fn relisting(server: &Listed, name: &str) -> Change {
    Change::ListServer { server: server.key, relay: server.relay.clone(), name: name.to_string(), host: server.host }
}
