//! The one-time code and the proofs that bind it to both device keys.
//!
//! Each side proves it knows the code with an HMAC over both device keys its connection proved, so a peer in the
//! middle, who sees a different key on each side, can forward neither proof. The code carries 128 random bits,
//! too many to guess from a proof offline.

use std::fmt;
use std::str::FromStr;

use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::device::PublicKey;
use crate::remote::{self, RelayUrl};
use crate::schema::{Shape, arg_shape, spelled};

const CODE_BYTES: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Code([u8; CODE_BYTES]);

/// Which side a proof comes from, so neither side's proof stands in for the other's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Client,
    Server,
}

impl Side {
    fn label(self) -> &'static [u8] {
        match self {
            Self::Client => b"octopus pairing client",
            Self::Server => b"octopus pairing server",
        }
    }
}

/// An HMAC-SHA256 of one side's claim to the code, spelled in hex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proof(Vec<u8>);

impl Code {
    pub fn random() -> Self {
        let mut bytes = [0u8; CODE_BYTES];
        SystemRandom::new().fill(&mut bytes).expect("the system RNG works");
        Self(bytes)
    }

    fn key(&self) -> hmac::Key {
        hmac::Key::new(hmac::HMAC_SHA256, &self.0)
    }

    pub fn prove(&self, side: Side, server: &PublicKey, client: &PublicKey) -> Proof {
        Proof(hmac::sign(&self.key(), &claim(side, server, client)).as_ref().to_vec())
    }

    /// Compares in constant time.
    pub fn proves(&self, proof: &Proof, side: Side, server: &PublicKey, client: &PublicKey) -> bool {
        hmac::verify(&self.key(), &claim(side, server, client), &proof.0).is_ok()
    }
}

fn claim(side: Side, server: &PublicKey, client: &PublicKey) -> Vec<u8> {
    [side.label(), server.as_bytes(), client.as_bytes()].concat()
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Code(…)")
    }
}

impl FromStr for Code {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(text.trim()).map_err(|error| format!("a pairing code is hex: {error}"))?;
        Ok(Self(bytes.try_into().map_err(|_| format!("a pairing code is {} hex digits", 2 * CODE_BYTES))?))
    }
}

/// Everything a client needs to pair: the code, and the key and relay that reach the server offering it. Spelled
/// `octopus://<server key in hex>?code=<code>&relay=<relay URL>`.
#[derive(Clone, PartialEq, Eq)]
pub struct Invite {
    pub code: Code,
    pub server: PublicKey,
    pub relay: RelayUrl,
}

const SCHEME: &str = "octopus";

impl fmt::Display for Invite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut url = url::Url::parse(&format!("{SCHEME}://{}", self.server.hex())).expect("a hex host is a URL");
        url.query_pairs_mut().append_pair("code", &self.code.to_string()).append_pair("relay", self.relay.as_str());
        f.write_str(url.as_str())
    }
}

impl fmt::Debug for Invite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invite({:?}, {}, {})", self.code, self.server, self.relay)
    }
}

impl FromStr for Invite {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let shape = format!("an invite is {SCHEME}://<server key>?code=<code>&relay=<relay URL>");
        let url = url::Url::parse(text.trim()).map_err(|error| format!("{shape}: {error}"))?;
        let bare = url.username().is_empty() && url.password().is_none() && url.port().is_none();
        let server = match (url.scheme(), url.host_str(), url.path(), url.fragment()) {
            (SCHEME, Some(server), "", None) if bare => PublicKey::from_hex(server)?,
            _ => return Err(shape),
        };
        let (mut code, mut relay) = (None, None);
        for (name, value) in url.query_pairs() {
            let twice = match name.as_ref() {
                "code" => code.replace(value.parse()?).is_some(),
                "relay" => relay.replace(remote::relay(&value)?).is_some(),
                other => return Err(format!("an invite has no {other}; {shape}")),
            };
            if twice {
                return Err(format!("an invite names one {name}"));
            }
        }
        match (code, relay) {
            (Some(code), Some(relay)) => Ok(Self { code, server, relay }),
            (code, relay) => {
                let missing = [("code", code.is_none()), ("relay", relay.is_none())];
                let missing: Vec<&str> = missing.iter().filter(|(_, absent)| *absent).map(|(name, _)| *name).collect();
                Err(format!("the invite names no {}", missing.join(" and no ")))
            }
        }
    }
}

spelled!(Invite);
arg_shape!(Shape::Text => Invite, Proof);

impl Serialize for Proof {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex::encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for Proof {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        hex::decode(String::deserialize(deserializer)?).map(Self).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceKey;

    #[test]
    fn an_invite_round_trips_as_text_and_names_what_is_wrong_with_a_bad_one() {
        let relay = "https://euc1-1.relay.n0.iroh.link./".parse().expect("a relay URL");
        let invite = Invite { code: Code::random(), server: DeviceKey::generate().public(), relay };
        assert_eq!(invite.to_string().parse::<Invite>(), Ok(invite.clone()));
        let (code, key) = (invite.code.to_string(), invite.server.hex());
        let relay = "relay=https%3A%2F%2Frelay.example.%2F";
        let bad = [
            (format!("https://{key}?code={code}&{relay}"), "an invite is octopus://"),
            (format!("octopus://02{}?code={code}&{relay}", "00".repeat(31)), "not an ed25519 point"),
            (format!("octopus://{key}?code={code}&relay=ftp%3A%2F%2Fx%2F"), "not http or https"),
            (format!("octopus://{key}"), "names no code and no relay"),
            (format!("octopus://{key}?code={code}&code={code}&{relay}"), "names one code"),
            (format!("octopus://{key}:7300?code={code}&{relay}"), "an invite is octopus://"),
            (format!("octopus://{key}?code={code}&{relay}&port=1"), "has no port"),
        ];
        for (text, error) in bad {
            let refused = text.parse::<Invite>().expect_err(&text);
            assert!(refused.contains(error), "{text}: {refused}");
        }
    }
}
