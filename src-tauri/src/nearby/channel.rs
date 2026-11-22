//! The channel between two installs on the local network: TLS 1.3 over a
//! blocking TCP socket, both sides presenting their identity's self-signed
//! certificate, with whole messages framed on top.
//!
//! TLS does the key agreement and the authentication, and nothing here adds
//! to either. What the handshake proves is only that the peer holds the key
//! its device id names, and anyone can make a key: whether to trust that
//! device is decided above this layer (`trust`), with proofs bound to the
//! session's [`Channel::binding`].

use std::fmt;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::Resumption;
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::NoServerSessionStorage;
use rustls::sign::SingleCertAndKey;
use rustls::{
    CertificateError, ClientConfig, ClientConnection, DigitallySignedStruct, DistinguishedName,
    PeerMisbehaved, ServerConfig, ServerConnection, SignatureScheme, StreamOwned,
};
use serde::{Deserialize, Serialize};

use super::identity::{DeviceId, Identity};
use super::{Error, Result};

/// The application protocol both sides name in the handshake.
const ALPN: &[u8] = b"rowel-nearby/1";

/// The exporter label the binding is drawn under (RFC 8446 §7.5).
const BINDING_LABEL: &[u8] = b"EXPORTER-rowel-nearby-binding";

/// The most one frame carries. A longer message goes as several.
pub const MAX_FRAME: usize = 1024 * 1024;

/// The most a control message may be.
const MAX_CONTROL: usize = 64 * 1024;

/// How long connecting, the handshake, and any one read or write may stall
/// before the channel gives up on the peer. A deadline on the stall, not on a
/// whole message: a pack over a slow network legitimately takes longer.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The protocol version `hello` announces.
pub const PROTOCOL_VERSION: u32 = 1;

/// The areas of control messages this build speaks.
const AREAS: &[&str] = &["hello", "trust", "sync"];

/// Which end of a connection a side is. Part of every key proof, so a proof
/// one side sends cannot be handed back to it as the other's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The side that dialled.
    Connector,
    /// The side that answered.
    Acceptor,
}

impl Role {
    pub fn other(self) -> Self {
        match self {
            Self::Connector => Self::Acceptor,
            Self::Acceptor => Self::Connector,
        }
    }
}

/// An open, authenticated connection to one device.
///
/// Any error from a read or a write leaves the stream at an unknown point in
/// a message, so a channel that has failed once is dropped, not reused.
pub struct Channel {
    tls: Tls,
    peer: DeviceId,
    binding: [u8; 32],
}

/// [`connect_over`] a socket of its own, for tests.
#[cfg(test)]
pub fn connect(
    addr: std::net::SocketAddr,
    identity: &Identity,
    expect: Option<&DeviceId>,
) -> Result<Channel> {
    connect_over(
        TcpStream::connect_timeout(&addr, TIMEOUT)?,
        identity,
        expect,
    )
}

/// The dialling side, over a socket the caller connected (and keeps a handle
/// on, to shut it down from another thread). With `expect`, the handshake is
/// refused unless the device that answers is that one, before this side
/// shows its own certificate.
pub fn connect_over(
    sock: TcpStream,
    identity: &Identity,
    expect: Option<&DeviceId>,
) -> Result<Channel> {
    let provider = provider()?;
    let verifier = Arc::new(PeerVerifier::new(&provider, expect.copied()));
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_cert_resolver(Arc::new(SingleCertAndKey::from(identity.certified_key())));
    config.alpn_protocols = vec![ALPN.to_vec()];
    // Every connection is a full handshake, so the verifier, and with it
    // `expect`, sees every one.
    config.resumption = Resumption::disabled();
    // The name is a placeholder the verifier ignores; there is no reason to
    // send it.
    config.enable_sni = false;
    let name = ServerName::try_from("rowel-nearby").expect("a valid DNS name");
    let conn = ClientConnection::new(Arc::new(config), name)?;
    match handshake(conn.into(), sock) {
        Err(Error::Tls(rustls::Error::InvalidCertificate(
            CertificateError::ApplicationVerificationFailure,
        ))) if expect.is_some() => Err(Error::UnexpectedPeer),
        other => other,
    }
}

/// Answer a connection a listener accepted.
pub fn accept(sock: TcpStream, identity: &Identity) -> Result<Channel> {
    let provider = provider()?;
    let verifier = Arc::new(PeerVerifier::new(&provider, None));
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(verifier)
        .with_cert_resolver(Arc::new(SingleCertAndKey::from(identity.certified_key())));
    config.alpn_protocols = vec![ALPN.to_vec()];
    config.session_storage = Arc::new(NoServerSessionStorage {});
    config.send_tls13_tickets = 0;
    let conn = ServerConnection::new(Arc::new(config))?;
    handshake(conn.into(), sock)
}

/// The process's rustls provider: the ring one the app installs once for
/// every TLS client it has (`sync::install_crypto_provider`).
fn provider() -> Result<Arc<CryptoProvider>> {
    crate::sync::install_crypto_provider();
    CryptoProvider::get_default()
        .cloned()
        .ok_or_else(|| Error::Tls(rustls::Error::General("no TLS provider installed".into())))
}

fn handshake(mut conn: rustls::Connection, mut sock: TcpStream) -> Result<Channel> {
    // Control messages are small and answered at once; Nagle would hold each
    // one back for the previous one's acknowledgement.
    sock.set_nodelay(true)?;
    set_timeouts(&sock, TIMEOUT)?;
    // Runs the whole handshake: it returns once that is done, or failed.
    conn.complete_io(&mut sock)?;
    if conn.is_handshaking() {
        return Err(Error::Protocol("the handshake did not finish".into()));
    }
    if conn.alpn_protocol() != Some(ALPN) {
        return Err(Error::Protocol(
            "the peer does not speak rowel-nearby/1".into(),
        ));
    }
    let cert = conn
        .peer_certificates()
        .and_then(|certs| certs.first())
        .ok_or_else(|| Error::Protocol("the peer presented no certificate".into()))?;
    let peer = DeviceId::of(cert)?;
    let binding = conn.export_keying_material([0u8; 32], BINDING_LABEL, None)?;
    let tls = match conn {
        rustls::Connection::Client(c) => Tls::Client(StreamOwned::new(c, sock)),
        rustls::Connection::Server(s) => Tls::Server(StreamOwned::new(s, sock)),
    };
    Ok(Channel { tls, peer, binding })
}

fn set_timeouts(sock: &TcpStream, timeout: Duration) -> Result<()> {
    sock.set_read_timeout(Some(timeout))?;
    sock.set_write_timeout(Some(timeout))?;
    Ok(())
}

impl Channel {
    /// The device at the other end, as its certificate names it.
    pub fn peer(&self) -> &DeviceId {
        &self.peer
    }

    pub fn role(&self) -> Role {
        match self.tls {
            Tls::Client(_) => Role::Connector,
            Tls::Server(_) => Role::Acceptor,
        }
    }

    /// Thirty-two bytes exported from this session's TLS secrets. Both ends
    /// hold the same ones, and nobody else does; every session has its own,
    /// and so do the two halves of an interception, since each half is a
    /// handshake of its own. Anything signed over it is good for this
    /// connection only.
    pub fn binding(&self) -> &[u8; 32] {
        &self.binding
    }

    /// How long one read or write may stall from now on.
    #[cfg(test)]
    pub fn set_timeout(&self, timeout: Duration) -> Result<()> {
        set_timeouts(self.tls.socket(), timeout)
    }

    /// Send one whole message: its length as a big-endian `u32`, then the
    /// bytes in frames of at most [`MAX_FRAME`], each behind its own length.
    pub fn send(&mut self, message: &[u8]) -> Result<()> {
        let total = u32::try_from(message.len()).map_err(|_| Error::TooLarge {
            len: message.len(),
            max: u32::MAX as usize,
        })?;
        self.tls.write_all(&total.to_be_bytes())?;
        for frame in message.chunks(MAX_FRAME) {
            self.tls.write_all(&(frame.len() as u32).to_be_bytes())?;
            self.tls.write_all(frame)?;
        }
        self.tls.flush()?;
        Ok(())
    }

    /// Receive one whole message of at most `max_len` bytes. A longer one is
    /// refused on its declared length, before anything is allocated for it,
    /// and the buffer grows only as frames arrive, so a peer that declares a
    /// long message and stops costs what it sent.
    pub fn recv(&mut self, max_len: usize) -> Result<Vec<u8>> {
        let mut total = [0u8; 4];
        self.tls.read_exact(&mut total)?;
        let total = u32::from_be_bytes(total) as usize;
        if total > max_len {
            return Err(Error::TooLarge {
                len: total,
                max: max_len,
            });
        }
        let mut message = Vec::new();
        while message.len() < total {
            let mut len = [0u8; 4];
            self.tls.read_exact(&mut len)?;
            let len = u32::from_be_bytes(len) as usize;
            if len == 0 || len > MAX_FRAME || len > total - message.len() {
                return Err(Error::Protocol(format!(
                    "a frame of {len} bytes in a message of {total}"
                )));
            }
            let start = message.len();
            message.resize(start + len, 0);
            self.tls.read_exact(&mut message[start..])?;
        }
        Ok(message)
    }

    pub fn send_control(&mut self, message: &Control) -> Result<()> {
        let bytes = serde_json::to_vec(message)
            .map_err(|e| Error::Protocol(format!("a control message that cannot be sent: {e}")))?;
        self.send(&bytes)
    }

    /// The next control message. One whose `t` this build does not speak is
    /// [`Control::Unsupported`], and the channel stays usable.
    pub fn recv_control(&mut self) -> Result<Control> {
        parse_control(&self.recv(MAX_CONTROL)?)
    }
}

fn parse_control(bytes: &[u8]) -> Result<Control> {
    serde_json::from_slice(bytes)
        .map_err(|e| Error::Protocol(format!("a malformed control message: {e}")))
}

impl fmt::Debug for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Channel")
            .field("peer", &self.peer)
            .field("role", &self.role())
            .finish()
    }
}

/// The TLS stream, as whichever end of the handshake this side was.
enum Tls {
    Client(StreamOwned<ClientConnection, TcpStream>),
    Server(StreamOwned<ServerConnection, TcpStream>),
}

impl Tls {
    #[cfg(test)]
    fn socket(&self) -> &TcpStream {
        match self {
            Self::Client(s) => &s.sock,
            Self::Server(s) => &s.sock,
        }
    }
}

impl Read for Tls {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Client(s) => s.read(buf),
            Self::Server(s) => s.read(buf),
        }
    }
}

impl Write for Tls {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Client(s) => s.write(buf),
            Self::Server(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Client(s) => s.flush(),
            Self::Server(s) => s.flush(),
        }
    }
}

/// The verifier both ends use. A device's certificate is self-signed and the
/// key in it is the whole of its identity, so there is no chain to check: any
/// one certificate is taken as the peer's, and the handshake signature is
/// then verified against the key in it with the provider's own algorithms,
/// as any verifier does. That signature is what proves the peer holds the key
/// its device id names. Ed25519 is the only scheme offered or accepted.
#[derive(Debug)]
struct PeerVerifier {
    algorithms: WebPkiSupportedAlgorithms,
    expect: Option<DeviceId>,
}

impl PeerVerifier {
    fn new(provider: &CryptoProvider, expect: Option<DeviceId>) -> Self {
        Self {
            algorithms: provider.signature_verification_algorithms,
            expect,
        }
    }

    fn check_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
    ) -> std::result::Result<(), rustls::Error> {
        if !intermediates.is_empty() {
            return Err(rustls::Error::General(
                "a nearby device presents one certificate".into(),
            ));
        }
        let id = DeviceId::of(end_entity)?;
        match self.expect {
            Some(expect) if expect != id => Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            )),
            _ => Ok(()),
        }
    }

    fn check_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        if dss.scheme != SignatureScheme::ED25519 {
            return Err(PeerMisbehaved::SignedHandshakeWithUnadvertisedSigScheme.into());
        }
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }
}

// Never reached: both configurations allow TLS 1.3 alone.
fn no_tls12() -> rustls::Error {
    rustls::Error::General("TLS 1.2 is not spoken on the nearby channel".into())
}

impl ServerCertVerifier for PeerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        self.check_cert(end_entity, intermediates)?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(no_tls12())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.check_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

impl ClientCertVerifier for PeerVerifier {
    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        self.check_cert(end_entity, intermediates)?;
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(no_tls12())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.check_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

/// A control message: JSON tagged `"t": "<area>/<name>"`. The areas are
/// `hello`, `trust`, `sync`, `setup` and `share`; this build speaks the first
/// three. Fields a newer peer adds to a message are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum Control {
    /// What each side sends first: the protocol version it speaks, its name
    /// and platform for the other to show, and the areas it understands.
    #[serde(rename = "hello/hello")]
    Hello {
        version: u32,
        name: String,
        platform: String,
        areas: Vec<String>,
    },
    /// A proof of a lockbox key for the vault `vault` (`trust::own_proof`).
    #[serde(rename = "trust/proof")]
    Proof {
        vault: String,
        #[serde(with = "hex32")]
        proof: [u8; 32],
    },
    /// A proof made with a key the sender has retired (`trust::own_proof` over
    /// that key): it holds the lockbox under a newer password now, and a
    /// receiver this proof verifies for is the one behind.
    #[serde(rename = "trust/proof-retired")]
    RetiredProof {
        vault: String,
        #[serde(with = "hex32")]
        proof: [u8; 32],
    },
    /// The sender has sent every proof it is going to on this connection.
    #[serde(rename = "trust/done")]
    ProofsDone,
    /// Every device the sender refuses, by id. Each side keeps the union.
    #[serde(rename = "sync/removed")]
    Removed { ids: Vec<String> },
    /// From the side that runs: the answering side's revision of `vault` and
    /// its clock, please. `now_ms` is the asker's clock.
    #[serde(rename = "sync/head")]
    Head { vault: String, now_ms: i64 },
    /// The answer to `sync/head`.
    #[serde(rename = "sync/head-reply")]
    HeadReply { revision: String, now_ms: i64 },
    /// The answering side's pack of `vault`, please.
    #[serde(rename = "sync/fetch")]
    Fetch { vault: String },
    /// The answer to `sync/fetch`, at `revision`; the pack is the next message.
    #[serde(rename = "sync/pack")]
    Pack { revision: String },
    /// A pack of `vault` for the answering side to merge; it is the next
    /// message.
    #[serde(rename = "sync/absorb")]
    Absorb { vault: String },
    /// The answer to `sync/absorb`: the revision after the merge.
    #[serde(rename = "sync/absorbed")]
    Absorbed { revision: String },
    /// The run of `vault` ended, and what it failed with if it did.
    #[serde(rename = "sync/done")]
    Done {
        vault: String,
        error: Option<Failure>,
    },
    /// The answer to a run the answering side will not take on now: another
    /// run holds its claim, or it paused the asker. Asked again later.
    #[serde(rename = "sync/busy")]
    Busy,
    /// The answer to a request that failed on the answering side.
    #[serde(rename = "sync/error")]
    Failed(Failure),
    /// A `t` this build does not speak: an area or a message added later.
    /// Received, never sent, and not an error, so a newer peer does not break
    /// its connection to an older one.
    #[serde(other, skip_serializing)]
    Unsupported,
}

/// An error that crossed the channel: the app's error kind
/// (`crate::error::Error::kind`) and its sentence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub kind: String,
    pub message: String,
}

impl Control {
    // The variant's name only: its fields can hold proofs, ids and names.
    pub fn kind(&self) -> String {
        let debug = format!("{self:?}");
        let end = debug
            .find(|c: char| !c.is_alphanumeric())
            .unwrap_or(debug.len());
        debug[..end].to_string()
    }

    /// This build's `hello`, under the device name `name`.
    pub fn hello(name: &str) -> Self {
        Self::Hello {
            version: PROTOCOL_VERSION,
            name: name.to_owned(),
            platform: std::env::consts::OS.to_owned(),
            areas: AREAS.iter().map(|area| area.to_string()).collect(),
        }
    }
}

// Thirty-two bytes as hex in a control message.
mod hex32 {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let text = String::deserialize(d)?;
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(&text, &mut bytes).map_err(D::Error::custom)?;
        Ok(bytes)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::nearby::identity::tests::identity;
    use std::net::TcpListener;
    use std::time::Instant;

    #[test]
    fn a_message_names_its_kind_without_its_fields() {
        let proof = Control::Proof {
            vault: "v".into(),
            proof: [7; 32],
        };
        assert_eq!(proof.kind(), "Proof");
        assert_eq!(Control::ProofsDone.kind(), "ProofsDone");
    }

    /// A connected pair over loopback: (the connector's end, the acceptor's).
    pub fn pair(connector: &Identity, acceptor: &Identity) -> (Channel, Channel) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::scope(|s| {
            let answered = s.spawn(|| accept(listener.accept().unwrap().0, acceptor));
            let dialled = connect(addr, connector, None).unwrap();
            (dialled, answered.join().unwrap().unwrap())
        })
    }

    /// Run a raw rustls client against `accept`; the acceptor's result.
    fn accept_raw_client(config: ClientConfig) -> Result<Channel> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let acceptor = identity();
        std::thread::scope(|s| {
            let answered = s.spawn(|| accept(listener.accept().unwrap().0, &acceptor));
            let mut sock = TcpStream::connect(addr).unwrap();
            let name = ServerName::try_from("rowel-nearby").unwrap();
            let mut conn = ClientConnection::new(Arc::new(config), name).unwrap();
            // Whatever the client makes of it, the acceptor's verdict is the
            // one under test; the client hangs up after one attempt.
            let _ = conn.complete_io(&mut sock);
            drop(sock);
            answered.join().unwrap()
        })
    }

    #[test]
    fn both_ends_see_each_other_and_share_a_binding() {
        let (a, b) = (identity(), identity());
        let (mut dialled, mut answered) = pair(&a, &b);

        assert_eq!(dialled.peer(), b.device_id());
        assert_eq!(answered.peer(), a.device_id());
        assert_eq!(dialled.role(), Role::Connector);
        assert_eq!(answered.role(), Role::Acceptor);
        assert_eq!(dialled.binding(), answered.binding());

        dialled.send(b"ping").unwrap();
        assert_eq!(answered.recv(16).unwrap(), b"ping");
        answered.send(b"").unwrap();
        assert_eq!(dialled.recv(16).unwrap(), b"");

        // Same two devices, a second session: a binding of its own.
        let (again, _) = pair(&a, &b);
        assert_ne!(again.binding(), dialled.binding());
    }

    #[test]
    fn connect_expecting_another_device_is_refused() {
        let (a, b, stranger) = (identity(), identity(), identity());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::scope(|s| {
            let answered = s.spawn(|| accept(listener.accept().unwrap().0, &b));
            let dialled = connect(addr, &a, Some(stranger.device_id()));
            assert!(matches!(dialled, Err(Error::UnexpectedPeer)), "{dialled:?}");
            assert!(answered.join().unwrap().is_err());
        });

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::scope(|s| {
            let answered = s.spawn(|| accept(listener.accept().unwrap().0, &b));
            let dialled = connect(addr, &a, Some(b.device_id())).unwrap();
            assert_eq!(dialled.peer(), b.device_id());
            assert!(answered.join().unwrap().is_ok());
        });
    }

    #[test]
    fn a_tls12_client_is_refused() {
        let provider = provider().unwrap();
        let client = identity();
        let mut config = ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS12])
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PeerVerifier::new(&provider, None)))
            .with_client_cert_resolver(Arc::new(SingleCertAndKey::from(client.certified_key())));
        config.alpn_protocols = vec![ALPN.to_vec()];
        assert!(accept_raw_client(config).is_err());
    }

    #[test]
    fn a_client_without_a_certificate_is_refused() {
        let provider = provider().unwrap();
        let mut config = ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PeerVerifier::new(&provider, None)))
            .with_no_client_auth();
        config.alpn_protocols = vec![ALPN.to_vec()];
        assert!(accept_raw_client(config).is_err());
    }

    // Someone else's certificate, which anyone can copy, with a handshake
    // signed by a key of one's own: the signature check is what refuses it.
    #[test]
    fn a_certificate_presented_without_its_key_is_refused() {
        let provider = provider().unwrap();
        let (victim, impostor) = (identity(), identity());
        let borrowed = rustls::sign::CertifiedKey::new(
            victim.certified_key().cert.clone(),
            impostor.certified_key().key.clone(),
        );
        let mut config = ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PeerVerifier::new(&provider, None)))
            .with_client_cert_resolver(Arc::new(SingleCertAndKey::from(borrowed)));
        config.alpn_protocols = vec![ALPN.to_vec()];
        assert!(matches!(
            accept_raw_client(config),
            Err(Error::Tls(rustls::Error::InvalidCertificate(
                CertificateError::BadSignature
            )))
        ));
    }

    #[test]
    fn a_message_of_several_megabytes_crosses_in_frames() {
        let (a, b) = (identity(), identity());
        let (mut dialled, mut answered) = pair(&a, &b);
        let message: Vec<u8> = (0..3 * MAX_FRAME + 12345)
            .map(|i| (i % 251) as u8)
            .collect();
        std::thread::scope(|s| {
            s.spawn(|| dialled.send(&message).unwrap());
            assert_eq!(answered.recv(4 * MAX_FRAME).unwrap(), message);
        });
    }

    // Only a header is sent: a receiver that allocated or read on before
    // refusing would wait out its deadline instead of answering at once.
    #[test]
    fn a_declared_length_over_the_limit_is_refused_on_the_header() {
        let (a, b) = (identity(), identity());
        let (mut dialled, mut answered) = pair(&a, &b);
        dialled.tls.write_all(&u32::MAX.to_be_bytes()).unwrap();
        dialled.tls.flush().unwrap();
        answered.set_timeout(Duration::from_secs(5)).unwrap();
        let started = Instant::now();
        assert!(matches!(
            answered.recv(MAX_FRAME),
            Err(Error::TooLarge { len, max }) if len == u32::MAX as usize && max == MAX_FRAME
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_frame_longer_than_its_message_is_refused() {
        let (a, b) = (identity(), identity());
        let (mut dialled, mut answered) = pair(&a, &b);
        dialled.tls.write_all(&4u32.to_be_bytes()).unwrap();
        dialled.tls.write_all(&5u32.to_be_bytes()).unwrap();
        dialled.tls.flush().unwrap();
        assert!(matches!(answered.recv(16), Err(Error::Protocol(_))));
    }

    #[test]
    fn a_stalled_peer_hits_the_read_deadline() {
        let (a, b) = (identity(), identity());
        let (_dialled, mut answered) = pair(&a, &b);
        answered.set_timeout(Duration::from_millis(200)).unwrap();
        let started = Instant::now();
        assert!(matches!(answered.recv(16), Err(Error::TimedOut)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn an_unknown_control_message_is_unsupported_and_the_channel_goes_on() {
        let (a, b) = (identity(), identity());
        let (mut dialled, mut answered) = pair(&a, &b);
        dialled
            .send(br#"{"t":"setup/offer","vault":"v1"}"#)
            .unwrap();
        dialled.send(br#"{"t":"trust/something-new"}"#).unwrap();
        dialled.send_control(&Control::hello("Laptop")).unwrap();

        assert_eq!(answered.recv_control().unwrap(), Control::Unsupported);
        assert_eq!(answered.recv_control().unwrap(), Control::Unsupported);
        match answered.recv_control().unwrap() {
            Control::Hello {
                version,
                name,
                areas,
                ..
            } => {
                assert_eq!(version, PROTOCOL_VERSION);
                assert_eq!(name, "Laptop");
                assert_eq!(areas, ["hello", "trust", "sync"]);
            }
            other => panic!("expected hello, got {other:?}"),
        }
    }

    #[test]
    fn a_sync_message_crosses_with_its_failure() {
        let (a, b) = (identity(), identity());
        let (mut dialled, mut answered) = pair(&a, &b);
        let done = Control::Done {
            vault: "v1".into(),
            error: Some(Failure {
                kind: "clockSkew".into(),
                message: "6 minutes apart".into(),
            }),
        };
        dialled.send_control(&done).unwrap();
        assert_eq!(answered.recv_control().unwrap(), done);
        let json = serde_json::to_value(Control::Failed(Failure {
            kind: "other".into(),
            message: "no".into(),
        }))
        .unwrap();
        assert_eq!(json["t"], "sync/error");
        assert_eq!(json["kind"], "other");
    }

    // What is not a control message at all is an error, not "unsupported".
    #[test]
    fn a_malformed_control_message_is_an_error() {
        for bytes in [
            &br#"{"no":"tag"}"#[..],
            b"not json",
            br#"{"t":"trust/proof","vault":"v","proof":"zz"}"#,
        ] {
            let (a, b) = (identity(), identity());
            let (mut dialled, mut answered) = pair(&a, &b);
            dialled.send(bytes).unwrap();
            assert!(matches!(answered.recv_control(), Err(Error::Protocol(_))));
        }
    }

    #[test]
    fn control_messages_are_tagged_by_area_and_name() {
        let json = serde_json::to_value(Control::Proof {
            vault: "v1".into(),
            proof: [0xab; 32],
        })
        .unwrap();
        assert_eq!(json["t"], "trust/proof");
        assert_eq!(json["proof"], "ab".repeat(32));
        assert!(serde_json::to_vec(&Control::Unsupported).is_err());
    }
}
