//! OS secure store for the vault key material, biometric-gated.
//!
//! The stored value is opaque key material — today the `Cryptor` "secret"
//! string, tomorrow (Phase 2) an Argon2id-derived key — so this module never
//! interprets it. The biometric gate model differs per platform, and on macOS
//! it differs per *build*, so the gate in force is named by a [`GateMode`]:
//!
//! - [`GateMode::Protected`] (Apple only): a data-protection Keychain item with
//!   a `SecAccessControl` requiring biometry (`kSecAccessControlBiometryCurrentSet`).
//!   The OS enforces Touch ID / Face ID on *read* and auto-invalidates the item
//!   if the enrolled biometrics change. Adding such an item requires the
//!   `keychain-access-groups` entitlement, so it only works in a properly
//!   signed build — an ad-hoc-signed dev build gets `errSecMissingEntitlement`.
//!   On iOS the data-protection keychain is the only keychain, so the switch
//!   macOS needs is simply absent there.
//! - [`GateMode::Prompt`] (Apple only): verify-then-read. The app runs an
//!   explicit biometric check ([`crate::biometrics::authenticate`]) and only
//!   then reads the key from an ordinary login-keychain item. The gate is
//!   app-enforced rather than OS-enforced, which is all an unentitled macOS
//!   build can do — and the same model the legacy Electron app used via keytar.
//! - [`GateMode::HelloKey`] (Windows only): the key material is sealed under a
//!   wrapping key derived from a Windows Hello *key credential* signature, and
//!   only the sealed blob goes to Credential Manager. Producing that signature
//!   requires a Hello prompt the OS enforces, so the stored item is inert to
//!   anything that cannot pass it — unlike a verify-then-read item in Credential
//!   Manager, which has no gate of its own and is readable by any process
//!   running as the same user.
//! - **other (Linux, …):** unsupported — we report biometric unavailable rather
//!   than store a key that nothing can gate.
//!
//! The mode is decided **once, at enrollment** ([`KeyStore::store`]), recorded
//! by the caller alongside the enrollment marker, and passed back in on every
//! [`KeyStore::retrieve`]. Retrieval never re-probes and never tries the other
//! mode: silently degrading from an OS-enforced gate to an app-enforced one
//! would weaken the user's protection without telling them, and silently
//! *upgrading* would just fail to find the item. A mode mismatch surfaces as an
//! error the user can act on (re-enroll) instead of a quiet downgrade.

use crate::error::{Error, Result};
use zeroize::Zeroizing;

// Used only by the Apple/Windows key-store impls; absent on the unsupported
// fallback (Linux), so gate them to avoid a dead_code error there.
#[cfg(any(target_vendor = "apple", target_os = "windows"))]
const SERVICE: &str = "app.rowel.desktop.vault";
#[cfg(any(target_vendor = "apple", target_os = "windows"))]
const ACCOUNT: &str = "master-key";
// Separate account for the verify-then-read item on Apple platforms. The two
// modes carry different access control, so they must never be able to resolve
// each other's item: a distinct account makes a cross-mode read a clean
// `NotFound` rather than an item read under the wrong gate.
#[cfg(target_vendor = "apple")]
const ACCOUNT_PROMPT: &str = "master-key-prompt";

/// How an enrolled key is gated. Recorded at enrollment; never re-derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateMode {
    /// OS-enforced on read (macOS data-protection keychain + `SecAccessControl`).
    Protected,
    /// App-enforced: explicit biometric prompt, then a plain credential-store read.
    Prompt,
    /// Windows: sealed under a Windows Hello key-credential signature, so the
    /// stored blob cannot be opened without passing the Hello prompt.
    HelloKey,
}

impl GateMode {
    /// The marker value persisted next to the enrollment flag. Stable on disk —
    /// changing these strings orphans existing enrollments.
    pub fn as_marker(self) -> &'static str {
        match self {
            Self::Protected => "protected",
            Self::Prompt => "prompt",
            Self::HelloKey => "hello-key",
        }
    }

    /// Read a persisted marker. Anything unrecognised — including the legacy
    /// `"1"` marker written before modes existed — reads as the mode that build
    /// would have used, so an old enrollment keeps working (or fails loudly)
    /// rather than being reinterpreted under a gate it was never stored behind.
    pub fn from_marker(marker: &str) -> Self {
        match marker.trim() {
            "prompt" => Self::Prompt,
            "protected" => Self::Protected,
            "hello-key" => Self::HelloKey,
            _ => Self::LEGACY,
        }
    }

    // Pre-mode enrollments: macOS only ever wrote the protected item, every
    // other platform only ever wrote the verify-then-read one. iOS had no
    // pre-mode build at all, but shares macOS' enrollment path.
    #[cfg(target_vendor = "apple")]
    const LEGACY: Self = Self::Protected;
    #[cfg(not(target_vendor = "apple"))]
    const LEGACY: Self = Self::Prompt;
}

/// Abstraction over the platform key store so the non-interactive unlock logic
/// is unit-testable with an in-memory mock (biometric prompts can't run headlessly).
pub trait KeyStore {
    /// Store `key` biometry-gated, returning the [`GateMode`] actually used.
    /// Opt-in; call while unlocked. The caller must persist the returned mode.
    fn store(&self, key: &[u8]) -> Result<GateMode>;
    /// Retrieve the key through `mode`'s gate — and only that one.
    /// Fails with [`Error::NotFound`] when nothing is enrolled under it.
    fn retrieve(&self, mode: GateMode) -> Result<Zeroizing<Vec<u8>>>;
    /// Delete the stored key in *every* mode. Idempotent (a missing key is not
    /// an error), so disabling or re-enrolling can never orphan the other item.
    fn delete(&self) -> Result<()>;
}

/// The real, platform-backed key store.
pub struct Platform;

impl KeyStore for Platform {
    fn store(&self, key: &[u8]) -> Result<GateMode> {
        imp::store(key)
    }
    fn retrieve(&self, mode: GateMode) -> Result<Zeroizing<Vec<u8>>> {
        imp::retrieve(mode)
    }
    fn delete(&self) -> Result<()> {
        imp::delete()
    }
}

/// Whether this platform can biometric-gate the secure store at all.
pub fn is_supported() -> bool {
    imp::SUPPORTED && hello_key_store_available()
}

// Windows binds the stored material to a Hello *key credential*, so a device
// whose key store cannot mint one has no gate to offer even when
// `biometrics::is_available()` says a Hello prompt exists. Nowhere else has a
// second requirement, hence the constant `true`.
#[cfg(not(target_os = "windows"))]
fn hello_key_store_available() -> bool {
    true
}

#[cfg(target_os = "windows")]
fn hello_key_store_available() -> bool {
    imp::key_credentials_available()
}

// --- Windows Hello key wrapping ---------------------------------------------
//
// The platform-independent halves of the Windows path live here, uncfg'd from
// the OS, so the wrap/unwrap round trip is testable on the machines we develop
// on rather than only on Windows.

/// The message the Hello key credential signs. Fixed on purpose: the design
/// relies on Hello signing with RSA PKCS#1 v1.5 over SHA-256, a deterministic
/// scheme, so signing a constant yields a byte-identical signature every time —
/// which is what makes the derived wrapping key reproducible across unlocks.
///
/// Microsoft's documentation disagrees with itself on the scheme. The Windows
/// Hello developer guide says "We are using SHA256 as the hash algorithm and
/// Pkcs1 for SignaturePadding" and ships server code that verifies with
/// `RSASignaturePadding.Pkcs1`; the `KeyCredentialManager` class reference's
/// remarks say "PKCS #1 RSA PSS with SHA256", which is probabilistic and would
/// make this design unworkable. Practice sides with the guide: Bitwarden's
/// desktop client ships this same construction ("a signing API, that
/// deterministically signs a challenge, from which a windows hello key is
/// derived" — `desktop_native/biometric/src/windows.rs`). Still, enrollment
/// does not trust either page: it verifies the signature it just obtained as
/// PKCS#1 v1.5 against the credential's own public key
/// (`assert_pkcs1_signature`) and refuses to enroll otherwise, rather than
/// store a blob no later prompt could open. One enrollment on a real Windows
/// machine settles the question.
///
/// Nothing is secret about the challenge — the secrecy is the private key,
/// which lives in the TPM/Hello key store and only signs after a prompt.
#[cfg(target_os = "windows")]
const HELLO_CHALLENGE: &[u8] = b"rowel-biometric-v1";

/// HKDF label, distinct from every vault subkey label so this wrapping key can
/// never collide with the SQLCipher or payload key.
#[cfg(any(target_os = "windows", test))]
const HELLO_WRAP_INFO: &[u8] = b"rowel windows-hello wrap";

/// Bound into the wrap as associated data, so the blob can only ever be read
/// back as a Hello-wrapped master and not as some other AEAD payload sealed
/// under a colliding key. Empty today: this wrap is the only thing the
/// wrapping key seals, and keeping the format as it was means an enrolment
/// made before the AEAD grew an AAD parameter still unwraps.
#[cfg(any(target_os = "windows", test))]
const HELLO_WRAP_AAD: &[u8] = b"";

#[cfg(any(target_os = "windows", test))]
fn hello_wrapping_key(signature: &[u8]) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(crate::crypto::hkdf_subkey(signature, HELLO_WRAP_INFO))
}

/// Seal the key material under the signature-derived wrapping key. Only this
/// blob is handed to Credential Manager: on its own it is inert, because
/// reproducing the wrapping key needs a fresh Hello signature.
#[cfg(any(target_os = "windows", test))]
fn wrap_master(signature: &[u8], master: &[u8]) -> Result<Vec<u8>> {
    crate::crypto::seal_aead(&*hello_wrapping_key(signature), HELLO_WRAP_AAD, master)
}

/// Reverse of [`wrap_master`]. A blob that was tampered with, or a signature
/// from a different Hello key, fails the GCM tag rather than yielding garbage.
#[cfg(any(target_os = "windows", test))]
fn unwrap_master(signature: &[u8], blob: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    crate::crypto::unseal_aead(&*hello_wrapping_key(signature), HELLO_WRAP_AAD, blob)
        .map(Zeroizing::new)
}

/// Outcome of a protected-mode store attempt, classified so the enrollment
/// policy below can be expressed — and tested — without a real keychain.
#[cfg(any(target_vendor = "apple", test))]
enum ProtectedOutcome {
    Stored,
    /// The platform refused for lack of a code-signing entitlement. Recoverable:
    /// the build simply isn't signed for the data-protection keychain.
    NoEntitlement,
    /// Anything else — a real failure, not a capability gap.
    Failed(Error),
}

// Enrollment policy: prefer the OS-enforced gate, fall back to the app-enforced
// one *only* for the one recoverable reason (no entitlement). Every other
// failure propagates: a keychain that is broken rather than unentitled must not
// quietly hand the user a weaker gate than the one they'd otherwise have got.
#[cfg(any(target_vendor = "apple", test))]
fn enroll(
    protected: impl FnOnce() -> ProtectedOutcome,
    prompt: impl FnOnce() -> Result<()>,
) -> Result<GateMode> {
    match protected() {
        ProtectedOutcome::Stored => Ok(GateMode::Protected),
        ProtectedOutcome::NoEntitlement => prompt().map(|()| GateMode::Prompt),
        ProtectedOutcome::Failed(e) => Err(e),
    }
}

// The read policy for the iOS App Group's keychain access group, apart from
// the keychain so it can be tested without one. The item under the group
// (`shared`) is the answer whenever there is one. Only a clean miss there asks
// for the item an older install wrote outside the group (`legacy`) — a
// cancelled Face ID sheet or any other failure is the answer too, and must not
// put up a second sheet. Found there, it is moved into the group (`adopt`); a
// move that fails is logged and the key still returned, since the unlock the
// user asked for does not depend on it, and the next one tries again.
#[cfg(any(target_os = "ios", test))]
fn read_through_group(
    shared: impl FnOnce() -> Result<Zeroizing<Vec<u8>>>,
    legacy: impl FnOnce() -> Result<Zeroizing<Vec<u8>>>,
    adopt: impl FnOnce(&[u8]) -> Result<()>,
) -> Result<Zeroizing<Vec<u8>>> {
    match shared() {
        Err(Error::NotFound) => {
            let key = legacy()?;
            match adopt(&key) {
                Ok(()) => log::info!("moved the biometric key into the App Group's keychain group"),
                Err(e) => log::warn!(
                    "could not move the biometric key into the App Group's keychain group: {e}"
                ),
            }
            Ok(key)
        }
        found => found,
    }
}

#[cfg(target_vendor = "apple")]
mod imp {
    use super::*;
    use crate::biometrics;
    use security_framework::access_control::{ProtectionMode, SecAccessControl};
    use security_framework::passwords::{
        delete_generic_password_options, generic_password, set_generic_password_options,
        AccessControlOptions, PasswordOptions,
    };

    pub const SUPPORTED: bool = true;

    // errSecItemNotFound — no such keychain item.
    const ERR_ITEM_NOT_FOUND: i32 = -25300;
    // errSecUserCanceled — the user dismissed the authentication sheet the
    // protected item's SecAccessControl put up.
    const ERR_USER_CANCELED: i32 = -128;
    // errSecMissingEntitlement — the data-protection keychain rejected the item
    // because the binary lacks `keychain-access-groups`. Matched by OSStatus, not
    // by message text, which is localized.
    const ERR_MISSING_ENTITLEMENT: i32 = -34018;

    // Data-protection keychain: the only one that honours a biometric
    // SecAccessControl, and the only one that needs an entitlement. macOS has to
    // opt in; on iOS it is the only keychain there is, and the switch that opts
    // in does not exist.
    fn protected_options() -> PasswordOptions {
        #[allow(unused_mut)]
        let mut opts = shared(ACCOUNT);
        #[cfg(target_os = "macos")]
        opts.use_protected_keychain();
        opts
    }

    // The item for `account`. On iOS it is in the App Group's access group, so
    // the AutoFill extension can read the key the app enrolled; every write
    // names the group, and so does every read, which then finds nothing else.
    fn shared(account: &str) -> PasswordOptions {
        #[allow(unused_mut)]
        let mut opts = PasswordOptions::new_generic_password(SERVICE, account);
        #[cfg(target_os = "ios")]
        opts.set_access_group(rowel_core::app::APP_GROUP);
        opts
    }

    // The access control the protected item is stored behind: the biometric
    // constraint, and the protection class that says when the item is readable
    // at all.
    //
    // `set_access_control_options` would build this for us, but it hardcodes
    // `kSecAttrAccessibleWhenUnlocked` — no `ThisDeviceOnly`, so the item rides
    // along in an encrypted backup and can be restored onto another device.
    // `AccessibleWhenPasscodeSetThisDeviceOnly` is the strictest class there is:
    // the key never leaves this device, and it stops existing if the user
    // removes their passcode — which is the same moment biometrics stop meaning
    // anything.
    //
    // Changing the class does not strand an existing enrolment: `retrieve` and
    // `delete` query by service and account only — neither the class nor the
    // access control is part of the lookup — so an item written under the old
    // class still reads and still deletes. Re-enrolling (which `store` always
    // does, deleting first) moves it to the new one.
    fn access_control() -> Result<SecAccessControl> {
        SecAccessControl::create_with_protection(
            Some(ProtectionMode::AccessibleWhenPasscodeSetThisDeviceOnly),
            AccessControlOptions::BIOMETRY_CURRENT_SET.bits(),
        )
        .map_err(map_err)
    }

    // Ordinary keychain item: no access control, no entitlement, no OS gate.
    // The biometric check happens in `retrieve` before we ever read this.
    fn prompt_options() -> PasswordOptions {
        shared(ACCOUNT_PROMPT)
    }

    // The protected item, written. Adding never prompts; reading it back does.
    fn add_protected(key: &[u8]) -> std::result::Result<(), ProtectedOutcome> {
        let mut opts = protected_options();
        opts.set_access_control(access_control().map_err(ProtectedOutcome::Failed)?);
        set_generic_password_options(key, opts).map_err(|e| match e.code() {
            ERR_MISSING_ENTITLEMENT => ProtectedOutcome::NoEntitlement,
            _ => ProtectedOutcome::Failed(map_err(e)),
        })
    }

    fn add_prompt(key: &[u8]) -> Result<()> {
        set_generic_password_options(key, prompt_options()).map_err(map_err)
    }

    pub fn store(key: &[u8]) -> Result<GateMode> {
        // Clear both modes first. Adding a fresh item never prompts, whereas
        // *updating* a biometry-protected one would; and a leftover item in the
        // mode we don't end up using would outlive the enrollment it belongs to.
        let _ = delete();
        enroll(
            || match add_protected(key) {
                Ok(()) => ProtectedOutcome::Stored,
                Err(outcome) => outcome,
            },
            || add_prompt(key),
        )
    }

    pub fn retrieve(mode: GateMode) -> Result<Zeroizing<Vec<u8>>> {
        match mode {
            // Requesting the data triggers the OS Touch ID prompt via the stored
            // SecAccessControl (OS-enforced-on-read).
            GateMode::Protected => read(protected_options, ACCOUNT, |key| {
                add_protected(key).map_err(|outcome| match outcome {
                    ProtectedOutcome::Failed(e) => e,
                    _ => Error::Other("the protected keychain item cannot be written".into()),
                })
            }),
            // Verify-then-read: the gate is ours, so it must run first.
            GateMode::Prompt => {
                biometrics::authenticate()?;
                read(prompt_options, ACCOUNT_PROMPT, add_prompt)
            }
            // A Windows-only mode: no Apple build ever enrolls it, and reading
            // an enrollment under a gate it was not stored behind is exactly
            // what this module refuses to do.
            GateMode::HelloKey => Err(Error::NotFound),
        }
    }

    #[cfg(not(target_os = "ios"))]
    fn read(
        options: fn() -> PasswordOptions,
        _account: &str,
        _add: impl FnOnce(&[u8]) -> Result<()>,
    ) -> Result<Zeroizing<Vec<u8>>> {
        read_item(options())
    }

    // An install enrolled before the App Group has its item in the app's own
    // access group, where a query naming the shared one does not look. So a
    // miss there asks once more without the group, and an item found that way
    // is written again under the group (`add`) and then removed from where it
    // was — found in the same place from the next unlock on, and readable by
    // the extension.
    #[cfg(target_os = "ios")]
    fn read(
        options: fn() -> PasswordOptions,
        account: &str,
        add: impl FnOnce(&[u8]) -> Result<()>,
    ) -> Result<Zeroizing<Vec<u8>>> {
        read_through_group(
            || read_item(options()),
            || read_item(PasswordOptions::new_generic_password(SERVICE, account)),
            |key| {
                add(key)?;
                delete_one(legacy(account))
            },
        )
    }

    // The item as an install before the App Group wrote it: no group named,
    // so iOS put it in the first group the app is entitled to — its
    // application identifier, team prefix and bundle id (the `developmentTeam`
    // in `tauri.conf.json`). Named exactly, so the delete that follows a move
    // cannot take the item just written under the shared group with it, which
    // a query without any group would.
    #[cfg(target_os = "ios")]
    fn legacy(account: &str) -> PasswordOptions {
        let mut opts = PasswordOptions::new_generic_password(SERVICE, account);
        opts.set_access_group(LEGACY_ACCESS_GROUP);
        opts
    }

    #[cfg(target_os = "ios")]
    const LEGACY_ACCESS_GROUP: &str = "UFBL3F444A.app.rowel.mobile";

    fn read_item(opts: PasswordOptions) -> Result<Zeroizing<Vec<u8>>> {
        generic_password(opts).map(Zeroizing::new).map_err(map_err)
    }

    pub fn delete() -> Result<()> {
        delete_one(protected_options())?;
        delete_one(prompt_options())?;
        // An item never moved — enrolled before the App Group, and not read
        // since — goes too, or disabling biometric unlock would leave it.
        #[cfg(target_os = "ios")]
        {
            delete_one(legacy(ACCOUNT))?;
            delete_one(legacy(ACCOUNT_PROMPT))?;
        }
        Ok(())
    }

    fn delete_one(opts: PasswordOptions) -> Result<()> {
        match delete_generic_password_options(opts) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == ERR_ITEM_NOT_FOUND => Ok(()),
            Err(e) => Err(map_err(e)),
        }
    }

    fn map_err(e: security_framework::base::Error) -> Error {
        match e.code() {
            ERR_ITEM_NOT_FOUND => Error::NotFound,
            // The Touch ID sheet the protected read puts up was dismissed. The
            // user's choice, not a failure — the lock screen goes back to
            // waiting rather than reporting one.
            ERR_USER_CANCELED => Error::Cancelled,
            // Distinct from NotFound on purpose: the item may well still exist,
            // this build just can't reach it. Callers must not treat it as "the
            // enrollment is gone" and delete the marker (see `unlock_biometric`).
            ERR_MISSING_ENTITLEMENT => Error::Other(
                "this build is not entitled to read the protected keychain item; \
                 re-enable biometric unlock to re-enroll"
                    .into(),
            ),
            _ => Error::Other(e.to_string()),
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use super::*;
    use keyring::{Entry, Error as KrError};
    use windows::core::HSTRING;
    use windows::Security::Credentials::{
        KeyCredentialCreationOption, KeyCredentialManager, KeyCredentialRetrievalResult,
        KeyCredentialStatus,
    };
    use windows::Storage::Streams::{DataReader, DataWriter, IBuffer};

    pub const SUPPORTED: bool = true;

    // The Hello key credential the stored blob is bound to. Per app, not per
    // user; reusing SERVICE keeps the two halves of one enrollment named alike.
    const CREDENTIAL_NAME: &str = SERVICE;

    fn entry() -> Result<Entry> {
        Entry::new(SERVICE, ACCOUNT).map_err(|e| Error::Other(e.to_string()))
    }

    // `join` is windows-rs 0.62's name for what used to be `get`: block the
    // calling thread until the WinRT async operation completes (same idiom as
    // `biometrics.rs`). Every secure-store call already runs off the UI thread.
    pub fn key_credentials_available() -> bool {
        KeyCredentialManager::IsSupportedAsync()
            .and_then(|op| op.join())
            .unwrap_or(false)
    }

    // Credential Manager has no gate of its own, so the material is sealed
    // before it goes in: `RequestCreateAsync` is itself the Hello prompt, and
    // ReplaceExisting mints a fresh key pair so a re-enrollment never inherits
    // the previous one's wrapping key.
    pub fn store(key: &[u8]) -> Result<GateMode> {
        let created = KeyCredentialManager::RequestCreateAsync(
            &HSTRING::from(CREDENTIAL_NAME),
            KeyCredentialCreationOption::ReplaceExisting,
        )
        .and_then(|op| op.join())
        .map_err(win_err)?;
        let signature = sign_challenge(&created)?;
        // Before anything is stored: a signature that is not the deterministic
        // one the design assumes would seal the key under a value no later
        // prompt can reproduce, and the enrollment would only be found broken
        // at the first unlock.
        assert_pkcs1_signature(&created, &signature)?;
        let blob = wrap_master(&signature, key)?;
        entry()?
            .set_secret(&blob)
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(GateMode::HelloKey)
    }

    // Verify the enrollment signature as RSA PKCS#1 v1.5 / SHA-256 against the
    // credential's own public key. PKCS#1 v1.5 is deterministic, so passing this
    // is what guarantees the next prompt reproduces the same bytes and with them
    // the wrapping key; a probabilistic scheme (RSA-PSS) fails it, and the
    // enrollment is refused with a reason instead of stored unopenable. One
    // public-key operation, no second prompt.
    fn assert_pkcs1_signature(
        result: &KeyCredentialRetrievalResult,
        signature: &[u8],
    ) -> Result<()> {
        use windows::Security::Cryptography::Core::{
            AsymmetricAlgorithmNames, AsymmetricKeyAlgorithmProvider, CryptographicEngine,
            CryptographicPublicKeyBlobType,
        };

        let public_key = result
            .Credential()
            .map_err(win_err)?
            .RetrievePublicKeyWithBlobType(CryptographicPublicKeyBlobType::BCryptPublicKey)
            .map_err(win_err)?;
        let provider = AsymmetricKeyAlgorithmProvider::OpenAlgorithm(
            &AsymmetricAlgorithmNames::RsaSignPkcs1Sha256().map_err(win_err)?,
        )
        .map_err(win_err)?;
        let key = provider
            .ImportPublicKeyWithBlobType(
                &public_key,
                CryptographicPublicKeyBlobType::BCryptPublicKey,
            )
            .map_err(win_err)?;
        let verified = CryptographicEngine::VerifySignature(
            &key,
            &to_buffer(HELLO_CHALLENGE)?,
            &to_buffer(signature)?,
        )
        .map_err(win_err)?;
        if verified {
            Ok(())
        } else {
            Err(Error::Other(
                "Windows Hello on this device does not sign with RSA PKCS#1 v1.5, which \
                 biometric unlock relies on to reproduce its key; biometric unlock is \
                 unavailable here"
                    .into(),
            ))
        }
    }

    pub fn retrieve(mode: GateMode) -> Result<Zeroizing<Vec<u8>>> {
        if mode != GateMode::HelloKey {
            // A pre-Hello-key enrollment (`Prompt`, which is also what the
            // legacy "1" marker reads as here). That item is an *ungated* copy
            // of the master key — Credential Manager enforces nothing, so any
            // process running as this user could read it. Delete it rather than
            // read it: the caller treats NotFound as "the enrollment is gone"
            // and un-enrolls, so re-enabling biometrics produces a wrapped one.
            // A failed delete propagates as itself: reporting NotFound over a
            // key that is still there would clear the marker and with it the
            // only path that ever retries the removal.
            delete()?;
            return Err(Error::NotFound);
        }
        // Opening and signing *is* the verification — the wrapping key cannot
        // exist without a signature Hello refuses to produce unprompted. Hence
        // no `biometrics::authenticate()` here: a prompt that merely precedes a
        // plain read gates nothing.
        let opened = KeyCredentialManager::OpenAsync(&HSTRING::from(CREDENTIAL_NAME))
            .and_then(|op| op.join())
            .map_err(win_err)?;
        let signature = sign_challenge(&opened)?;
        let blob = match entry()?.get_secret() {
            Ok(blob) => blob,
            Err(KrError::NoEntry) => return Err(Error::NotFound),
            Err(e) => return Err(Error::Other(e.to_string())),
        };
        // A GCM failure here means this signature is not the one the blob was
        // sealed under — a reset Hello key, or a signature scheme that changed
        // under us. Neither is "the enrollment is gone", so not `NotFound`; the
        // user re-enrolls and the message says so.
        unwrap_master(&signature, &blob).map_err(|_| {
            Error::Other(
                "Windows Hello did not reproduce the biometric key; re-enable biometric \
                 unlock to re-enroll"
                    .into(),
            )
        })
    }

    pub fn delete() -> Result<()> {
        // Both halves go, and a missing one is not an error: a leftover key
        // credential would keep prompting for material that no longer exists,
        // and a leftover blob would outlive the key that opens it.
        let _ = KeyCredentialManager::DeleteAsync(&HSTRING::from(CREDENTIAL_NAME))
            .and_then(|op| op.join());
        match entry()?.delete_credential() {
            Ok(()) | Err(KrError::NoEntry) => Ok(()),
            Err(e) => Err(Error::Other(e.to_string())),
        }
    }

    // Both halves of a Hello operation report a `KeyCredentialStatus`. Mapped
    // the way `biometrics.rs` maps `UserConsentVerificationResult`: a dismissed
    // or declined prompt is the user's choice, not a failure, and a missing
    // credential means the enrollment is gone (the caller clears the marker).
    fn check_status(status: KeyCredentialStatus) -> Result<()> {
        if status == KeyCredentialStatus::Success {
            Ok(())
        } else if status == KeyCredentialStatus::NotFound {
            Err(Error::NotFound)
        } else if status == KeyCredentialStatus::UserCanceled
            || status == KeyCredentialStatus::UserPrefersPassword
        {
            Err(Error::Cancelled)
        } else {
            Err(Error::Other(format!(
                "Windows Hello refused the key credential ({status:?})"
            )))
        }
    }

    fn sign_challenge(result: &KeyCredentialRetrievalResult) -> Result<Zeroizing<Vec<u8>>> {
        check_status(result.Status().map_err(win_err)?)?;
        let credential = result.Credential().map_err(win_err)?;
        let signed = credential
            .RequestSignAsync(&to_buffer(HELLO_CHALLENGE)?)
            .and_then(|op| op.join())
            .map_err(win_err)?;
        check_status(signed.Status().map_err(win_err)?)?;
        let buffer = signed.Result().map_err(win_err)?;
        Ok(Zeroizing::new(from_buffer(&buffer)?))
    }

    // WinRT speaks `IBuffer`, not slices. `DataWriter`/`DataReader` are the
    // conversion that needs no extra `windows` feature beyond Storage_Streams.
    fn to_buffer(bytes: &[u8]) -> Result<IBuffer> {
        let writer = DataWriter::new().map_err(win_err)?;
        writer.WriteBytes(bytes).map_err(win_err)?;
        writer.DetachBuffer().map_err(win_err)
    }

    fn from_buffer(buffer: &IBuffer) -> Result<Vec<u8>> {
        let len = buffer.Length().map_err(win_err)? as usize;
        let reader = DataReader::FromBuffer(buffer).map_err(win_err)?;
        let mut out = vec![0u8; len];
        reader.ReadBytes(&mut out).map_err(win_err)?;
        Ok(out)
    }

    fn win_err(e: windows::core::Error) -> Error {
        Error::Other(e.to_string())
    }
}

#[cfg(not(any(target_vendor = "apple", target_os = "windows")))]
mod imp {
    use super::*;

    pub const SUPPORTED: bool = false;

    fn unsupported() -> Error {
        Error::Other("biometric secure store is not supported on this platform".into())
    }

    pub fn store(_key: &[u8]) -> Result<GateMode> {
        Err(unsupported())
    }
    pub fn retrieve(_mode: GateMode) -> Result<Zeroizing<Vec<u8>>> {
        Err(unsupported())
    }
    pub fn delete() -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    // In-memory stand-in for the OS secure store (no biometric prompt). Keeps a
    // slot per mode so a cross-mode read is observably a miss, exactly as it is
    // on disk where the two modes are different keychain items.
    #[derive(Default)]
    struct MockStore {
        protected: RefCell<Option<Vec<u8>>>,
        prompt: RefCell<Option<Vec<u8>>>,
        hello_key: RefCell<Option<Vec<u8>>>,
        // Simulates an ad-hoc-signed build: the protected keychain refuses.
        no_entitlement: Cell<bool>,
    }

    impl MockStore {
        fn unentitled() -> Self {
            let s = Self::default();
            s.no_entitlement.set(true);
            s
        }
        fn slot(&self, mode: GateMode) -> &RefCell<Option<Vec<u8>>> {
            match mode {
                GateMode::Protected => &self.protected,
                GateMode::Prompt => &self.prompt,
                GateMode::HelloKey => &self.hello_key,
            }
        }
    }

    impl KeyStore for MockStore {
        fn store(&self, key: &[u8]) -> Result<GateMode> {
            self.delete()?;
            let no_entitlement = self.no_entitlement.get();
            enroll(
                || {
                    if no_entitlement {
                        ProtectedOutcome::NoEntitlement
                    } else {
                        *self.protected.borrow_mut() = Some(key.to_vec());
                        ProtectedOutcome::Stored
                    }
                },
                || {
                    *self.prompt.borrow_mut() = Some(key.to_vec());
                    Ok(())
                },
            )
        }
        fn retrieve(&self, mode: GateMode) -> Result<Zeroizing<Vec<u8>>> {
            self.slot(mode)
                .borrow()
                .clone()
                .map(Zeroizing::new)
                .ok_or(Error::NotFound)
        }
        fn delete(&self) -> Result<()> {
            *self.protected.borrow_mut() = None;
            *self.prompt.borrow_mut() = None;
            *self.hello_key.borrow_mut() = None;
            Ok(())
        }
    }

    #[test]
    fn store_then_retrieve_roundtrips_the_key() {
        let secret = crate::crypto::hash_secret("hunter2");
        let store = MockStore::default();
        let mode = store.store(secret.as_bytes()).unwrap();
        assert_eq!(mode, GateMode::Protected);
        assert_eq!(&*store.retrieve(mode).unwrap(), secret.as_bytes());
    }

    #[test]
    fn restore_overwrites_a_stale_key() {
        // change_master_password re-stores the new material over the old.
        let store = MockStore::default();
        store
            .store(crate::crypto::hash_secret("old-pass").as_bytes())
            .unwrap();
        let new = crate::crypto::hash_secret("new-pass");
        let mode = store.store(new.as_bytes()).unwrap();
        assert_eq!(&*store.retrieve(mode).unwrap(), new.as_bytes());
    }

    #[test]
    fn delete_makes_retrieve_not_found() {
        let store = MockStore::default();
        let mode = store.store(b"k").unwrap();
        store.delete().unwrap();
        assert!(matches!(store.retrieve(mode), Err(Error::NotFound)));
    }

    #[test]
    fn an_unentitled_build_enrolls_in_prompt_mode() {
        let store = MockStore::unentitled();
        assert_eq!(store.store(b"k").unwrap(), GateMode::Prompt);
        assert_eq!(&*store.retrieve(GateMode::Prompt).unwrap(), b"k");
    }

    #[test]
    fn retrieval_never_falls_through_to_the_other_mode() {
        // The recorded mode is the only one consulted. Reading a prompt-mode
        // enrollment as protected (or vice versa) must miss, not silently
        // succeed through the weaker/other gate.
        let store = MockStore::unentitled();
        assert_eq!(store.store(b"k").unwrap(), GateMode::Prompt);
        assert!(matches!(
            store.retrieve(GateMode::Protected),
            Err(Error::NotFound)
        ));

        let store = MockStore::default();
        assert_eq!(store.store(b"k").unwrap(), GateMode::Protected);
        assert!(matches!(
            store.retrieve(GateMode::Prompt),
            Err(Error::NotFound)
        ));
    }

    #[test]
    fn re_enrollment_leaves_no_orphan_in_the_other_mode() {
        // Enroll protected, then re-enroll as an unentitled build would: the
        // protected item must be gone, not left behind holding stale material.
        let store = MockStore::default();
        store.store(b"old").unwrap();
        store.no_entitlement.set(true);
        assert_eq!(store.store(b"new").unwrap(), GateMode::Prompt);
        assert!(store.protected.borrow().is_none());
        assert_eq!(&*store.retrieve(GateMode::Prompt).unwrap(), b"new");
    }

    #[test]
    fn enroll_prefers_protected_and_never_calls_the_fallback() {
        let fallback_ran = Cell::new(false);
        let mode = enroll(
            || ProtectedOutcome::Stored,
            || {
                fallback_ran.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(mode, GateMode::Protected);
        assert!(!fallback_ran.get(), "protected succeeded; no fallback");
    }

    #[test]
    fn enroll_propagates_a_non_entitlement_failure() {
        // A broken keychain must not be quietly downgraded to the weaker gate.
        let fallback_ran = Cell::new(false);
        let result = enroll(
            || ProtectedOutcome::Failed(Error::Other("keychain is on fire".into())),
            || {
                fallback_ran.set(true);
                Ok(())
            },
        );
        assert!(matches!(result, Err(Error::Other(m)) if m == "keychain is on fire"));
        assert!(!fallback_ran.get(), "only NoEntitlement falls back");
    }

    // --- the App Group keychain group (iOS) ------------------------------------

    fn key(bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        Ok(Zeroizing::new(bytes.to_vec()))
    }

    // Enrolled since the group: found there, and the old place never asked.
    #[test]
    fn a_key_in_the_group_is_read_there_and_nothing_moves() {
        let asked_legacy = Cell::new(false);
        let adopted = Cell::new(false);
        let got = read_through_group(
            || key(b"k"),
            || {
                asked_legacy.set(true);
                key(b"old")
            },
            |_| {
                adopted.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(&*got, b"k");
        assert!(!asked_legacy.get() && !adopted.get());
    }

    // Enrolled before the group: read from the old place and moved, with the
    // very bytes that were read.
    #[test]
    fn a_key_outside_the_group_is_read_and_moved_into_it() {
        let moved = RefCell::new(None);
        let got = read_through_group(
            || Err(Error::NotFound),
            || key(b"old"),
            |k| {
                *moved.borrow_mut() = Some(k.to_vec());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(&*got, b"old");
        assert_eq!(moved.borrow().as_deref(), Some(&b"old"[..]));
    }

    // The unlock does not depend on the move: a key that will not move is
    // still the key, and the next unlock tries the move again.
    #[test]
    fn a_move_that_fails_still_returns_the_key() {
        let got = read_through_group(
            || Err(Error::NotFound),
            || key(b"old"),
            |_| Err(Error::Other("keychain is on fire".into())),
        )
        .unwrap();
        assert_eq!(&*got, b"old");
    }

    // Enrolled nowhere: the miss is the answer, and nothing is written.
    #[test]
    fn no_key_anywhere_is_not_found() {
        let adopted = Cell::new(false);
        let result = read_through_group(
            || Err(Error::NotFound),
            || Err(Error::NotFound),
            |_| {
                adopted.set(true);
                Ok(())
            },
        );
        assert!(matches!(result, Err(Error::NotFound)));
        assert!(!adopted.get());
    }

    // A dismissed Face ID sheet on the group's item is the user's answer: the
    // old place is not asked, which would put a second sheet up.
    #[test]
    fn a_cancelled_read_in_the_group_is_not_retried_outside_it() {
        let asked_legacy = Cell::new(false);
        let result = read_through_group(
            || Err(Error::Cancelled),
            || {
                asked_legacy.set(true);
                key(b"old")
            },
            |_| Ok(()),
        );
        assert!(matches!(result, Err(Error::Cancelled)));
        assert!(!asked_legacy.get());
    }

    #[test]
    fn gate_mode_markers_round_trip() {
        for mode in [GateMode::Protected, GateMode::Prompt, GateMode::HelloKey] {
            assert_eq!(GateMode::from_marker(mode.as_marker()), mode);
        }
        // A pre-mode marker reads as whatever that build actually wrote.
        assert_eq!(GateMode::from_marker("1"), GateMode::LEGACY);
        assert_eq!(GateMode::from_marker(""), GateMode::LEGACY);
    }

    // --- Windows Hello wrapping (platform-independent halves) ----------------

    // A stand-in for what `KeyCredential::RequestSignAsync` hands back: opaque
    // bytes that only a successful Hello prompt can reproduce.
    const SIGNATURE: &[u8] = b"a-windows-hello-rsa-signature";

    #[test]
    fn hello_wrapping_round_trips_the_key_material() {
        let master = crate::crypto::hash_secret("hunter2").into_bytes();
        let blob = wrap_master(SIGNATURE, &master).unwrap();
        // Only this blob reaches Credential Manager; the key must not be in it.
        assert!(!blob.windows(master.len()).any(|w| w == master.as_slice()));
        assert_eq!(&*unwrap_master(SIGNATURE, &blob).unwrap(), &master);
    }

    #[test]
    fn a_tampered_hello_blob_does_not_unwrap() {
        let mut blob = wrap_master(SIGNATURE, b"master").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0x01;
        assert!(unwrap_master(SIGNATURE, &blob).is_err());
        // Too short to even hold a nonce and tag: also a clean failure.
        assert!(unwrap_master(SIGNATURE, b"nope").is_err());
    }

    #[test]
    fn a_different_hello_signature_does_not_unwrap() {
        // Re-enrolling Hello (new PIN, reset key credential) yields a different
        // signature, so the old blob is inert rather than readable.
        let blob = wrap_master(SIGNATURE, b"master").unwrap();
        assert!(unwrap_master(b"a-different-signature", &blob).is_err());
    }

    #[test]
    fn the_hello_wrapping_key_is_not_a_vault_subkey() {
        // The wrapping key shares the master's HKDF salt, so only the distinct
        // `info` label keeps it clear of the SQLCipher and payload subkeys.
        let master = [7u8; 32];
        assert_ne!(
            *hello_wrapping_key(&master),
            crate::crypto::hkdf_subkey(&master, b"sqlcipher-db-key")
        );
        assert_ne!(
            *hello_wrapping_key(&master),
            crate::crypto::hkdf_subkey(&master, b"payload-aead-key")
        );
    }

    // --- Real-keychain probes -------------------------------------------------
    //
    // #[ignore]d: they touch this machine's actual login/data-protection
    // keychains, so CI never runs them. They use their own service/account so
    // they can never collide with a real enrollment, and clean up after
    // themselves. Run with: cargo test -- --ignored probe_
    #[cfg(target_os = "macos")]
    mod probes {
        use security_framework::passwords::{
            delete_generic_password_options, set_generic_password_options, AccessControlOptions,
            PasswordOptions,
        };

        const PROBE_SERVICE: &str = "app.rowel.desktop.probe";

        #[test]
        #[ignore = "touches the real keychain"]
        fn probe_protected_store_reports_its_status() {
            let mut opts = PasswordOptions::new_generic_password(PROBE_SERVICE, "probe-protected");
            opts.use_protected_keychain();
            opts.set_access_control_options(AccessControlOptions::BIOMETRY_CURRENT_SET);
            let result = set_generic_password_options(b"probe", opts);
            match &result {
                Ok(()) => println!("protected store: OK (this build is entitled)"),
                Err(e) => println!("protected store: code {} ({e})", e.code()),
            }
            if result.is_ok() {
                let mut opts =
                    PasswordOptions::new_generic_password(PROBE_SERVICE, "probe-protected");
                opts.use_protected_keychain();
                let _ = delete_generic_password_options(opts);
            }
        }

        #[test]
        #[ignore = "touches the real keychain"]
        fn probe_prompt_store_and_delete_roundtrip() {
            // Store + delete only: retrieval would need a real fingerprint.
            let opts = PasswordOptions::new_generic_password(PROBE_SERVICE, "probe-prompt");
            set_generic_password_options(b"probe", opts).expect("plain login-keychain store");
            let opts = PasswordOptions::new_generic_password(PROBE_SERVICE, "probe-prompt");
            delete_generic_password_options(opts).expect("plain login-keychain delete");
            println!("prompt store + delete: OK");
        }
    }
}
