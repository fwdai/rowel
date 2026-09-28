//! The WebAuthn authenticator: registers ("make credential") and signs in
//! ("get assertion") with passkeys held inside sealed login entries.
//!
//! The ceremony logic itself is 1Password's `passkey-rs`; this module supplies
//! the two things it leaves to the vendor — where credentials are stored
//! ([`store::PasskeyVault`]) and how the user is verified ([`UnlockedSession`]).
//! Deliberately free of Tauri types and of any notion of transport: the
//! browser-extension host (`browser::passkeys`) plays the WebAuthn client and
//! feeds requests in.
//!
//! ## User verification
//! An unlocked vault is the *identity* half of user verification: only the
//! holder of the master password (or the biometric) has one. The *intent*
//! half — that the user wants this registration or this sign-in to happen —
//! is asked for every ceremony through [`UserConsent`], which the transport
//! supplies when it builds the [`Authenticator`]. There is no default: nothing
//! can construct an authenticator without saying how the user is asked, so a
//! transport has to bring its confirm prompt with it rather than inherit a
//! silent yes. A refusal ends the ceremony as `OperationDenied`; presence and
//! verification are both reported only after an approval. A sign-in is asked
//! about with every account it could be as — the site may name none, and the
//! vault may hold several for it — and the user's answer picks one. The one
//! ceremony refused before the user is asked is a registration whose
//! excludeCredentials names a credential we already hold —
//! `passkey-authenticator` leaves that answer to the verification method.
//!
//! ## Signature counters
//! Passkeys here sync between devices, so a per-device counter would look like
//! a cloned authenticator to any relying party that tracks it. New credentials
//! are therefore created with the constant zero the WebAuthn spec recommends
//! for that case and are never incremented on sign-in. A credential imported
//! with a non-zero counter keeps counting, since its previous owner already
//! taught the relying party to expect that.

pub mod key;
pub mod store;

#[cfg(test)]
mod tests;

use std::sync::{Arc, Mutex};

use passkey_authenticator::{UiHint, UserCheck, UserValidationMethod};
use passkey_types::ctap2::{get_assertion, make_credential, Aaguid, Ctap2Error, StatusCode};
use passkey_types::webauthn::{PublicKeyCredentialDescriptor, PublicKeyCredentialType};

use store::{PasskeyVault, Stored, VaultCredentialStore};

/// Rowel's AAGUID — `8f2b41d7-6c93-4e1a-a50d-37e8b16429c5`, a UUIDv4 generated
/// once for this crate. One fixed identifier for "the Rowel desktop
/// authenticator", the same on every install: it names the model, never the
/// device, because a per-install value would be a handle for correlating a user
/// across relying parties.
pub const AAGUID: Aaguid = Aaguid([
    0x8f, 0x2b, 0x41, 0xd7, 0x6c, 0x93, 0x4e, 0x1a, 0xa5, 0x0d, 0x37, 0xe8, 0xb1, 0x64, 0x29, 0xc5,
]);

/// The CTAP2 authenticator type this module builds.
pub type Ctap2Authenticator<V> =
    passkey_authenticator::Authenticator<VaultCredentialStore<V>, UnlockedSession>;

/// An account a sign-in could be as: one passkey the vault holds for the site,
/// named the way the site named it at registration, and when it was made —
/// which is what tells two passkeys apart when a site named them the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub credential_id: String,
    pub user_name: String,
    pub user_display_name: String,
    pub created_at: Option<String>,
}

impl From<&Stored> for Account {
    fn from(stored: &Stored) -> Self {
        Self {
            credential_id: stored.passkey.credential_id.clone(),
            user_name: stored.passkey.user_name.clone(),
            user_display_name: stored.passkey.user_display_name.clone(),
            created_at: stored.passkey.created_at.clone(),
        }
    }
}

/// What the user is being asked to approve, in the terms a prompt would show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ceremony<'a> {
    /// Create a passkey for `rp_id`, for the account the site calls `user_name`
    /// (and shows as `user_display_name`).
    Register {
        rp_id: &'a str,
        user_name: Option<&'a str>,
        user_display_name: Option<&'a str>,
    },
    /// Sign in to `rp_id` as one of `accounts` — every passkey the vault holds
    /// for it that the site's allow list admits, newest first, never empty.
    SignIn {
        rp_id: &'a str,
        accounts: &'a [Account],
    },
}

/// The user's say over a ceremony. `approve` runs before a key is created or a
/// signature made; `None` ends the ceremony as denied. For a sign-in the answer
/// is which of the accounts: `Some(i)` signs in as `accounts[i]`. A
/// registration has nothing to choose, so any `Some` is a yes. Supplied by
/// whatever feeds requests in — the browser-extension host and its confirm
/// sheet — and blocking is fine: the authenticator runs off the UI thread.
pub trait UserConsent: Send + Sync {
    fn approve(&self, ceremony: Ceremony<'_>) -> Option<usize>;
}

/// Registers and asserts passkeys against a [`PasskeyVault`].
pub struct Authenticator<V: PasskeyVault> {
    inner: Ctap2Authenticator<V>,
    consent: Arc<dyn UserConsent>,
    /// The record the user picked for the sign-in under way — the store hands
    /// the library that one and nothing else; see [`Self::get_assertion`].
    chosen: Chosen,
}

/// The record the user picked, by identity — the entry it is on and its
/// credential id — rather than by value: the vault may move on under a sign-in
/// (a sync lands while the user decides) without the pick meaning another
/// record. Shared by the authenticator, its store and its verification
/// method, so the pick made in `get_assertion` is what the other two go by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    pub entry_id: String,
    pub credential_id: String,
}

impl Picked {
    pub fn is(&self, stored: &Stored) -> bool {
        self.entry_id == stored.entry_id && self.credential_id == stored.passkey.credential_id
    }
}

pub(crate) type Chosen = Arc<Mutex<Option<Picked>>>;

impl<V: PasskeyVault> Authenticator<V> {
    pub fn new(vault: V, consent: impl UserConsent + 'static) -> Self {
        let consent: Arc<dyn UserConsent> = Arc::new(consent);
        let chosen: Chosen = Arc::default();
        Self {
            inner: passkey_authenticator::Authenticator::new(
                AAGUID,
                VaultCredentialStore::new(vault, chosen.clone()),
                UnlockedSession {
                    consent: consent.clone(),
                    chosen: chosen.clone(),
                },
            ),
            consent,
            chosen,
        }
    }

    // Both ceremonies fail with the bare CTAP status: it carries no message,
    // but it is the one detail a transport acts on — a refusal, an excluded
    // credential and no credential at all each have their own answer in the
    // extension's protocol. The vault's own failures are logged where they
    // happen (`store::vault_err`).

    /// Registration: create a credential, store it on a login entry, and return
    /// the attestation the relying party asked for.
    pub async fn make_credential(
        &mut self,
        request: make_credential::Request,
    ) -> std::result::Result<make_credential::Response, StatusCode> {
        self.inner.make_credential(request).await
    }

    /// Sign-in: assert an existing credential for the request's rpId.
    ///
    /// The library would sign with the newest passkey for the site and ask
    /// about that one alone; a site that names no credential expects the
    /// authenticator to let the user pick the account. So the user is asked
    /// here, with every account the request admits at stake, and the request
    /// is narrowed to their pick before the library sees it — to the record
    /// itself, through the store, not just its id: an import can leave two
    /// records with one credential id, and a yes to one is not a yes to the
    /// other.
    pub async fn get_assertion(
        &mut self,
        mut request: get_assertion::Request,
    ) -> std::result::Result<get_assertion::Response, StatusCode> {
        let allow = request
            .allow_list
            .as_deref()
            .filter(|list| !list.is_empty());
        let found = self.inner.store().matching(allow, &request.rp_id)?;
        if found.is_empty() {
            return Err(Ctap2Error::NoCredentials.into());
        }
        let accounts: Vec<Account> = found.iter().map(Account::from).collect();
        let picked = self
            .consent
            .approve(Ceremony::SignIn {
                rp_id: &request.rp_id,
                accounts: &accounts,
            })
            .and_then(|i| found.get(i))
            .ok_or(Ctap2Error::OperationDenied)?;
        let id = key::decode_credential_id(&picked.passkey.credential_id)
            .map_err(|_| Ctap2Error::NoCredentials)?;
        request.allow_list = Some(vec![PublicKeyCredentialDescriptor {
            ty: PublicKeyCredentialType::PublicKey,
            id: id.into(),
            transports: None,
        }]);
        *self.chosen.lock().unwrap() = Some(Picked {
            entry_id: picked.entry_id.clone(),
            credential_id: picked.passkey.credential_id.clone(),
        });
        let signed = self.inner.get_assertion(request).await;
        *self.chosen.lock().unwrap() = None;
        signed
    }

    /// Hand the bare CTAP2 authenticator to a WebAuthn client (`passkey-client`),
    /// which is how the tests drive a whole registration. Only a registration:
    /// the sign-in's consent lives in [`Self::get_assertion`], so the bare
    /// authenticator refuses to sign. The extension host plays the client
    /// itself and sends raw CTAP2 requests (see `browser::passkeys`).
    #[cfg(test)]
    pub fn into_ctap2(self) -> Ctap2Authenticator<V> {
        self.inner
    }
}

/// User verification: the vault is unlocked (or this could not have been
/// built), and the user approves each ceremony. See the module docs.
pub struct UnlockedSession {
    consent: Arc<dyn UserConsent>,
    chosen: Chosen,
}

#[async_trait::async_trait]
impl UserValidationMethod for UnlockedSession {
    type PasskeyItem = passkey_types::Passkey;

    async fn check_user<'a>(
        &self,
        hint: UiHint<'a, Self::PasskeyItem>,
        _presence: bool,
        _verification: bool,
    ) -> std::result::Result<UserCheck, Ctap2Error> {
        let approved = match hint {
            // `make_credential` no longer refuses an excludeCredentials hit
            // itself (passkey-authenticator 0.5 hands the decision here, so a
            // UI can say "you already have one"), so refusing is ours to do —
            // otherwise a second registration would silently store a duplicate
            // credential.
            UiHint::InformExcludedCredentialFound(_) => return Err(Ctap2Error::CredentialExcluded),
            // Nothing to approve: the library fails the ceremony on its own
            // once this returns.
            UiHint::InformNoCredentialsFound => true,
            UiHint::RequestNewCredential(user, rp) => self
                .consent
                .approve(Ceremony::Register {
                    rp_id: &rp.id,
                    user_name: user.name.as_deref(),
                    user_display_name: user.display_name.as_deref(),
                })
                .is_some(),
            // Asked already, in `Authenticator::get_assertion`, with every
            // account at stake — and the store narrowed to the record picked,
            // so this is it. Anything else is a sign-in nobody was asked about.
            UiHint::RequestExistingCredential(passkey) => {
                self.chosen.lock().unwrap().as_ref().is_some_and(|chosen| {
                    store::same_credential(&chosen.credential_id, &passkey.credential_id)
                })
            }
        };
        if !approved {
            return Err(Ctap2Error::OperationDenied);
        }
        Ok(UserCheck {
            presence: true,
            verification: true,
        })
    }

    fn is_presence_enabled(&self) -> bool {
        true
    }

    fn is_verification_enabled(&self) -> Option<bool> {
        Some(true)
    }
}
