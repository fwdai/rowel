//! The TLS 1.3 handshake between two self-signed identities, and the binding it yields.

use std::net::TcpStream;
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::Resumption;
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::NoServerSessionStorage;
use rustls::sign::SingleCertAndKey;
use rustls::{
    CertificateError, ClientConfig, ClientConnection, Connection, DigitallySignedStruct,
    DistinguishedName, PeerMisbehaved, ServerConfig, ServerConnection, SignatureScheme,
    StreamOwned, SupportedProtocolVersion,
};

use super::{Channel, Role, Stream, TIMEOUT};
use crate::nearby::identity::{DeviceId, Identity};
use crate::nearby::{Error, Result};

const ALPN: &[u8] = b"rowel-nearby/1";
const BINDING_LABEL: &[u8] = b"EXPORTER-rowel-nearby-binding";
// TLS 1.3 alone: its exporter is unique to the session, and certificates travel encrypted.
const TLS13: &[&SupportedProtocolVersion] = &[&rustls::version::TLS13];

type Verdict<T> = std::result::Result<T, rustls::Error>;

pub fn connect_over(
    sock: TcpStream,
    identity: &Identity,
    expect: Option<&DeviceId>,
) -> Result<Channel> {
    let provider = provider();
    let mut config = ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(TLS13)?
        .dangerous()
        .with_custom_certificate_verifier(PeerVerifier::new(&provider, expect.copied()))
        .with_client_cert_resolver(own_cert(identity));
    config.alpn_protocols = vec![ALPN.to_vec()];
    // A full handshake every time, so the verifier, and `expect` with it, sees each one.
    config.resumption = Resumption::disabled();
    config.enable_sni = false;
    let name = ServerName::try_from("rowel-nearby").expect("a valid DNS name");
    open(ClientConnection::new(Arc::new(config), name)?.into(), sock)
}

pub fn accept(sock: TcpStream, identity: &Identity) -> Result<Channel> {
    let provider = provider();
    let mut config = ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(TLS13)?
        .with_client_cert_verifier(PeerVerifier::new(&provider, None))
        .with_cert_resolver(own_cert(identity));
    config.alpn_protocols = vec![ALPN.to_vec()];
    config.session_storage = Arc::new(NoServerSessionStorage {});
    config.send_tls13_tickets = 0;
    open(ServerConnection::new(Arc::new(config))?.into(), sock)
}

fn provider() -> Arc<CryptoProvider> {
    crate::sync::install_crypto_provider();
    CryptoProvider::get_default()
        .cloned()
        .expect("a provider was just installed")
}

fn own_cert(identity: &Identity) -> Arc<SingleCertAndKey> {
    Arc::new(SingleCertAndKey::from(identity.certified_key()))
}

fn open(mut conn: Connection, mut sock: TcpStream) -> Result<Channel> {
    sock.set_nodelay(true)?;
    sock.set_read_timeout(Some(TIMEOUT))?;
    sock.set_write_timeout(Some(TIMEOUT))?;
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
    // Held by both ends and nobody else, new each session: key proofs are bound to it.
    let binding = conn.export_keying_material([0u8; 32], BINDING_LABEL, None)?;
    let (stream, role): (Box<dyn Stream>, _) = match conn {
        Connection::Client(c) => (Box::new(StreamOwned::new(c, sock)), Role::Connector),
        Connection::Server(s) => (Box::new(StreamOwned::new(s, sock)), Role::Acceptor),
    };
    Ok(Channel {
        stream,
        role,
        peer,
        binding,
    })
}

// No CA: a device is its key, so any self-signed certificate is taken; its signature is checked.
#[derive(Debug)]
struct PeerVerifier {
    algorithms: WebPkiSupportedAlgorithms,
    expect: Option<DeviceId>,
}

impl PeerVerifier {
    fn new(provider: &CryptoProvider, expect: Option<DeviceId>) -> Arc<Self> {
        Arc::new(Self {
            algorithms: provider.signature_verification_algorithms,
            expect,
        })
    }

    fn check_cert(
        &self,
        cert: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
    ) -> Verdict<()> {
        if !intermediates.is_empty() {
            return Err(rustls::Error::General(
                "a nearby device presents one certificate".into(),
            ));
        }
        let id = DeviceId::of(cert)?;
        if self.expect.is_some_and(|expect| expect != id) {
            return Err(CertificateError::ApplicationVerificationFailure.into());
        }
        Ok(())
    }
}

macro_rules! signature_checks {
    () => {
        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Verdict<HandshakeSignatureValid> {
            Err(rustls::Error::General(
                "TLS 1.2 is not spoken on the nearby channel".into(),
            ))
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Verdict<HandshakeSignatureValid> {
            if dss.scheme != SignatureScheme::ED25519 {
                return Err(PeerMisbehaved::SignedHandshakeWithUnadvertisedSigScheme.into());
            }
            rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![SignatureScheme::ED25519]
        }
    };
}

impl ServerCertVerifier for PeerVerifier {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Verdict<ServerCertVerified> {
        self.check_cert(cert, intermediates)
            .map(|()| ServerCertVerified::assertion())
    }

    signature_checks!();
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
        cert: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _: UnixTime,
    ) -> Verdict<ClientCertVerified> {
        self.check_cert(cert, intermediates)
            .map(|()| ClientCertVerified::assertion())
    }

    signature_checks!();
}

#[cfg(test)]
mod tests {
    use std::net::TcpStream;

    use rustls::sign::CertifiedKey;

    use super::*;
    use crate::nearby::channel::tests::{connect, meet, pair};
    use crate::nearby::identity::tests::identity;

    // A client `connect_over` would not be: the acceptor's verdict on it.
    fn accept_raw_client(
        versions: &[&'static SupportedProtocolVersion],
        cert: Option<Arc<CertifiedKey>>,
    ) -> Result<Channel> {
        let provider = provider();
        let builder = ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(versions)
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(PeerVerifier::new(&provider, None));
        let mut config = match cert {
            Some(cert) => builder.with_client_cert_resolver(Arc::new(SingleCertAndKey::from(cert))),
            None => builder.with_no_client_auth(),
        };
        config.alpn_protocols = vec![ALPN.to_vec()];
        let name = ServerName::try_from("rowel-nearby").unwrap();
        let mut conn = ClientConnection::new(Arc::new(config), name).unwrap();
        let (_, answered, _) = meet(&identity(), |addr| {
            let _ = conn.complete_io(&mut TcpStream::connect(addr).unwrap());
        });
        answered
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

        let (again, _) = pair(&a, &b);
        assert_ne!(again.binding(), dialled.binding());
    }

    #[test]
    fn connect_expecting_another_device_is_refused() {
        let (a, b, stranger) = (identity(), identity(), identity());
        let (dialled, answered, _) = meet(&b, |addr| connect(addr, &a, Some(stranger.device_id())));
        assert!(matches!(
            dialled,
            Err(Error::Tls(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure
            )))
        ));
        assert!(answered.is_err());

        let (dialled, answered, _) = meet(&b, |addr| connect(addr, &a, Some(b.device_id())));
        assert_eq!(dialled.unwrap().peer(), b.device_id());
        assert!(answered.is_ok());
    }

    #[test]
    fn a_tls12_client_is_refused() {
        let client = identity();
        let tls12 = &[&rustls::version::TLS12];
        assert!(accept_raw_client(tls12, Some(client.certified_key())).is_err());
    }

    #[test]
    fn a_client_without_a_certificate_is_refused() {
        assert!(accept_raw_client(TLS13, None).is_err());
    }

    #[test]
    fn a_certificate_presented_without_its_key_is_refused() {
        let (victim, impostor) = (identity(), identity());
        let borrowed = CertifiedKey::new(
            victim.certified_key().cert.clone(),
            impostor.certified_key().key.clone(),
        );
        assert!(matches!(
            accept_raw_client(TLS13, Some(Arc::new(borrowed))),
            Err(Error::Tls(rustls::Error::InvalidCertificate(
                CertificateError::BadSignature
            )))
        ));
    }
}
