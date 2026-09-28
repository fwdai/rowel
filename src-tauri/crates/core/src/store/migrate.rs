//! Boundary adapter between app entries and store `Record`s. This is the one
//! file in the module that references the app's crypto/models — it is glue, not
//! part of the pure trait, and stays thin. The store itself never sees it.

use super::Record;
use crate::crypto::{Cryptor, PayloadCipher};
use crate::error::Result;
use crate::models::Entry;

/// A random 16-byte hex id for an entry the app itself creates (an import row, a
/// login opened for a new passkey). One generator so every such id has the same
/// shape as the frontend's.
pub fn new_entry_id() -> String {
    crate::crypto::random_hex_id()
}

/// Package a plaintext entry + its already-sealed payload into a store `Record`:
/// non-secret metadata to columns, the opaque payload as-is. The single place the
/// metadata/payload convention lives, shared by save and the `.swftx` imports.
pub fn build_record(entry: &Entry, payload: Vec<u8>) -> Result<Record> {
    Ok(Record {
        id: entry.id.clone(),
        kind: entry.kind.clone(),
        title: entry.title.clone(),
        tags: serde_json::to_string(&entry.tags.clone().unwrap_or_default())?,
        url_host: derived_url_host(entry),
        created_at: to_ms(&entry.created_at),
        updated_at: to_ms(&entry.updated_at),
        deleted_at: None,
        payload,
        card_brand: derived_card_brand(entry),
        // Only reaches the DB on insert: `upsert` excludes `favorite` from its
        // update set, so an ordinary save — which arrives with the field at its
        // serde default of false, the editor never sending it — cannot clear an
        // existing row's star. A `.swftx` import is the one caller that sets it,
        // and every entry it brings in is an insert on a fresh vault.
        favorite: entry.favorite,
        has_passkey: derived_has_passkey(entry),
        file_name: derived_file_name(entry),
        var_count: derived_var_count(entry),
        username: derived_username(entry),
    })
}

/// The inverse of [`build_record`], for a `.swftx` export: unseal the row's
/// payload back into a plaintext entry, re-attach the metadata the payload does
/// not carry (the star lives in a column), then obscure it under `out` for the
/// portable file. Paired with `build_record` here so the two cannot drift on
/// which columns survive a backup.
pub fn export_entry(rec: &Record, cipher: &PayloadCipher, out: &Cryptor) -> Result<Entry> {
    let mut entry = cipher.unseal(&rec.id, &rec.payload)?;
    entry.favorite = rec.favorite;
    out.obscure(&entry)
}

// The stored slug for a card entry: its detected network, or "none" so a
// completed derivation is distinguishable from a pre-column NULL.
pub fn derived_card_brand(entry: &Entry) -> Option<String> {
    (entry.kind == "card").then(|| {
        crate::cards::card_brand(entry.number.as_deref().unwrap_or_default())
            .unwrap_or("none")
            .to_string()
    })
}

/// The stored name of an env entry's file (`.env.production`), or `None` when
/// the file was pasted in and never had one — the list then shows the count
/// alone. Not a secret: it is what the file was called on disk, and the row's
/// title usually says more.
pub fn derived_file_name(entry: &Entry) -> Option<String> {
    (entry.kind == "env")
        .then(|| entry.file_name.as_deref().unwrap_or_default().trim())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

/// How many variables an env entry's file defines: `Some` for every env entry
/// (so a stamped row is distinguishable from a pre-column NULL), `None` for
/// every other kind.
pub fn derived_var_count(entry: &Entry) -> Option<i64> {
    (entry.kind == "env").then(|| count_env_vars(entry.body.as_deref().unwrap_or_default()))
}

/// Lines that look like an assignment: `^\s*(export\s+)?[A-Za-z_][A-Za-z0-9_]*\s*=`.
///
/// An approximation, deliberately: a multi-line quoted value whose continuation
/// line happens to read `KEY=…` is counted as a variable here, where the
/// frontend's parser (which tracks quoting) would not. The count is a list
/// subtitle, not a contract, and exactness is the TS parser's job — the two
/// only disagree on a file that is already unusual.
fn count_env_vars(body: &str) -> i64 {
    body.lines().filter(|line| is_assignment(line)).count() as i64
}

fn is_assignment(line: &str) -> bool {
    let rest = line.trim_start();
    // An `export` prefix only counts as one when whitespace follows it;
    // `export=1` is a variable called `export`.
    let rest = match rest.strip_prefix("export") {
        Some(after) if after.starts_with(char::is_whitespace) => after.trim_start(),
        _ => rest,
    };
    let mut chars = rest.char_indices();
    let Some((_, first)) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    let end = chars
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_'))
        .map(|(i, _)| i)
        .unwrap_or(rest.len());
    rest[end..].trim_start().starts_with('=')
}

/// The stored username of a login: `Some` for every login (so a stamped row is
/// distinguishable from a pre-column NULL), `None` for every other kind.
pub fn derived_username(entry: &Entry) -> Option<String> {
    (entry.kind == "login").then(|| {
        entry
            .username
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_string()
    })
}

/// Whether the entry holds any passkey — the plaintext flag a listing reads
/// instead of unsealing the payload. Not secret: that an account has a passkey
/// is no more revealing than that it has a password.
pub fn derived_has_passkey(entry: &Entry) -> bool {
    entry.passkeys.as_ref().is_some_and(|keys| !keys.is_empty())
}

/// The stored host: the website's, or — for a login that only has passkeys —
/// the first one's `rp_id`.
///
/// A passkey-only login imported from another manager routinely carries no
/// website at all, and the relying-party id *is* the site the credential is
/// bound to. Without the fallback such a row shows an empty subtitle and cannot
/// be found by typing the site's name, which is how anyone would look for it.
/// Mirrored in the frontend's `toEntryMeta`; keep the two in step.
fn derived_url_host(entry: &Entry) -> String {
    let host = host_of(entry.website.as_deref().unwrap_or_default());
    if !host.is_empty() {
        return host;
    }
    entry
        .passkeys
        .as_deref()
        .unwrap_or_default()
        .first()
        .map(|key| key.rp_id.clone())
        .unwrap_or_default()
}

/// Re-seal `.swftx` entries (obscured under the source `Cryptor`, possibly a
/// different master password) under the current session's payload cipher: expose
/// each entry's plaintext, seal it, build a `Record`. Used by `import_backup` and
/// `import_swftx`.
pub fn reseal_swftx(
    entries: &[Entry],
    src: &Cryptor,
    cipher: &PayloadCipher,
) -> Result<Vec<Record>> {
    entries.iter().map(|e| reseal_one(e, src, cipher)).collect()
}

/// Re-seal a single `.swftx` entry (used by the progress-emitting import loop).
pub fn reseal_one(obscured: &Entry, src: &Cryptor, cipher: &PayloadCipher) -> Result<Record> {
    let plain = src.expose(obscured)?;
    let payload = cipher.seal(&plain)?;
    build_record(&plain, payload)
}

fn host_of(website: &str) -> String {
    let s = website.trim();
    let s = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);
    s.split('/').next().unwrap_or("").to_string()
}

fn to_ms(iso: &Option<String>) -> i64 {
    iso.as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis())
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis())
}
