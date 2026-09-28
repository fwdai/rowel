//! Where passkeys live: inside the `passkeys` list of a sealed login entry.
//!
//! [`PasskeyVault`] is the narrow contract the authenticator needs (find by
//! rpId, insert, update). [`SessionVault`] implements it over an unlocked
//! session's store + payload cipher; [`VaultCredentialStore`] adapts any
//! `PasskeyVault` to `passkey-rs`'s async [`CredentialStore`].

use passkey_types::ctap2::get_assertion::Options;
use passkey_types::ctap2::make_credential::{
    PublicKeyCredentialRpEntity, PublicKeyCredentialUserEntity,
};
use passkey_types::ctap2::{Ctap2Error, StatusCode, U2FError};
use passkey_types::webauthn::PublicKeyCredentialDescriptor;

use crate::crypto::PayloadCipher;
use crate::error::{Error, Result};
use crate::models::{Entry, Passkey};
use crate::session::store_err;
use crate::store::{migrate, VaultStore};

use super::key;

/// A stored passkey together with the login entry that owns it — the entry id is
/// what an update has to write back to.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    pub entry_id: String,
    pub passkey: Passkey,
}

/// The vault as the authenticator sees it. Deliberately tiny: three operations,
/// no notion of entries, sealing or SQL.
pub trait PasskeyVault: Send + Sync {
    /// Every passkey registered for `rp_id`, newest first.
    fn find(&self, rp_id: &str) -> Result<Vec<Stored>>;

    /// Store a newly created passkey, attaching it to a login entry (see
    /// [`SessionVault::insert`] for how the entry is chosen).
    fn insert(&self, passkey: &Passkey) -> Result<()>;

    /// Replace the passkey with the same credential id on `entry_id`.
    fn update(&self, entry_id: &str, passkey: &Passkey) -> Result<()>;
}

// So callers can lend a vault to an authenticator instead of giving it away
// (the tests keep the vault to inspect it; a command keeps its session guard).
impl<V: PasskeyVault + ?Sized> PasskeyVault for &V {
    fn find(&self, rp_id: &str) -> Result<Vec<Stored>> {
        (**self).find(rp_id)
    }
    fn insert(&self, passkey: &Passkey) -> Result<()> {
        (**self).insert(passkey)
    }
    fn update(&self, entry_id: &str, passkey: &Passkey) -> Result<()> {
        (**self).update(entry_id, passkey)
    }
}

/// The real vault: passkeys read and written through an unlocked session's
/// store and payload cipher. Borrowed, never owned — the session owns both.
///
/// `dyn VaultStore + Sync` rather than plain `dyn VaultStore` because
/// `CredentialStore`'s futures must be `Send`, which makes every field of this
/// struct shared across threads. `SqliteStore` is a `Mutex<Connection>`, so it
/// already qualifies.
pub struct SessionVault<'a> {
    store: &'a (dyn VaultStore + Sync),
    cipher: &'a PayloadCipher,
}

impl<'a> SessionVault<'a> {
    pub fn new(store: &'a (dyn VaultStore + Sync), cipher: &'a PayloadCipher) -> Self {
        Self { store, cipher }
    }

    // Every live login, unsealed. There is no rp_id column, so a lookup costs
    // one unseal per login entry — fine for a personal vault, and the price of
    // keeping passkeys entirely inside the sealed payload. A `passkey_rp_id`
    // column (or an index table) is the fix if it ever shows up in a profile.
    fn logins(&self) -> Result<Vec<Entry>> {
        let metas = self.store.list().map_err(store_err)?;
        let mut entries = Vec::new();
        for meta in metas.iter().filter(|m| m.kind == "login") {
            if let Some(record) = self.store.get(&meta.id).map_err(store_err)? {
                entries.push(self.cipher.unseal(&record.id, &record.payload)?);
            }
        }
        Ok(entries)
    }

    fn save(&self, entry: &Entry) -> Result<()> {
        let payload = self.cipher.seal(entry)?;
        self.store
            .upsert(&migrate::build_record(entry, payload)?)
            .map_err(store_err)
    }
}

impl PasskeyVault for SessionVault<'_> {
    fn find(&self, rp_id: &str) -> Result<Vec<Stored>> {
        let mut found: Vec<Stored> = self
            .logins()?
            .into_iter()
            .flat_map(|entry| {
                let id = entry.id.clone();
                entry
                    .passkeys
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|p| p.rp_id == rp_id)
                    .map(move |passkey| Stored {
                        entry_id: id.clone(),
                        passkey,
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        sort_newest_first(&mut found);
        Ok(found)
    }

    /// Attach the passkey to the one existing login for this site if there is
    /// exactly one (matched on the `url_host` column, which is what the entry
    /// list and the browser both key off); otherwise start a login entry for it,
    /// so a passkey never silently lands on the wrong account.
    fn insert(&self, passkey: &Passkey) -> Result<()> {
        let metas = self.store.list().map_err(store_err)?;
        let mut hosts = metas
            .iter()
            .filter(|m| m.kind == "login" && m.url_host == passkey.rp_id);
        let existing = match (hosts.next(), hosts.next()) {
            (Some(one), None) => Some(one.id.clone()),
            _ => None,
        };

        let entry = match existing {
            Some(id) => {
                let mut entry = self.entry(&id)?;
                entry
                    .passkeys
                    .get_or_insert_with(Vec::new)
                    .push(passkey.clone());
                entry
            }
            None => new_login(passkey),
        };
        self.save(&entry)
    }

    fn update(&self, entry_id: &str, passkey: &Passkey) -> Result<()> {
        let mut entry = self.entry(entry_id)?;
        let slot = entry
            .passkeys
            .iter_mut()
            .flatten()
            .find(|p| p.credential_id == passkey.credential_id)
            .ok_or(Error::NotFound)?;
        *slot = passkey.clone();
        self.save(&entry)
    }
}

impl SessionVault<'_> {
    fn entry(&self, id: &str) -> Result<Entry> {
        let record = self
            .store
            .get(id)
            .map_err(store_err)?
            .ok_or(Error::NotFound)?;
        self.cipher.unseal(&record.id, &record.payload)
    }
}

// A login entry for a site the vault does not know yet: titled after the
// relying party, with the origin as its website so the entry list, favicons and
// a later `insert` all key off the same host.
fn new_login(passkey: &Passkey) -> Entry {
    let now = chrono::Utc::now().to_rfc3339();
    Entry {
        id: migrate::new_entry_id(),
        kind: "login".into(),
        title: passkey
            .rp_name
            .clone()
            .unwrap_or_else(|| passkey.rp_id.clone()),
        username: Some(passkey.user_name.clone()),
        website: Some(format!("https://{}", passkey.rp_id)),
        passkeys: Some(vec![passkey.clone()]),
        created_at: Some(now.clone()),
        updated_at: Some(now),
        ..Default::default()
    }
}

// Newest first: `get_assertion` takes the first credential it is offered.
fn sort_newest_first(found: &mut [Stored]) {
    found.sort_by(|a, b| b.passkey.created_at.cmp(&a.passkey.created_at));
}

/// Adapts a [`PasskeyVault`] to the async `CredentialStore` the authenticator
/// drives. Nothing but translation lives here — and, for a sign-in, the pick:
/// once `Authenticator::get_assertion` has one, this is the record the library
/// gets, whatever else the vault holds under that id.
pub struct VaultCredentialStore<V> {
    vault: V,
    chosen: super::Chosen,
}

impl<V: PasskeyVault> VaultCredentialStore<V> {
    pub fn new(vault: V, chosen: super::Chosen) -> Self {
        Self { vault, chosen }
    }

    /// The vault's passkeys for `rp_id` that could sign, newest first — those
    /// `ids` names, or every one when there is no list (ours are always
    /// discoverable). What the authenticator signs with, and what the user is
    /// offered to sign in as, come from here alike.
    pub fn matching(
        &self,
        ids: Option<&[PublicKeyCredentialDescriptor]>,
        rp_id: &str,
    ) -> std::result::Result<Vec<Stored>, StatusCode> {
        Ok(self
            .vault
            .find(rp_id)
            .map_err(vault_err)?
            .into_iter()
            .filter(|s| match ids {
                Some(ids) => ids
                    .iter()
                    .any(|d| same_credential(&s.passkey.credential_id, &d.id)),
                None => true,
            })
            // A credential we cannot read (a foreign key type, a mangled id) is
            // dropped rather than failing the ceremony, so one bad import does
            // not lock the user out of the good credentials beside it.
            .filter(|s| match key::to_passkey_types(&s.passkey) {
                Ok(_) => true,
                Err(e) => {
                    log::warn!("skipping unusable passkey on entry {}: {e}", s.entry_id);
                    false
                }
            })
            .collect())
    }
}

#[async_trait::async_trait]
impl<V: PasskeyVault> passkey_authenticator::CredentialStore for VaultCredentialStore<V> {
    type PasskeyItem = passkey_types::Passkey;

    async fn find_credentials(
        &self,
        ids: Option<&[PublicKeyCredentialDescriptor]>,
        rp_id: &str,
        // The user handle the ceremony knows about, only ever `Some` on the
        // exclude-list check of a registration. Ignored, as `MemoryStore` does:
        // an exclude list is matched on credential id, and narrowing by user
        // handle would let a site re-register over an account it already has.
        _user_handle: Option<&[u8]>,
    ) -> std::result::Result<Vec<Self::PasskeyItem>, StatusCode> {
        let chosen = self.chosen.lock().unwrap().clone();
        let found: Vec<_> = self
            .matching(ids, rp_id)?
            .iter()
            // The record the user picked, when there is one: not another that
            // happens to carry the same credential id.
            .filter(|s| chosen.as_ref().is_none_or(|chosen| chosen.is(s)))
            .filter_map(|s| key::to_passkey_types(&s.passkey).ok())
            .collect();

        if found.is_empty() {
            // What `MemoryStore` does, and what the CTAP2 flows expect: an
            // empty exclude-list match must not read as "already registered".
            return Err(StatusCode::from(Ctap2Error::NoCredentials));
        }
        Ok(found)
    }

    async fn save_credential(
        &mut self,
        cred: passkey_types::Passkey,
        user: PublicKeyCredentialUserEntity,
        rp: PublicKeyCredentialRpEntity,
        _options: Options,
    ) -> std::result::Result<(), StatusCode> {
        let passkey =
            key::from_passkey_types(&cred, &user, rp.name.as_deref()).map_err(vault_err)?;
        self.vault.insert(&passkey).map_err(vault_err)
    }

    async fn update_credential(
        &mut self,
        cred: &passkey_types::Passkey,
    ) -> std::result::Result<(), StatusCode> {
        // The record that signed — the one picked, when a pick is up, since
        // an import can leave two records under one id — and not merely the
        // first the vault lists under the id.
        let chosen = self.chosen.lock().unwrap().clone();
        let stored = self
            .vault
            .find(&cred.rp_id)
            .map_err(vault_err)?
            .into_iter()
            .filter(|s| chosen.as_ref().is_none_or(|chosen| chosen.is(s)))
            .find(|s| same_credential(&s.passkey.credential_id, &cred.credential_id))
            .ok_or_else(|| StatusCode::from(Ctap2Error::InvalidCredential))?;

        // Only the counter can change on an update, and only for an imported
        // credential that already had a non-zero one (see `to_passkey_types`).
        let updated = Passkey {
            counter: cred.counter.unwrap_or(0),
            ..stored.passkey
        };
        self.vault
            .update(&stored.entry_id, &updated)
            .map_err(vault_err)
    }

    async fn get_info(&self) -> passkey_authenticator::StoreInfo {
        passkey_authenticator::StoreInfo {
            // The vault *is* the credential list, so everything it holds is
            // discoverable whether or not the site asked for a resident key.
            discoverability: passkey_authenticator::DiscoverabilitySupport::ForcedDiscoverable,
        }
    }
}

// Credential ids are compared as bytes, never as strings: ours are stored
// exactly as their source wrote them, one exporter's padded base64url is
// another's unpadded, and Bitwarden's is a GUID (`key::decode_credential_id`).
pub(super) fn same_credential(stored: &str, id: &[u8]) -> bool {
    key::decode_credential_id(stored).is_ok_and(|bytes| bytes == id)
}

// A vault failure is ours, not the relying party's: report the CTAP catch-all
// and keep the real reason in the log rather than in a WebAuthn response.
fn vault_err(e: Error) -> StatusCode {
    log::error!("passkey vault error: {e}");
    StatusCode::Ctap1(U2FError::Other)
}

/// An in-memory [`PasskeyVault`] for tests: one bucket of passkeys per entry id.
#[cfg(test)]
#[derive(Default)]
pub struct MemoryVault {
    entries: std::sync::Mutex<Vec<Stored>>,
}

#[cfg(test)]
impl MemoryVault {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed a passkey as an import would, on its own entry.
    pub fn seed(&self, entry_id: &str, passkey: Passkey) {
        self.entries.lock().unwrap().push(Stored {
            entry_id: entry_id.to_owned(),
            passkey,
        });
    }

    pub fn all(&self) -> Vec<Stored> {
        self.entries.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl PasskeyVault for MemoryVault {
    fn find(&self, rp_id: &str) -> Result<Vec<Stored>> {
        let mut found: Vec<_> = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.passkey.rp_id == rp_id)
            .cloned()
            .collect();
        sort_newest_first(&mut found);
        Ok(found)
    }

    fn insert(&self, passkey: &Passkey) -> Result<()> {
        self.seed(&migrate::new_entry_id(), passkey.clone());
        Ok(())
    }

    fn update(&self, entry_id: &str, passkey: &Passkey) -> Result<()> {
        let mut entries = self.entries.lock().unwrap();
        let slot = entries
            .iter_mut()
            .find(|s| s.entry_id == entry_id && s.passkey.credential_id == passkey.credential_id)
            .ok_or(Error::NotFound)?;
        slot.passkey = passkey.clone();
        Ok(())
    }
}
