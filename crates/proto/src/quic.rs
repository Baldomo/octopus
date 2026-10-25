//! QUIC between machines: TLS 1.3 where each side proves its device key as a raw public key (RFC 7250).

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{ClientConfig, Endpoint, ServerConfig, TransportConfig, VarInt};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms, ring, verify_tls13_signature_with_raw_key};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, SubjectPublicKeyInfoDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::sign::CertifiedKey;
use rustls::{CertificateError, DigitallySignedStruct, DistinguishedName, PeerIncompatible, SignatureScheme, version};

use crate::device::{DeviceKey, PublicKey};

const ALPN: &[u8] = b"octopus/1";
/// Raw public keys carry no name, so every server answers to this one.
const SERVER_NAME: &str = "octopus";
/// A paired client's streams at once: a browser panel opens many tunnel streams beside terminals and file transfers.
pub const PAIRED_STREAMS: VarInt = VarInt::from_u32(1024);
/// A connection's streams before its client is known to be paired: the one that carries `pairing.pair`.
const UNPAIRED_STREAMS: VarInt = VarInt::from_u32(1);
/// A client opens every stream itself.
const SERVER_OPENED_STREAMS: VarInt = VarInt::from_u32(0);
const IDLE_TIMEOUT: Duration = Duration::from_secs(20);
const KEEP_ALIVE: Duration = Duration::from_secs(5);

/// The application error code a connection closes with when its work is done.
pub const DONE: VarInt = VarInt::from_u32(0);
/// The application error code a server closes a connection with when it refuses the client.
pub const REFUSED: VarInt = VarInt::from_u32(1);

fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

fn certified(key: &DeviceKey) -> Arc<CertifiedKey> {
    let private = PrivatePkcs8KeyDer::from(key.pkcs8().to_vec());
    let signer = ring::sign::any_eddsa_type(&private).expect("a device key is ed25519");
    Arc::new(CertifiedKey::new(vec![CertificateDer::from(key.public().spki())], signer))
}

fn transport(peer_streams: VarInt) -> Arc<TransportConfig> {
    let mut transport = TransportConfig::default();
    transport.max_concurrent_bidi_streams(peer_streams);
    transport.max_idle_timeout(Some(IDLE_TIMEOUT.try_into().expect("the idle timeout fits a varint")));
    transport.keep_alive_interval(Some(KEEP_ALIVE));
    Arc::new(transport)
}

fn bad_key() -> rustls::Error {
    rustls::Error::InvalidCertificate(CertificateError::BadEncoding)
}

/// Checks that the peer signed the handshake with the key it presented.
#[derive(Debug)]
struct Signatures(WebPkiSupportedAlgorithms);

impl Signatures {
    fn new() -> Self {
        Self(provider().signature_verification_algorithms)
    }

    fn verify(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature_with_raw_key(message, &SubjectPublicKeyInfoDer::from(cert.as_ref()), dss, &self.0)
    }
}

/// Accepts the server's key when it matches the pin; while pairing, there is no pin yet.
#[derive(Debug)]
struct ServerKey {
    pinned: Option<PublicKey>,
    signatures: Signatures,
}

impl ServerCertVerifier for ServerKey {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let presented = PublicKey::from_spki(end_entity).ok_or_else(bad_key)?;
        match self.pinned {
            Some(pinned) if pinned != presented => {
                Err(rustls::Error::General(format!("the server presented {presented}, not its pinned key {pinned}")))
            }
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(PeerIncompatible::Tls13RequiredForQuic.into())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.signatures.verify(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

/// Requires every client to prove a key; whether that key is paired is the server's call after the handshake.
#[derive(Debug)]
struct ClientKey(Signatures);

impl ClientCertVerifier for ClientKey {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        PublicKey::from_spki(end_entity).ok_or_else(bad_key)?;
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(PeerIncompatible::Tls13RequiredForQuic.into())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

fn invalid(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, error.to_string())
}

/// A server's QUIC listener, proving `key`. Each connection takes one stream until the server raises it to
/// `PAIRED_STREAMS` for a paired client.
pub fn listen(address: SocketAddr, key: &DeviceKey) -> io::Result<Endpoint> {
    let mut tls = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&version::TLS13])
        .map_err(invalid)?
        .with_client_cert_verifier(Arc::new(ClientKey(Signatures::new())))
        .with_cert_resolver(Arc::new(rustls::server::AlwaysResolvesServerRawPublicKeys::new(certified(key))));
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let mut config = ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls).map_err(invalid)?));
    config.transport_config(transport(UNPAIRED_STREAMS));
    Endpoint::server(config, address)
}

/// Connects to the server at `endpoint`, `host:port`, proving `key`. With no pin, any server key is accepted,
/// which only pairing does.
pub async fn connect(endpoint: &str, key: &DeviceKey, pinned: Option<PublicKey>) -> io::Result<quinn::Connection> {
    let address = tokio::net::lookup_host(endpoint)
        .await?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{endpoint} resolves to no address")))?;
    let local: SocketAddr = if address.is_ipv4() { ([0, 0, 0, 0], 0).into() } else { ([0u16; 8], 0).into() };
    let mut tls = rustls::ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&version::TLS13])
        .map_err(invalid)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(ServerKey { pinned, signatures: Signatures::new() }))
        .with_client_cert_resolver(Arc::new(rustls::client::AlwaysResolvesClientRawPublicKeys::new(certified(key))));
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let mut config = ClientConfig::new(Arc::new(QuicClientConfig::try_from(tls).map_err(invalid)?));
    config.transport_config(transport(SERVER_OPENED_STREAMS));
    let client = Endpoint::client(local)?;
    let connecting = client.connect_with(config, address, SERVER_NAME).map_err(invalid)?;
    connecting.await.map_err(|error| io::Error::other(format!("{endpoint}: {error}")))
}

/// The key the peer proved in the handshake.
pub fn peer_key(connection: &quinn::Connection) -> PublicKey {
    let identity = connection.peer_identity().expect("both sides prove a key in every handshake");
    let certificates = identity.downcast::<Vec<CertificateDer<'static>>>().expect("rustls reports certificates");
    let spki = certificates.first().expect("a proven key is one raw public key");
    PublicKey::from_spki(spki).expect("the verifiers accept only ed25519 keys")
}
