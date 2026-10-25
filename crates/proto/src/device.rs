//! The ed25519 key each device holds, which pairing pins and every QUIC handshake proves.

use std::fmt;
use std::str::FromStr;

use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

const PREFIX: &str = "ed25519:";
/// The DER of an ed25519 SubjectPublicKeyInfo up to the key itself (RFC 8410).
const SPKI_PREFIX: [u8; 12] = [0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00];

/// A device's public key, spelled `ed25519:<hex>`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey(pub [u8; 32]);

impl PublicKey {
    /// The key as TLS carries a raw public key (RFC 7250).
    pub fn spki(&self) -> Vec<u8> {
        [SPKI_PREFIX.as_slice(), &self.0].concat()
    }

    pub fn from_spki(spki: &[u8]) -> Option<Self> {
        let key = spki.strip_prefix(SPKI_PREFIX.as_slice())?;
        Some(Self(key.try_into().ok()?))
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{PREFIX}{}", hex::encode(self.0))
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for PublicKey {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let encoded = text.strip_prefix(PREFIX).ok_or_else(|| format!("{text:?} does not start with {PREFIX}"))?;
        let bytes = hex::decode(encoded).map_err(|error| format!("{text:?}: {error}"))?;
        Ok(Self(bytes.try_into().map_err(|_| format!("{text:?} is not 32 bytes"))?))
    }
}

impl Serialize for PublicKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
    }
}

/// This device's key pair, kept as PKCS#8.
pub struct DeviceKey {
    pkcs8: Vec<u8>,
    public: PublicKey,
}

impl DeviceKey {
    pub fn generate() -> Self {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).expect("the system RNG works");
        Self::from_pkcs8(pkcs8.as_ref()).expect("a generated key parses")
    }

    pub fn from_pkcs8(pkcs8: &[u8]) -> Result<Self, String> {
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8).map_err(|error| format!("an ed25519 PKCS#8 key: {error}"))?;
        let public = PublicKey(pair.public_key().as_ref().try_into().expect("an ed25519 public key is 32 bytes"));
        Ok(Self { pkcs8: pkcs8.to_vec(), public })
    }

    pub fn pkcs8(&self) -> &[u8] {
        &self.pkcs8
    }

    pub fn public(&self) -> PublicKey {
        self.public
    }
}
