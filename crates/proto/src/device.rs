//! The ed25519 key each device holds, which pairing pins. A server's is its iroh endpoint id; a client proves its own
//! on each connection.

use std::fmt;
use std::str::FromStr;

use ring::rand::SystemRandom;
use ring::signature::Ed25519KeyPair;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

const PREFIX: &str = "ed25519:";
/// The DER of the PKCS#8 v2 ed25519 key ring writes, up to the 32-byte seed (RFC 8410).
const PKCS8_SEED_PREFIX: [u8; 16] =
    [0x30, 0x51, 0x02, 0x01, 0x01, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20];

/// A device's public key, a point on the ed25519 curve spelled `ed25519:<hex>`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey(iroh::EndpointId);

impl PublicKey {
    pub fn as_bytes(&self) -> &[u8; 32] {
        self.0.as_bytes()
    }

    /// The iroh endpoint this key is.
    pub fn endpoint(&self) -> iroh::EndpointId {
        self.0
    }

    pub fn hex(&self) -> String {
        hex::encode(self.as_bytes())
    }

    pub(crate) fn from_bytes(bytes: &[u8; 32]) -> Result<Self, String> {
        iroh::EndpointId::from_bytes(bytes)
            .map(Self)
            .map_err(|_| format!("{} is not an ed25519 point", hex::encode(bytes)))
    }

    pub(crate) fn from_hex(encoded: &str) -> Result<Self, String> {
        let bytes: [u8; 32] = hex::decode(encoded)
            .map_err(|error| format!("{encoded:?}: {error}"))?
            .try_into()
            .map_err(|_| format!("{encoded:?} is not 32 bytes"))?;
        Self::from_bytes(&bytes)
    }
}

impl From<iroh::EndpointId> for PublicKey {
    fn from(id: iroh::EndpointId) -> Self {
        Self(id)
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{PREFIX}{}", self.hex())
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
        Self::from_hex(text.strip_prefix(PREFIX).ok_or_else(|| format!("{text:?} does not start with {PREFIX}"))?)
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
    secret: iroh::SecretKey,
}

impl DeviceKey {
    pub fn generate() -> Self {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).expect("the system RNG works");
        Self::from_pkcs8(pkcs8.as_ref()).expect("a generated key parses")
    }

    pub fn from_pkcs8(pkcs8: &[u8]) -> Result<Self, String> {
        Ed25519KeyPair::from_pkcs8(pkcs8).map_err(|error| format!("an ed25519 PKCS#8 key: {error}"))?;
        let seed = pkcs8
            .strip_prefix(PKCS8_SEED_PREFIX.as_slice())
            .and_then(<[u8]>::first_chunk::<32>)
            .ok_or("an ed25519 key in the PKCS#8 v2 layout ring writes")?;
        Ok(Self { pkcs8: pkcs8.to_vec(), secret: iroh::SecretKey::from_bytes(seed) })
    }

    pub fn pkcs8(&self) -> &[u8] {
        &self.pkcs8
    }

    pub fn public(&self) -> PublicKey {
        self.secret.public().into()
    }

    pub fn secret(&self) -> &iroh::SecretKey {
        &self.secret
    }
}
