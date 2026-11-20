//! This install's identity on the local network: one Ed25519 key pair, kept in
//! an owner-only file and made on first use, and a self-signed certificate
//! over it, made again at every load. A device is known by its [`DeviceId`],
//! the SHA-256 of its certificate's SubjectPublicKeyInfo. That depends on the
//! key alone, so it stays the same across restarts although the certificate
//! is new each time.

use std::fmt;
use std::io;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use rustls::server::ParsedCertificate;
use rustls::sign::CertifiedKey;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::{Error, Result};

/// A device on the local network: the SHA-256 of its certificate's
/// SubjectPublicKeyInfo, written as 64 lowercase hex digits. Ordered by its
/// bytes, which is the order of its hex too: of two devices that see each
/// other, the smaller dials.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceId([u8; 32]);

impl DeviceId {
    /// The id of the device whose certificate this is.
    pub fn of(cert: &CertificateDer<'_>) -> std::result::Result<Self, rustls::Error> {
        let spki = ParsedCertificate::try_from(cert)?.subject_public_key_info();
        Ok(Self(Sha256::digest(spki.as_ref()).into()))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The id these 32 bytes are. Any bytes are an id; whether a device holds
    /// its key is for a handshake to prove.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceId({self})")
    }
}

/// Exactly the form [`DeviceId`] is displayed in, and nothing looser: an id
/// arrives from Settings and in another device's removed set, where two
/// spellings of one id would be two ids.
impl FromStr for DeviceId {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        let lower_hex = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b);
        if text.len() != 64 || !text.bytes().all(lower_hex) {
            return Err(Error::Malformed("a device id is 64 lowercase hex digits"));
        }
        let mut id = [0u8; 32];
        hex::decode_to_slice(text, &mut id)
            .map_err(|_| Error::Malformed("a device id is 64 lowercase hex digits"))?;
        Ok(Self(id))
    }
}

/// This install's key and certificate, ready to present in a handshake.
pub struct Identity {
    id: DeviceId,
    key: Arc<CertifiedKey>,
}

impl Identity {
    /// The identity kept at `path`, made there first if there is none. A file
    /// that is there but does not hold an Ed25519 key is an error, not
    /// replaced: a new key would be a new device to every device that trusts
    /// this one.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        let pkcs8 = match std::fs::read(path) {
            Ok(bytes) => Zeroizing::new(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let key = Zeroizing::new(
                    rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).map_err(identity)?,
                );
                let pkcs8 = Zeroizing::new(key.serialized_der().to_vec());
                rowel_core::atomic::atomic_write_private(path, &pkcs8).map_err(identity)?;
                pkcs8
            }
            Err(e) => return Err(identity(e)),
        };
        Self::from_pkcs8(&pkcs8)
    }

    // The certificate is signed by rcgen and the handshakes by rustls, each
    // from the same PKCS#8 bytes; rcgen's copy of them is scrubbed as it
    // drops. Naming the algorithm refuses any key that is not Ed25519.
    fn from_pkcs8(pkcs8: &[u8]) -> Result<Self> {
        let der = PrivatePkcs8KeyDer::from(pkcs8);
        let signer = Zeroizing::new(
            rcgen::KeyPair::from_pkcs8_der_and_sign_algo(&der, &rcgen::PKCS_ED25519)
                .map_err(identity)?,
        );
        let mut params = rcgen::CertificateParams::default();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Rowel");
        let cert = params
            .self_signed(&*signer)
            .map_err(identity)?
            .der()
            .clone();
        let id = DeviceId::of(&cert)?;
        let key = rustls::crypto::ring::sign::any_eddsa_type(&der)?;
        Ok(Self {
            id,
            key: Arc::new(CertifiedKey::new(vec![cert], key)),
        })
    }

    pub fn device_id(&self) -> &DeviceId {
        &self.id
    }

    /// The certificate and signing key, for a TLS configuration.
    pub(super) fn certified_key(&self) -> Arc<CertifiedKey> {
        self.key.clone()
    }
}

fn identity(e: impl fmt::Display) -> Error {
    Error::Identity(e.to_string())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A fresh identity in a directory of its own.
    pub fn identity() -> Identity {
        let dir = tempfile::tempdir().unwrap();
        Identity::load_or_create(&dir.path().join("nearby.key")).unwrap()
    }

    #[test]
    fn an_identity_is_made_once_and_keeps_its_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nearby.key");

        let first = Identity::load_or_create(&path).unwrap();
        let stored = std::fs::read(&path).unwrap();
        let again = Identity::load_or_create(&path).unwrap();

        assert_eq!(first.device_id(), again.device_id());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            stored,
            "loading never rewrites the key"
        );
        // A new certificate at every load, over the same key.
        assert_eq!(
            DeviceId::of(again.certified_key().end_entity_cert().unwrap()).unwrap(),
            *first.device_id()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn two_installs_are_two_devices() {
        assert_ne!(identity().device_id(), identity().device_id());
    }

    // A damaged file, or a key of another kind, is reported and left alone.
    #[test]
    fn a_file_that_is_not_an_ed25519_key_is_refused_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nearby.key");
        std::fs::write(&path, b"not a key").unwrap();
        assert!(matches!(
            Identity::load_or_create(&path),
            Err(Error::Identity(_))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"not a key");

        let p256 = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        std::fs::write(&path, p256.serialized_der()).unwrap();
        assert!(Identity::load_or_create(&path).is_err());
    }

    #[test]
    fn a_device_id_reads_back_only_in_its_own_form() {
        let id = *identity().device_id();
        let text = id.to_string();
        assert_eq!(text.len(), 64);
        assert_eq!(text.parse::<DeviceId>().unwrap(), id);

        assert!(text.to_uppercase().parse::<DeviceId>().is_err());
        assert!(text[..62].parse::<DeviceId>().is_err());
        assert!(format!("{text}00").parse::<DeviceId>().is_err());
        assert!(format!("{}g", &text[..63]).parse::<DeviceId>().is_err());
    }
}
