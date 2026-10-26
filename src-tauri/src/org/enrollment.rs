//! This device's enrollment key: the one X25519 keypair an install generates,
//! whose public half a Rowel Server wraps org lockbox masters to.
//!
//! One curve, one job. The key only ever receives sealed boxes — a master at
//! enrollment, a new master at every rotation — and signs nothing; a member
//! proves who they are to the server through the sign-in, and a device
//! proves it is this device by being able to open what was sealed to it.
//! Approving a new device is comparing its fingerprint on a device that is
//! already approved, which is why the fingerprint is short enough to read
//! aloud.
//!
//! The secret lives in one root-level file, sealed under the app key like
//! every non-primary workspace's key: without an unlocked vault there is no
//! enrollment key, so a stolen disk holds nothing a server would hand a
//! master to.

use std::path::Path;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use crypto_box::SecretKey;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::crypto;
use crate::error::{Error, Result};
use crate::storage;

/// The id the sealed file is bound to (`crypto::seal_workspace_key`): a
/// sidecar moved under another name does not open.
const SEAL_ID: &str = "enrollment";

/// An org lockbox master is the vault key's 32 bytes, and nothing else is
/// ever sealed to this key.
const MASTER_LEN: usize = 32;

pub(crate) struct EnrollmentKey(SecretKey);

impl EnrollmentKey {
    pub(crate) fn generate() -> Self {
        Self(SecretKey::generate(&mut rand::thread_rng()))
    }

    /// The key whose secret scalar these 32 bytes are.
    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| Error::Crypto("an enrollment key is 32 bytes".into()))?;
        Ok(Self(SecretKey::from_bytes(bytes)))
    }

    /// The secret scalar, scrubbed on drop.
    pub(crate) fn to_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.0.to_bytes())
    }

    /// The public half, as the server stores it and as `enroll` sends it.
    pub(crate) fn public_key_b64(&self) -> String {
        STANDARD.encode(self.0.public_key().as_bytes())
    }

    /// The public key as a person compares it ([`fingerprint_of`]).
    pub(crate) fn fingerprint(&self) -> String {
        fingerprint_bytes(self.0.public_key().as_bytes())
    }

    /// The master inside a sealed box the server wrapped to this key
    /// (`Lockbox::wrapped_master`). Anything that opens to other than 32
    /// bytes is refused: it is not a master, whoever sealed it.
    pub(crate) fn unseal_master(&self, sealed_b64: &str) -> Result<Zeroizing<Vec<u8>>> {
        let sealed = STANDARD
            .decode(sealed_b64.trim())
            .map_err(|e| Error::Crypto(format!("the wrapped master is not base64: {e}")))?;
        let master = Zeroizing::new(self.0.unseal(&sealed).map_err(|_| {
            Error::Crypto(
                "the wrapped master does not open under this device's enrollment key".into(),
            )
        })?);
        if master.len() != MASTER_LEN {
            return Err(Error::Crypto(format!(
                "the wrapped master is {} bytes, not {MASTER_LEN}",
                master.len()
            )));
        }
        Ok(master)
    }
}

/// A public key as a person compares it: SHA-256 of the 32 raw bytes, the
/// first 16 hex digits in four groups of four. Sixty-four bits is what a
/// member reads off one screen and checks against another; the full key is
/// what the server actually wraps to. Of the base64 the server lists a
/// device's key as (`client::Device::public_key`), so the device being
/// approved and the device approving derive the same digits from the same
/// bytes.
pub(crate) fn fingerprint_of(public_key_b64: &str) -> Result<String> {
    let bytes = STANDARD
        .decode(public_key_b64.trim())
        .map_err(|e| Error::Crypto(format!("a device public key is base64: {e}")))?;
    if bytes.len() != 32 {
        return Err(Error::Crypto("a device public key is 32 bytes".into()));
    }
    Ok(fingerprint_bytes(&bytes))
}

fn fingerprint_bytes(public_key: &[u8]) -> String {
    let digest = hex::encode(Sha256::digest(public_key));
    digest.as_bytes()[..16]
        .chunks(4)
        .map(|group| std::str::from_utf8(group).expect("hex is ASCII"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The sealed key under `new` instead of `old`: a master-password change
/// replaced the app key (`appkey::replace_app_key`). Nothing to do before
/// the key has been minted.
pub(crate) fn reseal(root: &Path, old: &[u8], new: &[u8]) -> Result<()> {
    let path = root.join(storage::ENROLLMENT_KEY_FILE);
    if !path.exists() {
        return Ok(());
    }
    let key = load_or_create(root, old)?;
    let sealed = crypto::seal_workspace_key(new, SEAL_ID, &*key.to_bytes())?;
    storage::atomic_write_private(&path, sealed.as_bytes())
}

/// A sealed key the current app key does not open was sealed under one this
/// device no longer has — what deleting the primary leaves behind — and no
/// server will ever hand a master to it again: it goes, with a note, and the
/// next sign-in enrolls a fresh key as a new device. The rule that a file
/// that does not open is never silently replaced ([`load_or_create`]) stands
/// everywhere else; this is the one caller that has proved the app key.
pub(crate) fn remove_if_stale(root: &Path, app_key: &[u8]) {
    let path = root.join(storage::ENROLLMENT_KEY_FILE);
    if !path.exists() || load_or_create(root, app_key).is_ok() {
        return;
    }
    log::warn!("this device's enrollment key does not open under the app key; removing it");
    let _ = storage::remove_if_present(&path);
}

/// `master` sealed to the device whose enrollment public key is
/// `public_key_b64`, as the server wraps one: the fake server's half of
/// [`EnrollmentKey::unseal_master`].
#[cfg(test)]
pub(crate) fn seal_to(public_key_b64: &str, master: &[u8]) -> Result<String> {
    let bytes: [u8; 32] = STANDARD
        .decode(public_key_b64)
        .map_err(|e| Error::Crypto(e.to_string()))?
        .try_into()
        .map_err(|_| Error::Crypto("a device public key is 32 bytes".into()))?;
    let sealed = crypto_box::PublicKey::from(bytes)
        .seal(&mut rand::thread_rng(), master)
        .map_err(|_| Error::Crypto("sealing to the device failed".into()))?;
    Ok(STANDARD.encode(sealed))
}

/// This install's enrollment key, generated on first use and sealed under
/// `app_key` in the root from then on.
///
/// A file that is there but does not open is an error, never a reason to
/// mint another: the server holds the public half of the key that was
/// enrolled, and a fresh one would leave every org lockbox wrapped to a key
/// this device no longer has, with nothing to say why.
pub(crate) fn load_or_create(root: &Path, app_key: &[u8]) -> Result<EnrollmentKey> {
    let path = root.join(storage::ENROLLMENT_KEY_FILE);
    if path.exists() {
        let sealed = std::fs::read_to_string(&path)?;
        let secret = crypto::open_workspace_key(app_key, SEAL_ID, &sealed).map_err(|e| {
            Error::Crypto(format!(
                "this device's enrollment key does not open under the app key: {e}"
            ))
        })?;
        return EnrollmentKey::from_bytes(&secret);
    }
    let key = EnrollmentKey::generate();
    let sealed = crypto::seal_workspace_key(app_key, SEAL_ID, &*key.to_bytes())?;
    storage::atomic_write_private(&path, sealed.as_bytes())?;
    Ok(key)
}

/// [`load_or_create`] for the running app: the root and the app key from the
/// ring, under the sidecars lock, so a password change replacing the app key
/// cannot interleave with the write of a file sealed under it.
pub(crate) fn load_or_create_for(app: &tauri::AppHandle) -> Result<EnrollmentKey> {
    use tauri::Manager;
    let state = app.state::<crate::state::AppState>();
    let root = storage::root_dir(app)?;
    let _sidecars = state.sidecars.lock().unwrap();
    let app_key = state
        .keyring
        .lock()
        .unwrap()
        .app_key()
        .ok_or(Error::Locked)?;
    load_or_create(&root, &app_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP_KEY: &[u8] = &[7u8; 32];

    #[test]
    fn a_master_sealed_to_the_public_key_opens_and_nothing_else_does() {
        let key = EnrollmentKey::generate();
        let master = [42u8; 32];
        let sealed = seal_to(&key.public_key_b64(), &master).unwrap();
        assert_eq!(&*key.unseal_master(&sealed).unwrap(), &master[..]);

        let short = seal_to(&key.public_key_b64(), &[1u8; 16]).unwrap();
        assert!(key.unseal_master(&short).is_err(), "not a master");
        let other = EnrollmentKey::generate();
        assert!(other.unseal_master(&sealed).is_err(), "not this device");
        assert!(key.unseal_master("not base64!").is_err());
    }

    #[test]
    fn the_secret_round_trips_through_its_bytes() {
        let key = EnrollmentKey::generate();
        let again = EnrollmentKey::from_bytes(&*key.to_bytes()).unwrap();
        assert_eq!(key.public_key_b64(), again.public_key_b64());
        assert!(EnrollmentKey::from_bytes(&[0u8; 31]).is_err());
    }

    // Pinned: a fingerprint is what two people compare across two screens, so
    // its derivation and its spelling must never drift between builds.
    #[test]
    fn the_fingerprint_is_pinned() {
        let key = EnrollmentKey::from_bytes(&[1u8; 32]).unwrap();
        assert_eq!(key.fingerprint(), "1a92 f238 52dc 908d");
        assert_eq!(key.fingerprint().len(), 19);
        // The same digits off the base64 the server lists the key as.
        assert_eq!(
            fingerprint_of(&key.public_key_b64()).unwrap(),
            key.fingerprint()
        );
        assert!(fingerprint_of("not base64!").is_err());
        assert!(fingerprint_of(&STANDARD.encode([1u8; 16])).is_err());
    }

    // A password change moves the sealed key onto the new app key; a key left
    // under one the device no longer has goes, and one that opens stays.
    #[test]
    fn the_sealed_key_follows_the_app_key_and_a_stale_one_is_removed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(storage::ENROLLMENT_KEY_FILE);
        reseal(root.path(), APP_KEY, &[5u8; 32]).unwrap();
        assert!(!path.exists(), "nothing to re-seal is not an error");

        let minted = load_or_create(root.path(), APP_KEY).unwrap();
        reseal(root.path(), APP_KEY, &[5u8; 32]).unwrap();
        assert!(load_or_create(root.path(), APP_KEY).is_err());
        assert_eq!(
            load_or_create(root.path(), &[5u8; 32])
                .unwrap()
                .public_key_b64(),
            minted.public_key_b64()
        );

        remove_if_stale(root.path(), &[5u8; 32]);
        assert!(path.exists(), "it opens, so it stays");
        remove_if_stale(root.path(), APP_KEY);
        assert!(
            !path.exists(),
            "sealed under a key this device no longer has"
        );
    }

    #[test]
    fn the_key_is_minted_once_and_sealed_under_the_app_key() {
        let root = tempfile::tempdir().unwrap();
        let first = load_or_create(root.path(), APP_KEY).unwrap();
        let path = root.path().join(storage::ENROLLMENT_KEY_FILE);
        assert!(path.exists());
        let second = load_or_create(root.path(), APP_KEY).unwrap();
        assert_eq!(first.public_key_b64(), second.public_key_b64());

        // Another app key does not open it, and does not replace it either.
        let before = std::fs::read(&path).unwrap();
        assert!(load_or_create(root.path(), &[9u8; 32]).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}
