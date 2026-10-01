//! The pure half of sharing: the one-time key, the sealed envelope, and the
//! link that carries both halves of what a recipient needs.
//!
//! Nothing here touches the network, the vault or Tauri. The key is generated
//! here and never leaves the sender's machine except inside the link, so Drive
//! only ever holds bytes it cannot read.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{Error, Result};
use crate::models::Entry;

/// How long a share lives. Fixed rather than chosen: a share is meant to be
/// opened now, and a dial the sender has to think about is one more way to
/// leave a credential lying in someone else's Drive.
pub const SHARE_TTL_MS: i64 = 24 * 60 * 60 * 1000;

pub fn expires_at(now_ms: i64) -> i64 {
    now_ms + SHARE_TTL_MS
}

/// The most a share file may be. An entry is a few kilobytes; an `.env` file
/// a few hundred at the outside. The id in a pasted link can name any public
/// file on Drive, so the recipient never buffers more than this — and the
/// sender is told before upload, since a share over it could never be opened.
pub const MAX_SHARE_BYTES: usize = 2 * 1024 * 1024;

/// Every kind this build can store. A share carrying anything else is refused
/// on receipt rather than saved as a row no view knows how to render.
const KINDS: [&str; 7] = ["login", "note", "card", "identity", "ssh", "env", "apikey"];

const KEY_LEN: usize = 32;
const VERSION: u8 = 1;
const PREFIX: &str = "rowel://share#";
const VERSION_TAG: &str = "v1";

/// The AES-256 key a single share is sealed under. Fresh per share, never
/// derived from the vault: losing it costs the recipient the share and nothing
/// else.
pub struct ShareKey(Zeroizing<[u8; KEY_LEN]>);

impl ShareKey {
    pub fn generate() -> Self {
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        rand::thread_rng().fill_bytes(&mut *key);
        Self(key)
    }
}

impl AsRef<[u8]> for ShareKey {
    fn as_ref(&self) -> &[u8] {
        &*self.0
    }
}

// The key is the whole secret of a share, and links get logged.
impl std::fmt::Debug for ShareKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ShareKey(redacted)")
    }
}

/// Refuse what sanitizing would silently hollow out. A login whose only secret
/// is a passkey would arrive as a username and nothing else, and the sender
/// would not know why; better to say so before a link exists.
pub fn check_shareable(entry: &Entry) -> Result<()> {
    let has_passkey = entry.passkeys.as_ref().is_some_and(|p| !p.is_empty());
    let has_secret = [&entry.password, &entry.otp]
        .iter()
        .any(|v| v.as_deref().is_some_and(|s| !s.is_empty()));
    if entry.kind == "login" && has_passkey && !has_secret {
        // `Unsupported` rather than a kind of its own: what the sender can do
        // about it is the sentence, not the category (see `src/api/errors.ts`).
        return Err(Error::Unsupported(
            "this login holds only a passkey, and passkeys cannot be shared".into(),
        ));
    }
    Ok(())
}

/// Strip everything the recipient's vault must decide for itself.
///
/// The id is theirs to assign, the timestamps describe the sender's copy, the
/// star is the sender's opinion, a copied passkey would be a second
/// authenticator the site never registered, and the passwords the sender used
/// to have are not part of what they chose to share. Everything else — tags,
/// extra fields, the secret itself — is the point of the share and travels
/// intact.
///
/// Applied on both ends. Sending, so nothing leaves that should not; receiving,
/// because the plaintext is whatever the link's author sealed, and a crafted
/// envelope must not be able to name an existing row's id and replace it.
pub fn sanitize(entry: &Entry) -> Entry {
    Entry {
        id: String::new(),
        created_at: None,
        updated_at: None,
        password_updated_at: None,
        password_history: None,
        favorite: false,
        passkeys: None,
        ..entry.clone()
    }
}

/// The sealed plaintext. `entry` is generic so the reader can check the version
/// before parsing it: a future format may shape the entry differently, and that
/// has to read as "newer version", not as a parse failure.
///
/// The expiry rides inside the ciphertext, so it is authenticated by the same
/// tag as the entry: nobody holding the file — Drive included — can extend it.
#[derive(Serialize, Deserialize)]
struct Envelope<E> {
    v: u8,
    #[serde(rename = "expiresAt")]
    expires_at: i64,
    entry: E,
}

pub fn seal(key: &ShareKey, entry: &Entry, expires_ms: i64) -> Result<Vec<u8>> {
    let plaintext = serde_json::to_vec(&Envelope {
        v: VERSION,
        expires_at: expires_ms,
        entry: &sanitize(entry),
    })?;
    let sealed = crate::crypto::seal_aead(key.as_ref(), &[], &plaintext)?;
    // The recipient refuses anything over the cap, so a share this big would
    // upload fine and then fail everyone it was sent to. Refuse it here, where
    // the sender can still do something about it.
    if sealed.len() > MAX_SHARE_BYTES {
        return Err(Error::EntryTooLargeToShare);
    }
    Ok(sealed)
}

/// Open a share as of `now_ms`. What comes back is safe to save as a new row:
/// expired, unknown-kind and sender-supplied identity are all refused or
/// stripped here, before the frontend ever sees it.
pub fn unseal(key: &ShareKey, blob: &[u8], now_ms: i64) -> Result<Entry> {
    // A wrong key and a tampered file are the same event to the user: the link
    // they have does not open this share. The AEAD error underneath says
    // nothing they can act on.
    let plaintext =
        crate::crypto::unseal_aead(key.as_ref(), &[], blob).map_err(|_| Error::ShareLinkInvalid)?;

    let envelope: Envelope<serde_json::Value> = serde_json::from_slice(&plaintext)?;
    if envelope.v != VERSION {
        return Err(Error::ShareTooNew);
    }
    if envelope.expires_at <= now_ms {
        return Err(Error::ShareExpired);
    }

    let entry: Entry = serde_json::from_value(envelope.entry)?;
    // A kind this build has no view for can only have come from one that does,
    // so it reads as the same "made by a newer version" the envelope tag does.
    if !KINDS.contains(&entry.kind.as_str()) {
        return Err(Error::ShareTooNew);
    }
    Ok(sanitize(&entry))
}

/// A whole share in one pasteable token: `rowel://share#v1.<fileId>.<key>`.
///
/// One token rather than a URL with query parameters, because both halves have
/// to survive being pasted into a chat window by hand, and because a fragment
/// keeps the key out of anything that would forward the link to a server.
#[derive(Debug)]
pub struct Link {
    pub file_id: String,
    pub key: ShareKey,
}

impl Link {
    pub fn format(&self) -> String {
        format!(
            "{PREFIX}{VERSION_TAG}.{}.{}",
            self.file_id,
            URL_SAFE_NO_PAD.encode(self.key.as_ref())
        )
    }

    pub fn parse(s: &str) -> Result<Link> {
        let rest = s
            .trim()
            .strip_prefix(PREFIX)
            .ok_or(Error::ShareLinkInvalid)?;
        let mut parts = rest.split('.');

        let version = parts.next().unwrap_or_default();
        if version != VERSION_TAG {
            // A tag whose shape we recognize but whose number we don't can only
            // come from a build that speaks a format this one doesn't.
            return Err(if is_version_tag(version) {
                Error::ShareTooNew
            } else {
                Error::ShareLinkInvalid
            });
        }

        let file_id = parts.next().unwrap_or_default();
        let key = parts.next().ok_or(Error::ShareLinkInvalid)?;
        if parts.next().is_some() {
            return Err(Error::ShareLinkInvalid);
        }

        if file_id.is_empty() {
            return Err(Error::ShareLinkInvalid);
        }
        if !file_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(Error::ShareLinkInvalid);
        }

        // Held zeroized on the way in: this is the key, in the clear, for as
        // long as the decode's buffer lives.
        let decoded = Zeroizing::new(
            URL_SAFE_NO_PAD
                .decode(key)
                .map_err(|_| Error::ShareLinkInvalid)?,
        );
        if decoded.len() != KEY_LEN {
            return Err(Error::ShareLinkInvalid);
        }
        let mut bytes = Zeroizing::new([0u8; KEY_LEN]);
        bytes.copy_from_slice(&decoded);

        Ok(Link {
            file_id: file_id.to_string(),
            key: ShareKey(bytes),
        })
    }
}

fn is_version_tag(s: &str) -> bool {
    matches!(s.strip_prefix('v'), Some(rest) if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry() -> Entry {
        serde_json::from_value(json!({
            "id": "entry-1", "type": "login", "title": "Site",
            "website": "https://ex.com/login", "username": "alice",
            "password": "s3cret", "otp": "SEED",
            "tags": ["work", "eu"],
            "extra": [{"label": "PIN", "value": "1234"}],
            "favorite": true,
            "createdAt": "2024-01-01T00:00:00Z",
            "updatedAt": "2024-02-01T00:00:00Z",
            "password_updated_at": "2024-02-01T00:00:00Z",
            "passwordHistory": [{ "password": "0ld-s3cret", "replacedAt": "2024-02-01T00:00:00Z" }],
            "passkeys": [{
                "credentialId": "Y3JlZA", "rpId": "ex.com",
                "userHandle": "dXNlcg", "userName": "alice",
                "userDisplayName": "Alice", "privateKey": "cHJpdmF0ZUtleQ",
                "counter": 0
            }]
        }))
        .unwrap()
    }

    const NOW: i64 = 1_700_000_000_000;

    fn sealed(key: &ShareKey) -> Vec<u8> {
        seal(key, &entry(), expires_at(NOW)).unwrap()
    }

    #[test]
    fn seal_round_trips_the_entry_without_the_sender_s_copy() {
        let key = ShareKey::generate();
        let blob = sealed(&key);
        assert!(!blob.windows(6).any(|w| w == b"s3cret"));

        let back = unseal(&key, &blob, NOW).unwrap();
        // The sender's previous passwords never leave their vault.
        assert!(back.password_history.is_none());
        assert!(!serde_json::to_string(&sanitize(&entry()))
            .unwrap()
            .contains("0ld-s3cret"));
        assert_eq!(back.title, "Site");
        assert_eq!(back.password.as_deref(), Some("s3cret"));
        assert_eq!(back.otp.as_deref(), Some("SEED"));
        assert_eq!(back.username.as_deref(), Some("alice"));
        assert_eq!(
            back.tags.as_deref(),
            Some(&["work".into(), "eu".into()][..])
        );
        assert_eq!(back.extra.unwrap()[0].value, "1234");

        assert_eq!(back.id, "");
        assert!(back.created_at.is_none());
        assert!(back.updated_at.is_none());
        assert!(back.password_updated_at.is_none());
        assert!(!back.favorite);
        assert!(back.passkeys.is_none());
    }

    #[test]
    fn tampered_ciphertext_does_not_open() {
        let key = ShareKey::generate();
        let mut blob = sealed(&key);
        let last = blob.len() - 1;
        blob[last] ^= 1;
        assert!(matches!(
            unseal(&key, &blob, NOW).unwrap_err(),
            Error::ShareLinkInvalid
        ));
    }

    #[test]
    fn wrong_key_does_not_open() {
        let blob = sealed(&ShareKey::generate());
        assert!(unseal(&ShareKey::generate(), &blob, NOW).is_err());
    }

    #[test]
    fn a_newer_envelope_version_is_refused() {
        let key = ShareKey::generate();
        let plaintext = serde_json::to_vec(
            &json!({"v": 2, "expiresAt": i64::MAX, "entry": {"shape": "unknown"}}),
        )
        .unwrap();
        let blob = crate::crypto::seal_aead(key.as_ref(), &[], &plaintext).unwrap();
        assert!(matches!(
            unseal(&key, &blob, NOW).unwrap_err(),
            Error::ShareTooNew
        ));
    }

    // The sender's device may never get to delete the file; the recipient's
    // clock is what turns the link off.
    #[test]
    fn an_expired_envelope_is_refused_even_though_the_file_still_opens() {
        let key = ShareKey::generate();
        let blob = sealed(&key);
        assert!(unseal(&key, &blob, NOW + SHARE_TTL_MS - 1).is_ok());
        assert!(matches!(
            unseal(&key, &blob, NOW + SHARE_TTL_MS).unwrap_err(),
            Error::ShareExpired
        ));
    }

    // Sealed by hand, the way an attacker would: the plaintext names an
    // existing id and carries everything sanitizing normally strips.
    fn crafted(key: &ShareKey, entry: serde_json::Value) -> Vec<u8> {
        let plaintext =
            serde_json::to_vec(&json!({"v": 1, "expiresAt": i64::MAX, "entry": entry})).unwrap();
        crate::crypto::seal_aead(key.as_ref(), &[], &plaintext).unwrap()
    }

    #[test]
    fn a_crafted_envelope_cannot_smuggle_identity_or_passkeys_in() {
        let key = ShareKey::generate();
        let mut smuggled = serde_json::to_value(entry()).unwrap();
        smuggled["id"] = json!("victim-row");
        let back = unseal(&key, &crafted(&key, smuggled), NOW).unwrap();

        assert_eq!(back.id, "");
        assert!(back.created_at.is_none());
        assert!(back.updated_at.is_none());
        assert!(!back.favorite);
        assert!(back.passkeys.is_none());
        assert_eq!(back.password.as_deref(), Some("s3cret"));
    }

    #[test]
    fn an_unknown_kind_is_refused_on_receipt() {
        let key = ShareKey::generate();
        let blob = crafted(&key, json!({"id": "", "type": "wallet", "title": "x"}));
        assert!(matches!(
            unseal(&key, &blob, NOW).unwrap_err(),
            Error::ShareTooNew
        ));
    }

    #[test]
    fn a_passkey_only_login_cannot_be_shared_but_one_with_a_password_can() {
        let with_password = entry();
        assert!(check_shareable(&with_password).is_ok());

        let mut passkey_only = entry();
        passkey_only.password = None;
        passkey_only.otp = Some(String::new());
        assert_eq!(
            check_shareable(&passkey_only).unwrap_err().to_string(),
            "this login holds only a passkey, and passkeys cannot be shared"
        );

        let mut plain = entry();
        plain.password = None;
        plain.otp = None;
        plain.passkeys = None;
        assert!(check_shareable(&plain).is_ok());
    }

    #[test]
    fn link_round_trips() {
        let original = Link {
            file_id: "1A-bC_dEfG".into(),
            key: ShareKey::generate(),
        };
        let parsed = Link::parse(&original.format()).unwrap();
        assert_eq!(parsed.file_id, original.file_id);
        assert_eq!(parsed.key.as_ref(), original.key.as_ref());
    }

    #[test]
    fn surrounding_whitespace_is_ignored() {
        let original = Link {
            file_id: "fileId".into(),
            key: ShareKey::generate(),
        };
        let pasted = format!("  {}\n", original.format());
        assert_eq!(Link::parse(&pasted).unwrap().file_id, "fileId");
    }

    #[test]
    fn parse_refuses_anything_that_is_not_a_link() {
        let key = URL_SAFE_NO_PAD.encode([7u8; KEY_LEN]);
        for input in [
            "https://example.com/share".to_string(),
            format!("rowel://open#v1.fileId.{key}"),
            format!("rowel://share#v1.file!d.{key}"),
            format!("rowel://share#v1.fileId.{key}.extra"),
        ] {
            assert!(
                matches!(Link::parse(&input).unwrap_err(), Error::ShareLinkInvalid),
                "{input}"
            );
        }
    }

    #[test]
    fn parse_refuses_an_incomplete_link() {
        let key = URL_SAFE_NO_PAD.encode([7u8; KEY_LEN]);
        for input in [
            "rowel://share#v1.fileId".to_string(),
            format!("rowel://share#v1..{key}"),
            format!("rowel://share#v1.fileId.{}", &key[..20]),
            format!(
                "rowel://share#v1.fileId.{}",
                URL_SAFE_NO_PAD.encode([7u8; 64])
            ),
        ] {
            assert!(
                matches!(Link::parse(&input).unwrap_err(), Error::ShareLinkInvalid),
                "{input}"
            );
        }
    }

    #[test]
    fn parse_refuses_a_newer_link_version() {
        let key = URL_SAFE_NO_PAD.encode([7u8; KEY_LEN]);
        assert!(matches!(
            Link::parse(&format!("rowel://share#v2.fileId.{key}")).unwrap_err(),
            Error::ShareTooNew
        ));
    }

    #[test]
    fn expires_a_day_out() {
        assert_eq!(expires_at(1_000), 1_000 + 24 * 60 * 60 * 1000);
    }
}
