//! This install's Ed25519 key, the self-signed certificate made over it at each load, and its device id.

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

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DeviceId([u8; 32]);

impl DeviceId {
    pub fn of(cert: &CertificateDer<'_>) -> std::result::Result<Self, rustls::Error> {
        let spki = ParsedCertificate::try_from(cert)?.subject_public_key_info();
        Ok(Self(Sha256::digest(spki.as_ref()).into()))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

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

impl FromStr for DeviceId {
    type Err = Error;

    // Only the displayed form: two spellings of one id would be two ids.
    fn from_str(text: &str) -> Result<Self> {
        let mut id = [0u8; 32];
        match hex::decode_to_slice(text, &mut id) {
            Ok(()) if !text.bytes().any(|b| b.is_ascii_uppercase()) => Ok(Self(id)),
            _ => Err(Error::Malformed("a device id is 64 lowercase hex digits")),
        }
    }
}

pub struct Identity {
    id: DeviceId,
    key: Arc<CertifiedKey>,
}

impl Identity {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        let pkcs8 = match std::fs::read(path) {
            Ok(bytes) => Zeroizing::new(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => create(path)?,
            Err(e) => return Err(identity(e)),
        };
        let der = PrivatePkcs8KeyDer::from(pkcs8.as_slice());
        let cert = self_signed(&der)?;
        let key = rustls::crypto::ring::sign::any_eddsa_type(&der)?;
        Ok(Self {
            id: DeviceId::of(&cert)?,
            key: Arc::new(CertifiedKey::new(vec![cert], key)),
        })
    }

    pub fn device_id(&self) -> &DeviceId {
        &self.id
    }

    pub(super) fn certified_key(&self) -> Arc<CertifiedKey> {
        self.key.clone()
    }
}

fn create(path: &Path) -> Result<Zeroizing<Vec<u8>>> {
    let key = Zeroizing::new(rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).map_err(identity)?);
    let pkcs8 = Zeroizing::new(key.serialized_der().to_vec());
    rowel_core::atomic::atomic_write_private(path, &pkcs8).map_err(identity)?;
    Ok(pkcs8)
}

fn self_signed(der: &PrivatePkcs8KeyDer<'_>) -> Result<CertificateDer<'static>> {
    let signer = Zeroizing::new(
        rcgen::KeyPair::from_pkcs8_der_and_sign_algo(der, &rcgen::PKCS_ED25519)
            .map_err(identity)?,
    );
    let mut params = rcgen::CertificateParams::default();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "Rowel");
    Ok(params
        .self_signed(&*signer)
        .map_err(identity)?
        .der()
        .clone())
}

fn identity(e: impl fmt::Display) -> Error {
    Error::Identity(e.to_string())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

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
