//! Who may connect: the one-time codes a server offers, the accounts trusted by proving one, and forgetting them. A
//! code pairs the first client that proves it, then is spent.

use octopus_proto::Error;
use octopus_proto::device::PublicKey;
use octopus_proto::ops::{Accounts, Done, Offer, Paired, PairedAccount, PairedClient, PairingForget, PairingPair};
use octopus_proto::pairing::{Code, Invite, Proof, Side};
use octopus_proto::remote::Connection;

use crate::client::{self, Client};
use crate::{Server, State, lock, now};

const CODE_LIFETIME_MS: i64 = 10 * 60 * 1000;

/// Kept in memory only: a server restart spends every code.
#[derive(Default)]
pub(crate) struct Offers(Vec<(Code, i64)>);

impl Offers {
    /// A fresh code and when it stops pairing.
    pub fn offer(&mut self, now: i64) -> (Code, i64) {
        self.forget_expired(now);
        let offered = (Code::random(), now + CODE_LIFETIME_MS);
        self.0.push(offered);
        offered
    }

    pub fn offering(&mut self, now: i64) -> bool {
        self.forget_expired(now);
        !self.0.is_empty()
    }

    /// Spends and returns the unexpired code the client's proof was made with.
    pub(crate) fn redeem(&mut self, now: i64, proof: &Proof, server: &PublicKey, client: &PublicKey) -> Option<Code> {
        self.forget_expired(now);
        let spent = self.0.iter().position(|(code, _)| code.proves(proof, Side::Client, server, client))?;
        Some(self.0.remove(spent).0)
    }

    fn forget_expired(&mut self, now: i64) {
        self.0.retain(|(_, expires_at)| *expires_at > now);
    }
}

impl Server {
    pub(crate) fn offer(&self) -> Offer {
        let (code, expires_at) = lock(&self.offers).offer(now());
        let (server, relay) = (self.device.public(), self.relay.clone());
        Offer { invite: Invite { code, server, relay }, expires_at }
    }

    /// Whether a code or a link is on offer, the one time an unpaired device gets past the handshake.
    pub(crate) fn offering(&self) -> bool {
        let offering = lock(&self.offers).offering(now());
        offering || lock(&self.state).inviting(now())
    }

    /// Keeps `connection` among the open connections of `key` while a trusted account holds the key, so a removal or
    /// a forget closes them.
    pub(crate) fn track(&self, key: PublicKey, connection: &Connection) -> bool {
        let state = lock(&self.state);
        let member = state.is_member(&key);
        if member {
            lock(&self.connections).entry(key).or_default().push(connection.clone());
        }
        member
    }

    pub(crate) fn untrack(&self, key: &PublicKey, connection: &Connection) {
        let mut connections = lock(&self.connections);
        if let Some(open) = connections.get_mut(key) {
            open.retain(|each| each.stable_id() != connection.stable_id());
            if open.is_empty() {
                connections.remove(key);
            }
        }
    }

    /// A member device goes by the name its `session.hello` gives; a device no trusted account holds says hello to
    /// nobody.
    pub(crate) fn hello_paired(&self, key: PublicKey, name: &str) -> Result<Client, Error> {
        let mut state = lock(&self.state);
        if !state.is_member(&key) {
            return Err(Error::Unpaired);
        }
        self.name_device(&mut state, &key, name)?;
        Ok(Client::Paired(key))
    }

    /// Records the device's name, republishing the tasks whose lock it holds, as their label, when the name changed.
    fn name_device(&self, state: &mut State, key: &PublicKey, name: &str) -> Result<(), Error> {
        let named = state.store.device_name(key)?;
        if named.as_deref() != Some(name) {
            client::check_name(name)?;
            state.store.refuse_claim(name, key)?;
            state.store.name_device(name, key)?;
            if named.is_some() {
                let holder = Client::Paired(*key);
                self.touch_terminals(state, |terminal| terminal.holds(&holder))?;
            }
        }
        Ok(())
    }

    /// Trusts the device's account once it proves an unspent code.
    pub(crate) fn pair(&self, client: &PublicKey, op: PairingPair) -> Result<Paired, Error> {
        client::check_name(&op.client)?;
        let server = self.device.public();
        let mut state = lock(&self.state);
        state.store.refuse_claim(&op.client, client)?;
        let code = lock(&self.offers).redeem(now(), &op.proof, &server, client).ok_or(Error::CodeRefused)?;
        let merged = self.merged(&state, &op.account)?;
        if !merged.state.is_member(client) {
            return Err(Error::invalid("pairing.pair carries an account the pairing device is a member of"));
        }
        self.keep(&mut state, merged)?;
        self.name_device(&mut state, client, &op.client)?;
        self.evict(&mut state)?;
        Ok(Paired { server: self.name.clone(), proof: code.prove(Side::Server, &server, client) })
    }

    /// Stops trusting the account, closing its devices' connections and freeing every lock they hold.
    pub(crate) fn pairing_forget(&self, op: PairingForget) -> Result<Done, Error> {
        let mut state = lock(&self.state);
        if !self.untrust(&mut state, &op.account)? {
            return Err(Error::not_found(format!("a trusted account {}", op.account)));
        }
        self.evict(&mut state)?;
        Ok(Done {})
    }

    pub(crate) fn pairing_list(&self) -> Result<Accounts, Error> {
        let state = lock(&self.state);
        let recorded = state.store.devices()?;
        let connected = lock(&self.clients);
        let mut accounts: Vec<PairedAccount> = state
            .trusted
            .iter()
            .filter(|(_, trusted)| !trusted.state.members.is_empty())
            .map(|(id, trusted)| PairedAccount {
                account: *id,
                devices: trusted
                    .state
                    .members
                    .iter()
                    .map(|member| {
                        let (name, seen_at) = recorded.get(&member.key).cloned().unwrap_or((member.name.clone(), None));
                        let connected = connected.contains_key(&Client::Paired(member.key));
                        let hosts = trusted
                            .state
                            .servers
                            .iter()
                            .any(|listed| listed.key == self.device.public() && listed.host == Some(member.key));
                        PairedClient { name, key: member.key, hosts, connected, seen_at }
                    })
                    .collect(),
            })
            .collect();
        accounts.sort_by_key(|paired| paired.account);
        Ok(Accounts { accounts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use octopus_proto::device::DeviceKey;

    #[test]
    fn a_code_pairs_until_ten_minutes_after_its_offer() {
        let (server, client) = (DeviceKey::generate().public(), DeviceKey::generate().public());
        let mut offers = Offers::default();
        let ((early, early_end), (late, late_end)) = (offers.offer(0), offers.offer(0));
        assert_eq!(early_end, 10 * 60 * 1000);
        let proof = |code: Code| code.prove(Side::Client, &server, &client);
        assert!(offers.redeem(early_end - 1, &proof(early), &server, &client).is_some());
        assert!(offers.offering(late_end - 1));
        assert!(offers.redeem(late_end, &proof(late), &server, &client).is_none());
        assert!(!offers.offering(late_end));
    }
}
