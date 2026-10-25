//! The one-time codes a server offers. A code pairs the first client that proves it, then is spent.

use octopus_proto::device::PublicKey;
use octopus_proto::ops::Offer;
use octopus_proto::pairing::{Code, Proof, Side};

const CODE_LIFETIME_MS: i64 = 10 * 60 * 1000;

/// Kept in memory only: a server restart spends every code.
#[derive(Default)]
pub struct Offers(Vec<(Code, i64)>);

impl Offers {
    pub fn offer(&mut self, now: i64) -> Offer {
        self.forget_expired(now);
        let code = Code::random();
        let expires_at = now + CODE_LIFETIME_MS;
        self.0.push((code, expires_at));
        Offer { code, expires_at }
    }

    /// Spends and returns the unexpired code the client's proof was made with.
    pub fn redeem(&mut self, now: i64, proof: &Proof, server: &PublicKey, client: &PublicKey) -> Option<Code> {
        self.forget_expired(now);
        let spent = self.0.iter().position(|(code, _)| code.proves(proof, Side::Client, server, client))?;
        Some(self.0.remove(spent).0)
    }

    fn forget_expired(&mut self, now: i64) {
        self.0.retain(|(_, expires_at)| *expires_at > now);
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
        let (early, late) = (offers.offer(0), offers.offer(0));
        assert_eq!(early.expires_at, 10 * 60 * 1000);
        let proof = |offer: &Offer| offer.code.prove(Side::Client, &server, &client);
        assert!(offers.redeem(early.expires_at - 1, &proof(&early), &server, &client).is_some());
        assert!(offers.redeem(late.expires_at, &proof(&late), &server, &client).is_none());
    }
}
