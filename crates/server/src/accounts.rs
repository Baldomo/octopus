//! The accounts a server trusts: each one's log and settings, merged with every log and settings a member sends, and
//! who that lets in. A merge that removes a device closes its connections and frees its locks at once.

use std::collections::HashMap;

use octopus_proto::account::{self, Account, AccountId};
use octopus_proto::device::PublicKey;
use octopus_proto::ops::{AccountJoin, AccountSync, Hosted, Synced};
use octopus_proto::remote;
use octopus_proto::settings::Settings;
use octopus_proto::{Error, Event};

use crate::client::Client;
use crate::{Server, State, lock, now};

const REMOVED: &[u8] = b"this device is no longer a member of an account this server trusts";

/// A trusted account's log, what it comes to, and its settings.
#[derive(Clone)]
pub(crate) struct Trusted {
    pub account: Account,
    pub state: account::State,
    pub settings: Settings,
}

pub(crate) fn load(accounts: Vec<(Account, Settings)>) -> HashMap<AccountId, Trusted> {
    accounts.into_iter().map(|(account, settings)| (account.id(), Trusted::new(account, settings))).collect()
}

impl State {
    pub(crate) fn is_member(&self, key: &PublicKey) -> bool {
        self.trusted.values().any(|trusted| trusted.state.is_member(key))
    }

    /// Whether a trusted account holds an invite no device has taken yet.
    pub(crate) fn inviting(&self, now: i64) -> bool {
        self.trusted.values().any(|trusted| trusted.open_invite(None, now))
    }
}

impl Trusted {
    fn new(account: Account, settings: Settings) -> Self {
        let state = account.state();
        Self { account, state, settings }
    }

    /// Whether the invite whose key is `key`, or any invite, is open and unexpired at `now`.
    fn open_invite(&self, key: Option<&PublicKey>, now: i64) -> bool {
        self.state.open_invites.iter().any(|(_, open, expires)| key.is_none_or(|key| key == open) && *expires > now)
    }

    /// This log merged with `account`'s entries, keeping the settings of the devices still members. Nothing is kept
    /// until `keep`.
    fn merged(&self, account: &Account) -> Result<Self, Error> {
        let mut merged = self.account.clone();
        if merged.merge(account).map_err(Error::invalid)? == 0 {
            return Ok(self.clone());
        }
        let mut merged = Self::new(merged, self.settings.clone());
        merged.settings.retain_members(&merged.state);
        Ok(merged)
    }
}

impl Server {
    fn hosted(&self) -> Hosted {
        Hosted { key: self.device.public(), relay: self.relay.clone(), name: self.name.clone() }
    }

    /// The trusted log of `account` merged with it, or `account` itself when the server trusts no log of it yet.
    /// Nothing is kept until `keep`.
    pub(crate) fn merged(&self, state: &State, account: &Account) -> Result<Trusted, Error> {
        match state.trusted.get(&account.id()) {
            Some(trusted) => trusted.merged(account),
            None => Ok(Trusted::new(Account::checked(account).map_err(Error::invalid)?, Settings::new(account.id()))),
        }
    }

    /// Keeps the merged log and settings in place of the trusted ones, telling every client when either changed.
    /// Callers `evict` once their merges are done, so a device moving between accounts keeps the connection it moves
    /// on.
    pub(crate) fn keep(&self, state: &mut State, merged: Trusted) -> Result<(), Error> {
        let id = merged.account.id();
        let unchanged = state.trusted.get(&id).is_some_and(|trusted| {
            trusted.account.entries.len() == merged.account.entries.len() && trusted.settings == merged.settings
        });
        if unchanged {
            return Ok(());
        }
        state.store.keep_account(&merged.account, &merged.settings)?;
        state.trusted.insert(id, merged);
        let rev = state.store.rev()?;
        self.publish(state, rev, Event::AccountUpdated { account: id });
        Ok(())
    }

    /// Stops trusting the account, forgetting the names of its devices no other trusted account holds.
    pub(crate) fn untrust(&self, state: &mut State, id: &AccountId) -> Result<bool, Error> {
        let Some(forgotten) = state.trusted.remove(id) else { return Ok(false) };
        state.store.forget_account(id)?;
        let gone: Vec<PublicKey> =
            forgotten.state.members.iter().map(|member| member.key).filter(|key| !state.is_member(key)).collect();
        state.store.forget_devices(&gone)?;
        Ok(true)
    }

    /// Closes the connections of every device no trusted account holds and frees every lock such a device holds,
    /// connected or not.
    pub(crate) fn evict(&self, state: &mut State) -> Result<(), Error> {
        let connected: Vec<PublicKey> =
            lock(&self.connections).keys().filter(|key| !state.is_member(key)).copied().collect();
        for key in &connected {
            for connection in lock(&self.connections).remove(key).into_iter().flatten() {
                connection.close(remote::REFUSED, REMOVED);
            }
        }
        let holding: Vec<Client> = state
            .every_terminal()
            .filter_map(|(_, terminal)| terminal.holder())
            .filter(|holder| matches!(holder, Client::Paired(key) if !state.is_member(key)))
            .collect();
        for holder in &holding {
            self.touch_terminals(state, |terminal| terminal.free(holder))?;
        }
        Ok(())
    }

    /// A client on this machine is the server's owner, so the account it sends is trusted in place of the one it
    /// sent before. A paired device writes only into a trusted log that held it before the merge, or into the
    /// account the server first trusts by following its leave of one.
    pub(crate) fn account_sync(&self, client: &Client, op: AccountSync) -> Result<Synced, Error> {
        let mut state = lock(&self.state);
        let id = op.account.id();
        let sender = match client {
            Client::Paired(key) => Some(*key),
            Client::Local(_) => None,
        };
        let held = |state: &State, id: &AccountId, key: &PublicKey| {
            state.trusted.get(id).is_some_and(|trusted| trusted.state.is_member(key))
        };
        let mut followed = false;
        for former in &op.former {
            if sender.is_some_and(|key| !held(&state, &former.id(), &key)) {
                continue;
            }
            let Some(trusted) = state.trusted.get(&former.id()) else { continue };
            let merged = trusted.merged(former)?;
            followed |= sender.is_some_and(|key| merged.state.left.contains(&(key, id)));
            self.keep(&mut state, merged)?;
        }
        let member_before = sender.is_none_or(|key| held(&state, &id, &key));
        if !member_before && !(followed && !state.trusted.contains_key(&id)) {
            return Err(Error::Unpaired);
        }
        let mut merged = self.merged(&state, &op.account)?;
        merged.settings.merge(&op.settings, &merged.state).map_err(Error::invalid)?;
        let member = sender.is_none_or(|key| merged.state.is_member(&key));
        if !member_before && !member {
            return Err(Error::Unpaired);
        }
        self.keep(&mut state, merged)?;
        if !member {
            self.evict(&mut state)?;
            return Err(Error::Unpaired);
        }
        if sender.is_none() {
            let before = state.store.owner()?;
            if before != Some(id) {
                if let Some(before) = before {
                    self.untrust(&mut state, &before)?;
                }
                state.store.set_owner(&id)?;
            }
        }
        self.evict(&mut state)?;
        let Trusted { account, settings, .. } = state.trusted[&id].clone();
        Ok(Synced { account, settings, server: self.hosted() })
    }

    /// Takes a join on a trusted account's open, unexpired invite from the device `key`, keeping nothing unless the
    /// device becomes a member by it.
    pub(crate) fn account_join(&self, key: &PublicKey, op: AccountJoin) -> Result<Synced, Error> {
        let mut state = lock(&self.state);
        let trusted = state
            .trusted
            .get(&op.account)
            .ok_or_else(|| Error::not_found(format!("account {}", op.account.short())))?;
        if op.join.by != *key {
            return Err(Error::invalid("a join is signed by the device that sends it"));
        }
        let account::Change::Join { invite, .. } = &op.join.entry.change else {
            return Err(Error::invalid("account.join carries a join entry"));
        };
        if !trusted.open_invite(Some(invite), now()) {
            return Err(Error::invalid("the link is expired or spent; make a new one"));
        }
        let mut joining = trusted.account.clone();
        joining.entries.push(op.join);
        let merged = trusted.merged(&joining)?;
        if !merged.state.is_member(key) {
            return Err(Error::invalid(
                "the link's invite does not take this join: it is not proved with the link's secret",
            ));
        }
        let (account, settings) = (merged.account.clone(), merged.settings.clone());
        self.keep(&mut state, merged)?;
        self.evict(&mut state)?;
        Ok(Synced { account, settings, server: self.hosted() })
    }
}
