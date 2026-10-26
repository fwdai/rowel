//! This device's enrollment key: the one X25519 keypair an install generates,
//! whose public half a Rowel Server wraps org lockbox masters to.
//!
//! One curve, one job. The key only ever receives sealed boxes — a challenge
//! at enrollment, a master once approved, a new master at every rotation —
//! and signs nothing; a member proves who they are to the server through the
//! sign-in, and a device proves it is this device by being able to open what
//! was sealed to it, which is what answering the challenge shows the server.
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
use zeroize::Zeroizing;

use crate::crypto;
use crate::error::{Error, Result};
use crate::storage;

/// The id the sealed file is bound to (`crypto::seal_workspace_key`): a
/// sidecar moved under another name does not open.
const SEAL_ID: &str = "enrollment";

/// An org lockbox master is the vault key's 32 bytes.
const MASTER_LEN: usize = 32;

/// The random secret of an enrollment challenge, behind its context.
#[cfg(any(desktop, test))]
const CHALLENGE_LEN: usize = 32;

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
    #[cfg(any(desktop, test))]
    pub(crate) fn public_key_b64(&self) -> String {
        STANDARD.encode(self.0.public_key().as_bytes())
    }

    /// The public key as a person compares it ([`fingerprint_of`]).
    pub(crate) fn fingerprint(&self) -> String {
        crypto::key_fingerprint_bytes(self.0.public_key().as_bytes())
    }

    /// The master inside a sealed box the server wrapped to this key
    /// (`Lockbox::wrapped_master`). Anything that opens to other than 32
    /// bytes is refused: it is not a master, whoever sealed it.
    pub(crate) fn unseal_master(&self, sealed_b64: &str) -> Result<Zeroizing<Vec<u8>>> {
        let master = self.open_sealed("the wrapped master", sealed_b64)?;
        if master.len() != MASTER_LEN {
            return Err(Error::Crypto(format!(
                "the wrapped master is {} bytes, not {MASTER_LEN}",
                master.len()
            )));
        }
        Ok(master)
    }

    /// The secret inside an enrollment challenge the server at `server_url`
    /// (as `connections::normalize_url` spells it) sealed to this key, which
    /// `ServerClient::enroll` hands back as the proof that this app holds the
    /// key. Only a challenge for that server is opened: the context
    /// `ENROLL_CHALLENGE_CONTEXT`, then `server_url` and a NUL, then the 32
    /// bytes. A wrapped master sent in its place is refused rather than handed
    /// back in the clear, and so is another server's challenge, which a
    /// server this app also signs in to could otherwise relay to it and pass
    /// the answer off as this device's there.
    #[cfg(any(desktop, test))]
    pub(crate) fn open_challenge(
        &self,
        sealed_b64: &str,
        server_url: &str,
    ) -> Result<Zeroizing<Vec<u8>>> {
        let not_one = || Error::Crypto("the enrollment challenge is not one".into());
        let opened = self.open_sealed("the enrollment challenge", sealed_b64)?;
        let named = opened
            .strip_prefix(crypto::ENROLL_CHALLENGE_CONTEXT)
            .filter(|named| named.len() > CHALLENGE_LEN)
            .ok_or_else(not_one)?;
        let (head, secret) = named.split_at(named.len() - CHALLENGE_LEN);
        let url = head.strip_suffix(&[0]).ok_or_else(not_one)?;
        if url != server_url.as_bytes() {
            return Err(Error::Crypto(format!(
                "the enrollment challenge was issued for another server than {server_url}; \
                 nothing was sent back"
            )));
        }
        Ok(Zeroizing::new(secret.to_vec()))
    }

    /// What `sealed_b64`, named `what` in any error, opens to under this key.
    fn open_sealed(&self, what: &str, sealed_b64: &str) -> Result<Zeroizing<Vec<u8>>> {
        let sealed = STANDARD
            .decode(sealed_b64.trim())
            .map_err(|e| Error::Crypto(format!("{what} is not base64: {e}")))?;
        Ok(Zeroizing::new(self.0.unseal(&sealed).map_err(|_| {
            Error::Crypto(format!(
                "{what} does not open under this device's enrollment key"
            ))
        })?))
    }
}

/// A public key as a person compares it, of the base64 the server lists a
/// device's key as (`client::Device::public_key`): the one rule in
/// `rowel_core::crypto::key_fingerprint`, shared with the server's command
/// line so every screen shows the same digits.
pub(crate) fn fingerprint_of(public_key_b64: &str) -> Result<String> {
    crypto::key_fingerprint(public_key_b64)
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

/// `plaintext` sealed to the device whose enrollment public key is
/// `public_key_b64`, as the server seals a master or a challenge: the fake
/// server's half of [`EnrollmentKey::unseal_master`] and
/// [`EnrollmentKey::open_challenge`].
#[cfg(test)]
pub(crate) fn seal_to(public_key_b64: &str, plaintext: &[u8]) -> Result<String> {
    let bytes: [u8; 32] = STANDARD
        .decode(public_key_b64)
        .map_err(|e| Error::Crypto(e.to_string()))?
        .try_into()
        .map_err(|_| Error::Crypto("a device public key is 32 bytes".into()))?;
    let sealed = crypto_box::PublicKey::from(bytes)
        .seal(&mut rand::thread_rng(), plaintext)
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
        // The wrong-key verdict passes through as it is: the one failure a
        // caller may act on (`org::set_aside_stale`).
        let secret =
            crypto::open_workspace_key(app_key, SEAL_ID, &sealed).map_err(|e| match e {
                Error::SealedUnderAnotherKey => e,
                e => Error::Crypto(format!(
                    "this device's enrollment key does not open under the app key: {e}"
                )),
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

    // The proof a challenge is answered with is what it opened to, so a
    // master sent as a challenge must not open as one — the context sets the
    // two apart, and the length alone does not — and neither may a challenge
    // another server issued.
    #[test]
    fn a_challenge_opens_to_its_secret_for_its_server_alone() {
        const SERVER: &str = "https://rowel.acme.example";
        let key = EnrollmentKey::generate();
        let secret = [9u8; 32];
        let challenge_for = |url: &str| {
            let plaintext = [
                crypto::ENROLL_CHALLENGE_CONTEXT,
                url.as_bytes(),
                &[0],
                &secret,
            ];
            seal_to(&key.public_key_b64(), &plaintext.concat()).unwrap()
        };
        let sealed = challenge_for(SERVER);
        assert_eq!(&*key.open_challenge(&sealed, SERVER).unwrap(), &secret[..]);
        assert!(key.unseal_master(&sealed).is_err(), "not a master");
        assert!(EnrollmentKey::generate()
            .open_challenge(&sealed, SERVER)
            .is_err());

        for elsewhere in [
            "https://rowel.evil.example",
            "https://rowel.acme.example/x",
            "",
        ] {
            let refused = key.open_challenge(&challenge_for(elsewhere), SERVER);
            assert!(refused.is_err(), "{elsewhere:?}");
        }
        let master = seal_to(&key.public_key_b64(), &[42u8; 32]).unwrap();
        assert!(
            key.open_challenge(&master, SERVER).is_err(),
            "not a challenge"
        );
        let unlabelled = [SERVER.as_bytes(), &[0], &secret].concat();
        let unlabelled = seal_to(&key.public_key_b64(), &unlabelled).unwrap();
        assert!(
            key.open_challenge(&unlabelled, SERVER).is_err(),
            "no context"
        );
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

    // A password change moves the sealed key onto the new app key, and under
    // the old one it reads as sealed under another key: the verdict
    // `org::set_aside_stale` acts on, which a load never acts on itself.
    #[test]
    fn the_sealed_key_follows_the_app_key() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(storage::ENROLLMENT_KEY_FILE);
        reseal(root.path(), APP_KEY, &[5u8; 32]).unwrap();
        assert!(!path.exists(), "nothing to re-seal is not an error");

        let minted = load_or_create(root.path(), APP_KEY).unwrap();
        reseal(root.path(), APP_KEY, &[5u8; 32]).unwrap();
        assert!(matches!(
            load_or_create(root.path(), APP_KEY),
            Err(Error::SealedUnderAnotherKey)
        ));
        assert!(path.exists(), "a load that fails never replaces the key");
        assert_eq!(
            load_or_create(root.path(), &[5u8; 32])
                .unwrap()
                .public_key_b64(),
            minted.public_key_b64()
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
