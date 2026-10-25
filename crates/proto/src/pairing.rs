//! The one-time code and the proofs that bind it to both device keys.
//!
//! Each side proves it knows the code with an HMAC over both keys of its own TLS handshake, so a peer in the
//! middle, who sees a different key on each side, can forward neither proof. The code carries 128 random bits,
//! too many to guess from a proof offline.

use std::fmt;
use std::str::FromStr;

use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::device::PublicKey;
use crate::schema::{Arg, Shape};

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
    [side.label(), &server.0, &client.0].concat()
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

/// Spelled in hex on the wire, and taken as text by the CLI.
macro_rules! hex_text {
    ($name:ident) => {
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&hex::encode(&self.0))
            }
        }

        impl Arg for $name {
            fn shape() -> Shape {
                Shape::Text
            }
        }
    };
}

hex_text!(Code);
hex_text!(Proof);

impl<'de> Deserialize<'de> for Code {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
    }
}

impl<'de> Deserialize<'de> for Proof {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        hex::decode(String::deserialize(deserializer)?).map(Self).map_err(serde::de::Error::custom)
    }
}
