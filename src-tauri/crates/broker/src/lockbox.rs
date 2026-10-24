//! The lockbox side of a host: what any host — the app's `AppHost`, a headless
//! one — does to the open store on the extension's behalf, over the core's
//! types alone, so each of them lands the same row the same way.

use rowel_core::crypto::PayloadCipher;
use rowel_core::error::{store_err, Error};
use rowel_core::models::Entry;
use rowel_core::store::{migrate, SqliteStore, VaultStore};

use crate::protocol::Code;

pub fn save_failed(e: Error) -> Code {
    log::warn!("browser host: could not save a login: {e}");
    Code::ActionCancelledOrDenied
}

/// One row sealed and upserted, as `commands::vault::save_entry` writes one:
/// over the login `id` — its username and password, the rest of the entry
/// kept — or as a new login for `url` titled `host`. The stamps are the
/// editor's: `updatedAt` on every save, and the rotation stamp and password
/// history only when the password actually changed (the same
/// `Entry::record_password_change` the editor's save goes through). `now` is
/// passed in so a test can pin it.
#[allow(clippy::too_many_arguments)]
pub fn save_login_in(
    store: &SqliteStore,
    cipher: &PayloadCipher,
    id: Option<&str>,
    url: &str,
    host: &str,
    username: &str,
    password: &str,
    now: &str,
) -> Result<(), Code> {
    let mut stored = match id {
        Some(id) => {
            let record = store
                .get(id)
                .map_err(|e| save_failed(store_err(e)))?
                .ok_or(Code::NoValidUuidProvided)?;
            let entry = cipher
                .unseal(&record.id, &record.payload)
                .map_err(save_failed)?;
            if entry.kind != "login" {
                return Err(Code::NoValidUuidProvided);
            }
            Some(entry)
        }
        None => None,
    };
    // The stored row becomes the saved one by move, never by copy: what this
    // save replaces — the password, and the bookkeeping that follows it — is
    // taken out first for `record_password_change` to weigh, and the rest (the
    // OTP seed, the passkeys and their keys) crosses once, so no second copy of
    // any secret is left behind for a drop that would not scrub it.
    let mut previous = stored.as_mut().map(|s| Entry {
        password: s.password.take(),
        password_history: s.password_history.take(),
        password_updated_at: s.password_updated_at.take(),
        ..Default::default()
    });
    let mut entry = match stored {
        Some(mut entry) => {
            // Written back to the field it was served from: `logins_for` fills
            // the extension's username from `username`, or from `email` when
            // that is blank, so a login kept by its email must not grow a
            // second name beside it — unless what the page sent is not an
            // email at all, which the email field would only reject; that
            // goes to `username`, and the email stays what it was.
            let blank = |field: &Option<String>| field.as_deref().unwrap_or_default().is_empty();
            if blank(&entry.username) && !blank(&entry.email) && username.contains('@') {
                entry.email = Some(username.to_string());
            } else {
                entry.username = Some(username.to_string());
            }
            entry.password = Some(password.to_string());
            entry.updated_at = Some(now.to_string());
            entry
        }
        None => Entry {
            id: migrate::new_entry_id(),
            kind: "login".into(),
            title: host.to_string(),
            website: Some(url.to_string()),
            username: Some(username.to_string()),
            password: Some(password.to_string()),
            created_at: Some(now.to_string()),
            updated_at: Some(now.to_string()),
            ..Default::default()
        },
    };
    // Takes the old password with it, into the history or scrubbed, so
    // `previous` drops holding nothing.
    entry.record_password_change(previous.as_mut(), now);
    let payload = cipher.seal(&entry).map_err(save_failed)?;
    let record = migrate::build_record(&entry, payload).map_err(save_failed)?;
    store.upsert(&record).map_err(|e| save_failed(store_err(e)))
}
