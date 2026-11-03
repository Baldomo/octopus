//! iroh between machines: a client dials a server's key at the server's relay, the connection moves to a direct
//! path when one opens, and the client proves its device key before the server reads anything else.
//!
//! Every client process dials from an endpoint key of its own, since a relay forwards an endpoint id to its newest
//! connection only: the app and a CLI on one device would otherwise take each other's relay path. The device key
//! travels as the connection's one unidirectional stream: the key, then its signature over both endpoint ids, so
//! the proof is bound to this connection and to this server.

use std::collections::HashSet;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use iroh::endpoint::Connection;
use iroh::endpoint::{BindOpts, QuicTransportConfig, VarInt, presets};
pub use iroh::{Endpoint, RelayUrl};
use iroh::{EndpointAddr, EndpointId, RelayConfig, RelayMap, RelayMode, SecretKey, Signature};

use crate::device::{DeviceKey, PublicKey};

const ALPN: &[u8] = b"octopus/1";
/// A paired client's streams at once: a browser panel opens many tunnel streams beside terminals and file transfers.
pub(crate) const PAIRED_STREAMS: VarInt = VarInt::from_u32(1024);
/// A connection's streams before its client is known to be paired: the one that carries `pairing.pair`.
const UNPAIRED_STREAMS: VarInt = VarInt::from_u32(1);
/// The bytes a peer may have in flight before it is known to be paired; a paired client gets the whole window.
const UNPAIRED_WINDOW: VarInt = VarInt::from_u32(64 * 1024);
/// The one unidirectional stream a connection carries: its device proof.
const PROOF_STREAMS: VarInt = VarInt::from_u32(1);
/// A client opens every stream itself.
const SERVER_OPENED_STREAMS: VarInt = VarInt::from_u32(0);
const IDLE_TIMEOUT: Duration = Duration::from_secs(20);
const KEEP_ALIVE: Duration = Duration::from_secs(5);
const PROOF_LABEL: &[u8] = b"octopus device proof";
const PROOF_BYTES: usize = 32 + Signature::LENGTH;
/// A client sends its proof right after the handshake.
const PROOF_PATIENCE: Duration = Duration::from_secs(3);

/// The application error code a connection closes with when its work is done.
pub const DONE: VarInt = VarInt::from_u32(0);
/// The application error code a server closes a connection with when it refuses the client.
pub const REFUSED: VarInt = VarInt::from_u32(1);

/// Every cause in the chain, since iroh's errors keep theirs in `source`.
fn failed(what: impl std::fmt::Display, error: impl std::error::Error) -> io::Error {
    let mut text = format!("{what}: {error}");
    let mut cause = error.source();
    while let Some(next) = cause {
        text.push_str(&format!(": {next}"));
        cause = next.source();
    }
    io::Error::other(text)
}

/// n0's public relay in Europe: where a server the app installs is homed, and the relay an entry paired before relays
/// existed is taken to name.
pub const N0_EU_RELAY: &str = "https://euc1-1.relay.n0.iroh.link./";

/// A relay URL a client can reach a relay at: http or https.
pub fn relay(text: &str) -> Result<RelayUrl, String> {
    let url: url::Url = text.parse().map_err(|error| format!("relay URL {text:?}: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("relay URL {text:?} is not http or https"));
    }
    Ok(url.into())
}

/// The `Minimal` preset publishes no addresses to any discovery service: a peer learns them only from a connection
/// it holds.
fn builder(secret: SecretKey, relays: RelayMap, transport: QuicTransportConfig) -> iroh::endpoint::Builder {
    Endpoint::builder(presets::Minimal)
        .secret_key(secret)
        .relay_mode(RelayMode::Custom(relays))
        .transport_config(transport)
}

fn transport() -> iroh::endpoint::QuicTransportConfigBuilder {
    QuicTransportConfig::builder()
        .max_idle_timeout(Some(IDLE_TIMEOUT.try_into().expect("the idle timeout fits a varint")))
        .keep_alive_interval(KEEP_ALIVE)
}

/// A server's endpoint proving `key`, homed on `relay`, taking direct paths on UDP `udp`, and on IPv6 too when
/// `udp` is every IPv4 address. Each connection takes one stream and a small window until the server raises them
/// with `admit` for a paired client.
pub async fn listen(udp: SocketAddr, relay: RelayUrl, key: &DeviceKey) -> io::Result<Endpoint> {
    let transport = transport()
        .max_concurrent_bidi_streams(UNPAIRED_STREAMS)
        .max_concurrent_uni_streams(PROOF_STREAMS)
        .receive_window(UNPAIRED_WINDOW)
        .datagram_receive_buffer_size(None)
        .build();
    let mut builder = builder(key.secret().clone(), RelayMap::from(relay), transport)
        .alpns(vec![ALPN.to_vec()])
        .clear_ip_transports()
        .bind_addr(udp)
        .map_err(|error| failed(udp, error))?;
    if udp.ip() == Ipv4Addr::UNSPECIFIED {
        let ipv6 = SocketAddr::from((Ipv6Addr::UNSPECIFIED, udp.port()));
        builder = builder
            .bind_addr_with_opts(ipv6, BindOpts::default().set_is_required(false))
            .map_err(|error| failed(ipv6, error))?;
    }
    builder.bind().await.map_err(|error| failed(format!("bind UDP {udp}"), error))
}

/// Gives a paired client's connection every stream and the whole window.
pub fn admit(connection: &Connection) {
    connection.set_max_concurrent_bi_streams(PAIRED_STREAMS);
    connection.set_receive_window(VarInt::MAX);
}

/// A client process's endpoint, under a key of its own, homed on the relays of the servers it dials. It learns each
/// relay once, since every change to its relays re-runs its network report.
#[derive(Clone)]
pub struct Dialer {
    endpoint: Endpoint,
    relays: Arc<Mutex<HashSet<RelayUrl>>>,
}

fn claim(server: EndpointId, dialer: EndpointId) -> Vec<u8> {
    [PROOF_LABEL, server.as_bytes(), dialer.as_bytes()].concat()
}

impl Dialer {
    pub async fn bind() -> io::Result<Self> {
        let transport = transport()
            .max_concurrent_bidi_streams(SERVER_OPENED_STREAMS)
            .max_concurrent_uni_streams(SERVER_OPENED_STREAMS)
            .build();
        let endpoint = builder(SecretKey::generate(), RelayMap::empty(), transport)
            .bind()
            .await
            .map_err(|error| failed("bind this client's endpoint", error))?;
        Ok(Self { endpoint, relays: Arc::default() })
    }

    /// Dials the server holding `server` through `relay`, then proves `device` on the connection. Only that key's
    /// holder can complete the handshake.
    pub async fn connect(&self, device: &DeviceKey, server: PublicKey, relay: &RelayUrl) -> io::Result<Connection> {
        let at = format!("{server} at {relay}");
        let learned = self.relays.lock().unwrap_or_else(|poison| poison.into_inner()).insert(relay.clone());
        if learned {
            self.endpoint.insert_relay(relay.clone(), Arc::new(RelayConfig::from(relay.clone()))).await;
        }
        let connection = self
            .endpoint
            .connect(EndpointAddr::new(server.endpoint()).with_relay_url(relay.clone()), ALPN)
            .await
            .map_err(|error| failed(&at, error))?;
        let signature = device.secret().sign(&claim(server.endpoint(), self.endpoint.id()));
        let mut proof = connection.open_uni().await.map_err(|error| failed(&at, error))?;
        let bytes = [device.public().as_bytes().as_slice(), &signature.to_bytes()].concat();
        proof.write_all(&bytes).await.map_err(|error| failed(&at, error))?;
        proof.finish().map_err(|error| failed(&at, error))?;
        Ok(connection)
    }
}

/// The device key the client proved on `connection` to the server holding `server`, read before any other stream.
pub async fn proven_device(connection: &Connection, server: PublicKey) -> Option<PublicKey> {
    let read = async { connection.accept_uni().await.ok()?.read_to_end(PROOF_BYTES).await.ok() };
    let proof = tokio::time::timeout(PROOF_PATIENCE, read).await.ok()??;
    connection.set_max_concurrent_uni_streams(VarInt::from_u32(0));
    let (key, signature) = proof.split_first_chunk::<32>()?;
    let device = PublicKey::from_bytes(key).ok()?;
    let signature = Signature::from_bytes(signature.try_into().ok()?);
    let claim = claim(server.endpoint(), connection.remote_id());
    device.endpoint().verify(&claim, &signature).ok().map(|()| device)
}
