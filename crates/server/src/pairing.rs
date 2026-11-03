//! Who may connect: the one-time codes a server offers, the clients pinned by proving one, and forgetting them. A
//! code pairs the first client that proves it, then is spent.

use octopus_proto::Error;
use octopus_proto::device::PublicKey;
use octopus_proto::ops::{Clients, Done, Offer, Paired, PairedClient, PairingForget, PairingPair};
use octopus_proto::pairing::{Code, Invite, Proof, Side};
use octopus_proto::remote::{self, Connection};

use crate::client::{self, Client};
use crate::{Server, State, lock, now};

const FORGOTTEN: &[u8] = b"this client was forgotten; pair it again with a fresh invite";
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

    /// Whether an unspent code is on offer, the one time an unpaired client gets past the handshake.
    pub(crate) fn offering(&self) -> bool {
        lock(&self.offers).offering(now())
    }

    /// Keeps `connection` among the open connections of `key` while the key is pinned, so forgetting it closes them.
    pub(crate) fn track(&self, key: PublicKey, connection: &Connection) -> Result<bool, Error> {
        let state = lock(&self.state);
        let pinned = state.store.client_pinned_to(&key)?.is_some();
        if pinned {
            lock(&self.connections).entry(key).or_default().push(connection.clone());
        }
        Ok(pinned)
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

    /// A paired client goes by the name its `session.hello` gives; a key no longer pinned says hello to nobody.
    pub(crate) fn hello_paired(&self, key: PublicKey, name: &str) -> Result<Client, Error> {
        let mut state = lock(&self.state);
        let pinned = state.store.client_pinned_to(&key)?.ok_or(Error::Unpaired)?;
        if pinned != name {
            client::check_name(name)?;
            state.store.refuse_claim(name, &key)?;
            state.store.rename_client(&key, name)?;
            self.touch_held(&mut state, &Client::Paired(key))?;
        }
        Ok(Client::Paired(key))
    }

    /// Pins the client's key under its name once it proves an unspent code.
    pub(crate) fn pair(&self, client: &PublicKey, op: PairingPair) -> Result<Paired, Error> {
        client::check_name(&op.client)?;
        let server = self.device.public();
        let mut state = lock(&self.state);
        state.store.refuse_claim(&op.client, client)?;
        let code = lock(&self.offers).redeem(now(), &op.proof, &server, client).ok_or(Error::CodeRefused)?;
        let renamed = state.store.client_pinned_to(client)?.is_some_and(|pinned| pinned != op.client);
        state.store.pin_client(&op.client, client)?;
        if renamed {
            self.touch_held(&mut state, &Client::Paired(*client))?;
        }
        Ok(Paired { server: self.name.clone(), proof: code.prove(Side::Server, &server, client) })
    }

    /// Unpins the named client, closes its open connections and frees every lock it holds.
    pub(crate) fn pairing_forget(&self, op: PairingForget) -> Result<Done, Error> {
        let mut state = lock(&self.state);
        let key = state.store.forget_client(&op.client)?;
        for connection in lock(&self.connections).remove(&key).into_iter().flatten() {
            connection.close(remote::REFUSED, FORGOTTEN);
        }
        let forgotten = Client::Paired(key);
        self.touch_terminals(&mut state, |terminal| terminal.free(&forgotten))?;
        Ok(Done {})
    }

    pub(crate) fn pairing_list(&self) -> Result<Clients, Error> {
        let pinned = lock(&self.state).store.clients()?;
        let connected = lock(&self.clients);
        let clients = pinned
            .into_iter()
            .map(|(key, name, seen_at)| PairedClient {
                connected: connected.contains_key(&Client::Paired(key)),
                name,
                key,
                seen_at,
            })
            .collect();
        Ok(Clients { clients })
    }

    /// Republishes each task whose lock `client` holds, as its label changed.
    fn touch_held(&self, state: &mut State, client: &Client) -> Result<(), Error> {
        self.touch_terminals(state, |terminal| terminal.holds(client))
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
