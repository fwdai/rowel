//! The lockbox side of a host: what any host — the app's `AppHost`, a headless
//! one — does to the open store on the extension's or an agent's behalf, over
//! the core's types alone, so each of them lands the same row the same way and
//! reads the same rows back: the logins for a page, a TOTP code, the clients
//! let in, the rules the user wrote, and the field of an item a placeholder
//! names.

use std::collections::HashMap;

use zeroize::Zeroizing;

use rowel_core::crypto::PayloadCipher;
use rowel_core::error::{store_err, Error, Result};
use rowel_core::models::Entry;
use rowel_core::store::{migrate, EntryMeta, SqliteStore, VaultStore};

use crate::actions::{self, Client, Login};
use crate::policy::Rule;
use crate::protocol::Code;
use crate::reference::{self, Field};

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
) -> std::result::Result<(), Code> {
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

/// Every login in `store` for the page at `page_url` (whose host is `host`),
/// secrets unsealed.
///
/// Two passes. The plaintext `url_host` column picks the candidates cheaply,
/// without unsealing a payload per row; but it is not authenticated — anyone
/// holding the SQLCipher key could re-point a row at another site — so it
/// only narrows. What decides is the website inside the sealed payload, which
/// each candidate is checked against once it is open, by the full rule
/// ([`actions::site_matches`]: host, then scheme and port). A login with no
/// website is held to the passkey rpId its column was derived from, exactly
/// as `migrate::build_record` derives it.
pub fn logins_in(
    store: &SqliteStore,
    cipher: &PayloadCipher,
    page_url: &str,
    host: &str,
) -> Vec<Login> {
    let metas = match store.list() {
        Ok(metas) => metas,
        Err(e) => {
            log::warn!("browser host: could not list the vault: {e}");
            return Vec::new();
        }
    };
    let mut logins = Vec::new();
    for meta in metas
        .iter()
        .filter(|m| m.kind == "login" && actions::host_matches(host, &m.url_host))
    {
        let Ok(Some(record)) = store.get(&meta.id) else {
            continue;
        };
        let Ok(entry) = cipher.unseal(&record.id, &record.payload) else {
            log::warn!("browser host: entry {} does not open", meta.id);
            continue;
        };
        let saved = entry
            .website
            .as_deref()
            .filter(|website| !website.trim().is_empty())
            .or_else(|| {
                entry
                    .passkeys
                    .as_deref()
                    .and_then(|keys| keys.first())
                    .map(|key| key.rp_id.as_str())
            })
            .unwrap_or_default();
        if !actions::site_matches(page_url, saved) {
            continue;
        }
        let username = entry.login_name().unwrap_or_default().to_string();
        logins.push(Login {
            id: entry.id.clone(),
            title: entry.title.clone(),
            username,
            password: entry.password.clone().unwrap_or_default(),
            totp: otp_code(&entry),
        });
    }
    logins
}

/// The current TOTP code of the entry `id`, as `get-totp` serves it: `None`
/// when there is no such row, it does not open, or it holds no seed.
pub fn totp_in(store: &SqliteStore, cipher: &PayloadCipher, id: &str) -> Option<String> {
    let record = store.get(id).ok()??;
    let entry = cipher.unseal(&record.id, &record.payload).ok()?;
    otp_code(&entry)
}

/// The code `entry`'s seed is good for now, if it has one.
fn otp_code(entry: &Entry) -> Option<String> {
    entry
        .otp
        .as_deref()
        .filter(|otp| !otp.is_empty())
        .and_then(|otp| rowel_core::otp::code_now(otp).ok())
        .map(|otp| otp.code)
}

// --- the clients let in ------------------------------------------------------
//
// A `meta` row of the vault they were let into, sealed with the rest of it.
// In the vault rather than in `settings.json` for two reasons. The key is the
// whole credential — the protocol never asks the extension to sign with the
// private half — so on disk it must be as unreadable as what it opens. And an
// association is to one vault: the extension keys its own copy by the vault
// hash, and a workspace the user never let it into must not answer to it.
// Every read and write happens under the session lock, so two approvals
// cannot each write a list that misses the other's.

/// The `meta` key the clients let in are sealed under.
pub const CLIENTS_META: &str = "browser_clients";

pub fn clients_in(store: &SqliteStore) -> Vec<Client> {
    store
        .meta_get(CLIENTS_META)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

pub fn write_clients(store: &SqliteStore, clients: &[Client]) -> Result<()> {
    store
        .meta_set(CLIENTS_META, &serde_json::to_string(clients)?)
        .map_err(store_err)
}

/// A known client proved itself again: its `last_seen_at_ms`, one write per
/// connection. A write that fails loses nothing but the timestamp.
pub fn mark_seen(store: &SqliteStore, key: &str, now: i64) {
    let mut clients = clients_in(store);
    let Some(client) = clients.iter_mut().find(|c| c.key == key) else {
        return;
    };
    client.last_seen_at_ms = Some(now);
    if let Err(e) = write_clients(store, &clients) {
        log::warn!("browser host: could not note the client was seen: {e}");
    }
}

// --- the rules the user wrote -------------------------------------------------

/// The `meta` key the lockbox's own rules are sealed under: beside the
/// clients, for the same reasons — a rule names a client key that exists only
/// on this device, so there is nothing in it another device could use, and
/// the sync merge ignores `meta` anyway.
pub const RULES_META: &str = "agent_rules";

pub fn rules_in(store: &SqliteStore) -> Vec<Rule> {
    store
        .meta_get(RULES_META)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

pub fn write_rules(store: &SqliteStore, rules: &[Rule]) -> Result<()> {
    store
        .meta_set(RULES_META, &serde_json::to_string(rules)?)
        .map_err(store_err)
}

/// The lockbox's own rules that still apply at `now_ms`. A rule past its end
/// is dropped here and the shorter list written back, because the evaluator
/// has no clock and Settings should not list what no longer counts.
pub fn live_rules(store: &SqliteStore, now_ms: i64) -> Vec<Rule> {
    let rules = rules_in(store);
    let live: Vec<Rule> = rules
        .iter()
        .filter(|r| r.is_live(now_ms))
        .cloned()
        .collect();
    if live.len() != rules.len() {
        if let Err(e) = write_rules(store, &live) {
            log::warn!("agents: could not drop the expired rules: {e}");
        }
    }
    live
}

/// Every row's title by its id: what a list shows beside an item id.
pub fn titles_in(store: &SqliteStore) -> HashMap<String, String> {
    store
        .list()
        .map(|metas| {
            metas
                .into_iter()
                .map(|m| (m.id, m.title))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default()
}

// --- what a placeholder names --------------------------------------------------

/// The row `name` picks out among `metas`: the one with that id, or else
/// the one whose title's slug it is. Two items with the same slug name
/// neither — an agent that wrote `stripe` must not be served whichever of
/// two Stripes the list happens to put first — and ids always work.
pub fn item_by_name<'m>(metas: &'m [EntryMeta], name: &str) -> Option<&'m EntryMeta> {
    if let Some(meta) = metas.iter().find(|meta| meta.id == name) {
        return Some(meta);
    }
    let mut slugged = metas
        .iter()
        .filter(|meta| reference::slug(&meta.title) == name);
    let first = slugged.next()?;
    slugged.next().is_none().then_some(first)
}

/// The secret `field` of `entry`, when it has one that is not empty. An OTP
/// is the current code, as `get-totp` serves it; a `body.KEY` is that one
/// variable of an env item's file.
pub fn field_value(entry: &Entry, field: &Field) -> Option<Zeroizing<String>> {
    let value = match field {
        Field::Password => entry.password.clone(),
        Field::Otp => otp_code(entry),
        Field::ApiKey => entry.api_key.clone(),
        Field::PrivateKey => entry.private_key.clone(),
        Field::Number => entry.number.clone(),
        Field::Cvc => entry.cvc.clone(),
        Field::Body(key) => entry.body.as_deref().and_then(|body| env_var(body, key)),
    };
    value.filter(|value| !value.is_empty()).map(Zeroizing::new)
}

/// The value of `key` in an env file's `body`, read the way a shell would
/// read the common cases and no further: blank lines and `#` comments are
/// skipped, an `export ` prefix is allowed, a value in single or double
/// quotes is what is between them, an unquoted value ends at a ` #` comment,
/// and the last assignment of the name wins, as it does for a shell, dotenv
/// and `docker --env-file`. A tolerant parser rather than a shell: nothing is
/// expanded, so a value is what was written and never what another variable
/// held.
pub fn env_var(body: &str, key: &str) -> Option<String> {
    body.lines().rev().find_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let line = line.strip_prefix("export ").map_or(line, str::trim_start);
        let (name, value) = line.split_once('=')?;
        (name.trim() == key).then(|| env_value(value.trim()))
    })
}

/// One env file value, unquoted: what is between a quote and the last of its
/// kind (a comment may follow), or up to a whitespace-led `#`.
fn env_value(value: &str) -> String {
    for quote in ['"', '\''] {
        if let Some(rest) = value.strip_prefix(quote) {
            if let Some(end) = rest.rfind(quote) {
                return rest[..end].to_string();
            }
        }
    }
    let end = [" #", "\t#"]
        .iter()
        .filter_map(|comment| value.find(comment))
        .min()
        .unwrap_or(value.len());
    value[..end].trim_end().to_string()
}
