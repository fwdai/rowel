use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use zeroize::Zeroize;

// A vault entry. Kept as a single flat struct (rather than an enum) so it
// round-trips the untyped legacy object shape; `kind` discriminates
// login/note/card/identity.
//
// `Debug` is written by hand (below), not derived: the fields are the secrets
// the vault exists to keep, and a derived impl would print every one of them.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub month: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cvc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// ID-document fields (`identity` entries). `doc_type` is one of `passport`,
    /// `id_card`, `driver_license`, `residence_permit`, `other`; the three dates
    /// are ISO `YYYY-MM-DD`. The document number and full name reuse `number` and
    /// `name`. All `None` on every other kind, so a pre-identity vault JSON,
    /// `.swftx` backup and fixture serializes byte-identically to before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nationality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub birth_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiry_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personal_number: Option<String>,
    /// SSH key fields (`ssh` entries). ed25519 only for now: `private_key` is an
    /// OpenSSH PEM block, `public_key` the single `ssh-ed25519 AAAA… comment`
    /// line and `fingerprint` its `SHA256:…` digest, derived once at save time so
    /// the detail view needs no key parsing. `passphrase` is what the user
    /// protected the key with elsewhere — we never encrypt the key ourselves.
    /// camelCase on the wire, matching the draft keys the editor writes.
    #[serde(
        rename = "privateKey",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub private_key: Option<String>,
    #[serde(rename = "publicKey", default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
    /// `.env` file fields (`env` entries). `body` is the file text, verbatim —
    /// the entry's one secret, stored whole rather than as parsed rows so that
    /// comments, blank lines, `export` prefixes, quoting style and `${VAR}`
    /// references round-trip byte-exact; the variables table is a view over it,
    /// never the source of truth. `file_name` is what the dropped file was
    /// called (`.env.production`), not a secret. camelCase on the wire like
    /// `privateKey`. Both `None` on every other kind, so existing vault JSON,
    /// `.swftx` backups and fixtures serialize byte-identically to before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(rename = "fileName", default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// API key fields (`apikey` entries). `api_key` is the token itself — the
    /// entry's one secret. `environment` is `test` or `production`, `base_url`
    /// the API's root, and `scopes` whatever the issuer granted, kept as the
    /// text it was typed as (space- or comma-separated); none of the three is
    /// secret. An expiry reuses `expiry_date`, the way an identity reuses
    /// `number`. camelCase on the wire like `privateKey`. All `None` on every
    /// other kind, so existing vault JSON, `.swftx` backups and fixtures
    /// serialize byte-identically to before.
    #[serde(rename = "apiKey", default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(rename = "baseUrl", default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scopes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// WebAuthn passkeys stored on a login entry. `None` when the entry has
    /// none, so every pre-passkey vault JSON, `.swftx` backup and fixture
    /// serializes byte-identically to before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passkeys: Option<Vec<Passkey>>,
    /// Free-form label/value pairs, in the order the user put them in — what a
    /// document carries that the fixed rows have no room for ("Categories: B,
    /// BE"). Kind-agnostic: any entry may hold them. `None` when there are none,
    /// so every pre-extras vault JSON, `.swftx` backup and fixture serializes
    /// byte-identically to before. Not obscured per field: the payload is sealed
    /// as a whole, and per-field secrecy for extras is deferred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Vec<ExtraField>>,
    /// The user's star. Stored as a column rather than in the payload, so it
    /// rides along here only to survive a `.swftx` export/import round-trip —
    /// the editor never sends it, which is what keeps an ordinary save from
    /// clearing one (see `store::migrate::build_record`). Omitted when unset, so
    /// a backup of an unstarred vault stays byte-identical to the legacy format.
    #[serde(default, skip_serializing_if = "is_unset")]
    pub favorite: bool,
    #[serde(rename = "createdAt", default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(rename = "updatedAt", default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(
        rename = "password_updated_at",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub password_updated_at: Option<String>,
}

fn is_unset(flag: &bool) -> bool {
    !*flag
}

// Only what the list already shows. Everything else on an entry is, or may
// be, a secret, and `Debug` output ends up in panic messages, `assert_eq!`
// failures and `{:?}` log lines — none of which should ever carry a password.
// `finish_non_exhaustive` marks the withheld rest as `..`.
impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}

impl Entry {
    /// The entry as the webview may see it: the same row with every passkey's
    /// private key blanked.
    ///
    /// A passkey's private key is the one secret on an entry that nothing in
    /// the UI renders, edits or needs — it is used only by the authenticator,
    /// which lives in the core. So it never crosses the IPC boundary: a reveal
    /// hands out the credential's identity (who it is for, when it was made)
    /// and nothing that could sign with it. [`Entry::restore_passkey_keys`] is
    /// the other half — how a save puts back what was never sent.
    ///
    /// It consumes the entry and scrubs the keys where they lie: a copy that
    /// was cloned and then cleared would leave the key bytes on the heap for
    /// whatever reads freed memory next.
    pub fn redacted(mut self) -> Entry {
        self.zeroize_passkey_keys();
        self
    }

    /// Scrub every passkey private key on this entry, leaving each blank.
    /// [`Zeroize`] for `String` overwrites the bytes before emptying it, which
    /// `String::clear` on its own does not.
    fn zeroize_passkey_keys(&mut self) {
        for passkey in self.passkeys.iter_mut().flatten() {
            passkey.private_key.zeroize();
        }
    }

    /// Whether this entry carries any passkey at all, and so needs the stored
    /// row to complete it. Every passkey that comes in from the webview is
    /// blank — a save that carries a private key is refused before the merge
    /// (see `commands::vault::save_entry`), so there is nothing else a passkey
    /// here could be.
    pub fn has_passkeys(&self) -> bool {
        self.passkeys.iter().flatten().next().is_some()
    }

    /// Whether any passkey on this entry arrived with a private key of its own.
    /// Only the core ever holds one, so on the way in this is always a caller
    /// trying to put key material into the vault, and the save is refused.
    pub fn has_supplied_passkey_key(&self) -> bool {
        self.passkeys
            .iter()
            .flatten()
            .any(|p| !p.private_key.is_empty())
    }

    /// Put back the passkey private keys the webview was never given, taking
    /// each from the entry being replaced, matched on `credential_id`.
    ///
    /// Only the passkeys still on `self` are completed, in the order `self`
    /// lists them — so an edit that drops one drops it, and one that reorders
    /// them reorders them. A passkey whose credential id is not in the stored
    /// row cannot be completed by anyone: rather than save a passkey that can
    /// never sign, the save is refused ([`Error::NotFound`], since what is
    /// missing is the stored credential this one claims to be).
    ///
    /// Every incoming passkey is checked against the stored row before any key
    /// moves, so a refusal leaves `self` exactly as it arrived: a merge that
    /// moved as it went would strand the keys it had already moved in an entry
    /// the caller is about to drop, unscrubbed.
    ///
    /// The stored entry is consumed: each key is moved across rather than
    /// copied, and whatever is left of it — the keys of passkeys the edit
    /// dropped, or all of them if the merge was refused — is scrubbed here, on
    /// every path out, rather than left on the heap for its drop.
    pub fn restore_passkey_keys(&mut self, mut stored: Option<Entry>) -> Result<()> {
        let merged = self.take_passkey_keys(stored.as_mut());
        if let Some(stored) = stored.as_mut() {
            stored.zeroize_passkey_keys();
        }
        merged
    }

    fn take_passkey_keys(&mut self, stored: Option<&mut Entry>) -> Result<()> {
        let Some(mine) = self.passkeys.as_deref_mut() else {
            return Ok(());
        };
        let held: &mut [Passkey] = stored
            .and_then(|e| e.passkeys.as_deref_mut())
            .unwrap_or_default();

        // First pass: the stored passkey each incoming one is to be completed
        // from. A stored key is claimed by at most one incoming passkey, so two
        // passkeys naming the same credential cannot both walk away with it —
        // the second is as unaccounted for as an unknown id, and refused.
        let mut claimed: Vec<usize> = Vec::with_capacity(mine.len());
        for passkey in mine.iter() {
            let found = held
                .iter()
                .enumerate()
                .find(|(i, p)| {
                    p.credential_id == passkey.credential_id
                        && !p.private_key.is_empty()
                        && !claimed.contains(i)
                })
                .ok_or(Error::NotFound)?;
            claimed.push(found.0);
        }

        // Second pass: nothing can fail from here, so every key moves or none
        // does. Anything the incoming passkey held is scrubbed as it is
        // replaced rather than dropped as it lies.
        for (passkey, index) in mine.iter_mut().zip(claimed) {
            passkey.private_key.zeroize();
            passkey.private_key = std::mem::take(&mut held[index].private_key);
        }
        Ok(())
    }
}

/// A single WebAuthn credential held by a login entry. Only P-256 ECDSA is
/// supported, so there is no algorithm field. `credential_id`, `user_handle` and
/// `private_key` are base64url and are stored exactly as the source gave them —
/// never re-encoded, so a round-trip through import/export is byte-exact.
/// `private_key` is a secret and only ever lives inside the sealed payload —
/// which is why `Debug` is hand-written below and leaves it out.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Passkey {
    pub credential_id: String,
    pub rp_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rp_name: Option<String>,
    pub user_handle: String,
    pub user_name: String,
    pub user_display_name: String,
    /// PKCS#8 DER, base64url. Blank on its way out to the webview (see
    /// [`Entry::redacted`]) and blank on its way back in (see
    /// [`Entry::restore_passkey_keys`]), so it is omitted rather than sent as
    /// an empty string — and a payload that carries one is always a real key.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub private_key: String,
    #[serde(default)]
    pub counter: u32,
    /// RFC3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

// The credential's identity — what names it to a relying party — and nothing
// that could sign for it.
impl fmt::Debug for Passkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Passkey")
            .field("credential_id", &self.credential_id)
            .field("rp_id", &self.rp_id)
            .field("user_name", &self.user_name)
            .field("counter", &self.counter)
            .finish_non_exhaustive()
    }
}

/// One free-form field on an entry: a label the user wrote and its value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtraField {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultData {
    pub entries: Vec<Entry>,
}

// Non-secret entry metadata sent to the frontend for the list. Never carries a
// secret field; secrets stay in the encrypted payload, revealed one at a time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryMetaDto {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub tags: Vec<String>,
    pub url_host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_brand: Option<String>,
    pub favorite: bool,
    // Whether the entry holds a passkey. Derived metadata, so the list can mark
    // the row without revealing anything; the passkeys themselves stay sealed.
    pub has_passkey: bool,
    // An env file's name and variable count, derived at save time like
    // `card_brand`; absent on every other kind and on rows not yet stamped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub var_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    // Set only on the tombstones the Trash lists; absent for live entries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
}

// The one projection of a stored metadata row onto the frontend DTO (timestamps
// are ms → RFC3339). Every command reads its row as an `EntryMeta` (`list`,
// `list_deleted`, `row_meta`), so save and list cannot drift apart on which
// columns reach the UI.
impl From<&crate::store::EntryMeta> for EntryMetaDto {
    fn from(m: &crate::store::EntryMeta) -> Self {
        Self {
            id: m.id.clone(),
            kind: m.kind.clone(),
            title: m.title.clone(),
            tags: serde_json::from_str(&m.tags).unwrap_or_default(),
            url_host: m.url_host.clone(),
            // "none" marks a completed derivation with no match — internal only.
            card_brand: m.card_brand.clone().filter(|b| b != "none"),
            favorite: m.favorite,
            has_passkey: m.has_passkey,
            file_name: m.file_name.clone(),
            var_count: m.var_count,
            username: m.username.clone().filter(|u| !u.is_empty()),
            created_at: iso(m.created_at),
            updated_at: iso(m.updated_at),
            deleted_at: m.deleted_at.and_then(iso),
        }
    }
}

fn iso(ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.to_rfc3339())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlockResult {
    pub entries: Vec<EntryMetaDto>,
    pub sync_configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratorOptions {
    pub length: u32,
    #[serde(default)]
    pub numbers: bool,
    #[serde(default)]
    pub symbols: bool,
    #[serde(default)]
    pub uppercase: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lowercase: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_similar_characters: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

/// A freshly generated ed25519 keypair, in the forms an entry stores them in:
/// the OpenSSH PEM private block, the single-line public key, and the
/// `SHA256:…` fingerprint. Keyed like the `ssh` draft, so the generator's
/// output can be handed straight to a new entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshKeyPair {
    pub private_key: String,
    pub public_key: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtpResult {
    pub code: String,
    /// Seconds left in the current window.
    pub time: u32,
    /// How long that window is — 30s unless the seed said otherwise.
    pub period: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditItem {
    pub score: u8,
    pub is_weak: bool,
    pub is_repeating: bool,
    pub breached: bool,
}

// Audit results keyed by entry id.
pub type Audit = HashMap<String, AuditItem>;

#[cfg(test)]
mod tests {
    use super::*;

    fn passkey(credential_id: &str, private_key: &str) -> Passkey {
        Passkey {
            credential_id: credential_id.into(),
            rp_id: "acme.test".into(),
            rp_name: None,
            user_handle: "dWgx".into(),
            user_name: "alice".into(),
            user_display_name: "Alice".into(),
            private_key: private_key.into(),
            counter: 0,
            created_at: None,
        }
    }

    fn login(passkeys: Vec<Passkey>) -> Entry {
        Entry {
            id: "l1".into(),
            kind: "login".into(),
            title: "Site".into(),
            passkeys: Some(passkeys),
            ..Entry::default()
        }
    }

    // What a reveal hands the webview: the credential, without the one part of
    // it that can sign. Blank rather than absent in memory, so the save that
    // comes back is recognizable as one that needs its keys put back.
    #[test]
    fn redacting_blanks_every_passkey_private_key() {
        let entry = login(vec![passkey("c1", "k1"), passkey("c2", "k2")]);
        let out = entry.redacted();

        assert!(out.has_passkeys());
        assert!(!out.has_supplied_passkey_key());
        assert!(out
            .passkeys
            .as_ref()
            .unwrap()
            .iter()
            .all(|p| p.private_key.is_empty()));
        // The credential is otherwise intact.
        assert_eq!(out.passkeys.as_ref().unwrap()[1].credential_id, "c2");

        // Blank means omitted on the wire: the field never reaches the webview.
        let json = serde_json::to_value(&out).unwrap();
        assert!(json["passkeys"][0].get("privateKey").is_none());
    }

    // An entry with no passkeys needs nothing from the stored row; one with a
    // passkey always does, since the key it needs never leaves the core.
    #[test]
    fn only_an_entry_with_passkeys_needs_a_merge() {
        assert!(!login(vec![]).has_passkeys());
        assert!(!Entry::default().has_passkeys());
        assert!(login(vec![passkey("c1", "")]).has_passkeys());
    }

    // A key on the way in is never one the webview was given, so it is one it
    // made up: the save is refused rather than merged.
    #[test]
    fn a_supplied_private_key_is_recognized() {
        assert!(login(vec![passkey("c1", "k1")]).has_supplied_passkey_key());
        assert!(!login(vec![passkey("c1", "")]).has_supplied_passkey_key());
        assert!(!login(vec![]).has_supplied_passkey_key());
    }

    // The save's half of the redaction: keys come back off the stored row,
    // matched by credential id — and the edit's own order is what is kept.
    #[test]
    fn saving_merges_blank_keys_back_from_the_stored_entry() {
        let stored = login(vec![passkey("c1", "k1"), passkey("c2", "k2")]);
        let mut incoming = stored.clone().redacted();
        incoming.passkeys.as_mut().unwrap().reverse();
        incoming.title = "Renamed".into();

        incoming.restore_passkey_keys(Some(stored)).unwrap();

        let passkeys = incoming.passkeys.unwrap();
        assert_eq!(passkeys[0].credential_id, "c2");
        assert_eq!(passkeys[0].private_key, "k2");
        assert_eq!(passkeys[1].private_key, "k1");
    }

    // Dropping a passkey in the editor drops it: only what came back is filled.
    #[test]
    fn a_removed_passkey_is_not_put_back() {
        let stored = login(vec![passkey("c1", "k1"), passkey("c2", "k2")]);
        let mut incoming = login(vec![passkey("c2", "")]);

        incoming.restore_passkey_keys(Some(stored)).unwrap();

        let passkeys = incoming.passkeys.unwrap();
        assert_eq!(passkeys.len(), 1);
        assert_eq!(passkeys[0].credential_id, "c2");
        assert_eq!(passkeys[0].private_key, "k2");
    }

    // A keyless passkey nobody can complete — a credential id the stored row
    // does not hold, or no stored row at all — is refused rather than saved as
    // a credential that could never sign.
    #[test]
    fn a_blank_key_with_no_stored_match_is_refused() {
        let stored = login(vec![passkey("c1", "k1")]);
        let mut unknown = login(vec![passkey("c9", "")]);
        assert!(matches!(
            unknown.restore_passkey_keys(Some(stored)),
            Err(Error::NotFound)
        ));

        let mut fresh = login(vec![passkey("c1", "")]);
        assert!(matches!(
            fresh.restore_passkey_keys(None),
            Err(Error::NotFound)
        ));
    }

    // The refusal comes before anything moves: an unknown credential listed
    // after a known one leaves the known one's key where it was, so the entry
    // that is about to be dropped carries no key the scrub would miss.
    #[test]
    fn a_refused_merge_moves_no_key_at_all() {
        let mut stored = login(vec![passkey("c1", "k1"), passkey("c2", "k2")]);
        let mut incoming = login(vec![passkey("c1", ""), passkey("c9", "")]);

        assert!(matches!(
            incoming.take_passkey_keys(Some(&mut stored)),
            Err(Error::NotFound)
        ));
        assert!(incoming
            .passkeys
            .as_ref()
            .unwrap()
            .iter()
            .all(|p| p.private_key.is_empty()));
        // c1's key never left the stored row.
        assert_eq!(stored.passkeys.as_ref().unwrap()[0].private_key, "k1");

        // And the same through the entry point that scrubs the stored row.
        assert!(matches!(
            incoming.restore_passkey_keys(Some(stored)),
            Err(Error::NotFound)
        ));
        assert!(incoming
            .passkeys
            .unwrap()
            .iter()
            .all(|p| p.private_key.is_empty()));
    }

    // One stored key completes one passkey: a second claim on the same
    // credential is as unaccounted for as an id the row never held.
    #[test]
    fn a_duplicated_credential_id_is_refused() {
        let stored = login(vec![passkey("c1", "k1")]);
        let mut incoming = login(vec![passkey("c1", ""), passkey("c1", "")]);

        assert!(matches!(
            incoming.restore_passkey_keys(Some(stored)),
            Err(Error::NotFound)
        ));
    }

    // Pre-passkey JSON stays readable, and an entry without passkeys serializes
    // exactly as it did before the field existed.
    #[test]
    fn entry_without_passkeys_round_trips_without_the_key() {
        let json = r#"{"id":"1","type":"login","title":"Site","password":"pw"}"#;
        let entry: Entry = serde_json::from_str(json).unwrap();
        assert!(entry.passkeys.is_none());
        let out = serde_json::to_string(&entry).unwrap();
        assert!(!out.contains("passkeys"), "{out}");
    }

    #[test]
    fn passkey_serializes_camel_case_and_skips_absent_optionals() {
        let entry: Entry = serde_json::from_str(
            r#"{"id":"1","type":"login","title":"Site","passkeys":[
                 {"credentialId":"Y3JlZDE","rpId":"acme.test","userHandle":"dWgx",
                  "userName":"alice","userDisplayName":"Alice","privateKey":"cGsx"}]}"#,
        )
        .unwrap();
        let passkeys = entry.passkeys.clone().unwrap();
        assert_eq!(passkeys[0].credential_id, "Y3JlZDE");
        assert_eq!(passkeys[0].private_key, "cGsx");
        assert_eq!(passkeys[0].counter, 0);
        assert_eq!(passkeys[0].rp_name, None);

        let out = serde_json::to_value(&entry).unwrap();
        let p = &out["passkeys"][0];
        assert_eq!(p["userDisplayName"], "Alice");
        assert_eq!(p["counter"], 0);
        assert!(p.get("rpName").is_none());
        assert!(p.get("createdAt").is_none());
    }

    // The identity fields are snake_case on the wire (the scanner and the
    // frontend draft both key off these exact names), and absent when unset.
    #[test]
    fn identity_fields_round_trip_snake_case_and_stay_absent_when_unset() {
        let entry: Entry = serde_json::from_str(
            r#"{"id":"1","type":"identity","title":"Passport","name":"ADA LOVELACE",
                 "number":"X1234567","doc_type":"passport","country":"GBR",
                 "birth_date":"1815-12-10","personal_number":"99-1815"}"#,
        )
        .unwrap();
        assert_eq!(entry.doc_type.as_deref(), Some("passport"));
        assert_eq!(entry.birth_date.as_deref(), Some("1815-12-10"));
        assert_eq!(entry.personal_number.as_deref(), Some("99-1815"));

        let out = serde_json::to_value(&entry).unwrap();
        assert_eq!(out["doc_type"], "passport");
        assert_eq!(out["birth_date"], "1815-12-10");
        assert!(out.get("issue_date").is_none());
        assert!(out.get("nationality").is_none());
    }

    // A login has no identity fields, so its JSON is exactly what it always was.
    #[test]
    fn non_identity_entry_serializes_without_the_identity_keys() {
        let entry: Entry =
            serde_json::from_str(r#"{"id":"1","type":"login","title":"Site","password":"pw"}"#)
                .unwrap();
        let out = serde_json::to_string(&entry).unwrap();
        assert_eq!(
            out,
            r#"{"id":"1","type":"login","title":"Site","password":"pw"}"#
        );
    }

    // The file rides through untouched — every byte of it, including the
    // comment and the blank line — and its name is camelCase on the wire, the
    // key the editor's draft writes. A legacy login carries neither key.
    #[test]
    fn env_fields_round_trip_camel_case_and_stay_absent_on_other_kinds() {
        let body = "# api\nexport API_KEY='abc' # inline\n\nURL=${HOST}/v1\n";
        let entry: Entry = serde_json::from_value(serde_json::json!({
            "id": "1", "type": "env", "title": "api · production",
            "fileName": ".env.production", "body": body
        }))
        .unwrap();
        assert_eq!(entry.body.as_deref(), Some(body));
        assert_eq!(entry.file_name.as_deref(), Some(".env.production"));

        let out = serde_json::to_value(&entry).unwrap();
        assert_eq!(out["fileName"], ".env.production");
        assert_eq!(out["body"], body);
        assert!(out.get("file_name").is_none());

        let legacy: Entry =
            serde_json::from_str(r#"{"id":"1","type":"login","title":"Site","password":"pw"}"#)
                .unwrap();
        assert!(legacy.body.is_none());
        assert!(legacy.file_name.is_none());
        let out = serde_json::to_string(&legacy).unwrap();
        assert!(!out.contains("body") && !out.contains("fileName"), "{out}");
    }

    // The token and its base URL are camelCase on the wire — the keys the
    // editor's draft writes — and an expiry rides in the identity's date slot.
    // A legacy login carries none of the keys.
    #[test]
    fn apikey_fields_round_trip_camel_case_and_stay_absent_on_other_kinds() {
        let entry: Entry = serde_json::from_value(serde_json::json!({
            "id": "1", "type": "apikey", "title": "Coupler.io",
            "apiKey": "cpl_live_abc", "environment": "production",
            "baseUrl": "https://api.coupler.io/v1", "scopes": "read write",
            "expiry_date": "2027-01-01"
        }))
        .unwrap();
        assert_eq!(entry.api_key.as_deref(), Some("cpl_live_abc"));
        assert_eq!(entry.base_url.as_deref(), Some("https://api.coupler.io/v1"));
        assert_eq!(entry.expiry_date.as_deref(), Some("2027-01-01"));

        let out = serde_json::to_value(&entry).unwrap();
        assert_eq!(out["apiKey"], "cpl_live_abc");
        assert_eq!(out["baseUrl"], "https://api.coupler.io/v1");
        assert_eq!(out["environment"], "production");
        assert_eq!(out["scopes"], "read write");
        assert!(out.get("api_key").is_none());
        assert!(out.get("base_url").is_none());

        let legacy: Entry =
            serde_json::from_str(r#"{"id":"1","type":"login","title":"Site","password":"pw"}"#)
                .unwrap();
        assert!(legacy.api_key.is_none());
        let out = serde_json::to_string(&legacy).unwrap();
        assert!(!out.contains("apiKey") && !out.contains("scopes"), "{out}");
    }

    // Extras are ordered and kind-agnostic, and an entry without them carries
    // no `extra` key at all — the whole point of the `Option`.
    #[test]
    fn extra_fields_round_trip_in_order_and_stay_absent_when_unset() {
        let entry: Entry = serde_json::from_str(
            r#"{"id":"1","type":"identity","title":"Licence","extra":[
                 {"label":"Categories","value":"B, BE"},
                 {"label":"Blood type","value":"O+"}]}"#,
        )
        .unwrap();
        let extra = entry.extra.clone().unwrap();
        assert_eq!(extra[0].label, "Categories");
        assert_eq!(extra[1].value, "O+");

        let out = serde_json::to_value(&entry).unwrap();
        assert_eq!(out["extra"][0]["label"], "Categories");
        assert_eq!(out["extra"][1]["label"], "Blood type");

        let bare: Entry =
            serde_json::from_str(r#"{"id":"1","type":"note","title":"Wifi"}"#).unwrap();
        assert!(bare.extra.is_none());
        let out = serde_json::to_string(&bare).unwrap();
        assert!(!out.contains("extra"), "{out}");
    }

    fn meta(has_passkey: bool) -> crate::store::EntryMeta {
        crate::store::EntryMeta {
            id: "1".into(),
            kind: "login".into(),
            title: "Site".into(),
            tags: "[]".into(),
            url_host: "acme.test".into(),
            created_at: 0,
            updated_at: 0,
            deleted_at: None,
            card_brand: None,
            favorite: false,
            has_passkey,
            file_name: None,
            var_count: None,
            username: None,
        }
    }

    // The list needs to know a row has a passkey without revealing one, so the
    // flag has to reach the UI as `hasPasskey` — and never the passkeys.
    #[test]
    fn meta_dto_carries_the_passkey_flag() {
        let flagged = serde_json::to_value(EntryMetaDto::from(&meta(true))).unwrap();
        assert_eq!(flagged["hasPasskey"], true);
        assert!(flagged.get("passkeys").is_none());

        let plain = serde_json::to_value(EntryMetaDto::from(&meta(false))).unwrap();
        assert_eq!(plain["hasPasskey"], false);
    }

    // The env subtitle is drawn from these two alone, so they reach the UI in
    // camelCase when stamped and are left out — not nulled — when they are not,
    // matching `cardBrand` and the optional TS fields the frontend declares.
    #[test]
    fn meta_dto_carries_the_env_file_name_and_var_count_only_when_stamped() {
        let stamped = crate::store::EntryMeta {
            kind: "env".into(),
            file_name: Some(".env.production".into()),
            var_count: Some(14),
            ..meta(false)
        };
        let out = serde_json::to_value(EntryMetaDto::from(&stamped)).unwrap();
        assert_eq!(out["fileName"], ".env.production");
        assert_eq!(out["varCount"], 14);
        assert!(out.get("body").is_none());

        let bare = serde_json::to_value(EntryMetaDto::from(&meta(false))).unwrap();
        assert!(bare.get("fileName").is_none());
        assert!(bare.get("varCount").is_none());
    }

    #[test]
    fn meta_dto_carries_the_username_only_when_stamped_and_present() {
        let stamped = crate::store::EntryMeta {
            username: Some("alex@example.com".into()),
            ..meta(false)
        };
        let out = serde_json::to_value(EntryMetaDto::from(&stamped)).unwrap();
        assert_eq!(out["username"], "alex@example.com");
        assert!(out.get("password").is_none());

        let empty = crate::store::EntryMeta {
            username: Some("".into()),
            ..meta(false)
        };
        assert!(serde_json::to_value(EntryMetaDto::from(&empty))
            .unwrap()
            .get("username")
            .is_none());

        let bare = serde_json::to_value(EntryMetaDto::from(&meta(false))).unwrap();
        assert!(bare.get("username").is_none());
    }
}
