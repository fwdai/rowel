//! Import/export commands — the boundary between the pure `import` parser and the
//! app's crypto/store. Parsing produces plaintext `ImportedEntry` values; writing
//! seals each with the session payload cipher (`PayloadCipher::seal` +
//! `migrate::build_record`) and writes the lot in one transaction — the same
//! seal/record convention as
//! `import_swftx`, so payload sealing is never reimplemented here.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::crypto::PayloadCipher;
use crate::error::{Error, Result};
use crate::events;
use crate::import::{self, EntryKind, Format, ImportedEntry, RowError};
use crate::models::{Entry, EntryMetaDto, ExtraField};
use crate::save;
use crate::session::{list_metas, live_records, store_err};
use crate::state::AppState;
use crate::storage;
use crate::store::{migrate, Record, SqliteStore, VaultStore};

// Bound the input: a foreign export should never be gigabytes or millions of rows.
const MAX_BYTES: u64 = 25 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowErrorDto {
    pub row: usize,
    pub message: String,
}

impl From<&RowError> for RowErrorDto {
    fn from(e: &RowError) -> Self {
        RowErrorDto {
            row: e.row,
            message: e.message.clone(),
        }
    }
}

// Preview (dry_run): `imported` is 0 and `total` is the would-be count. Real run:
// `imported` is what was written, `skipped` the rows that failed to parse,
// `duplicates` the rows dropped because the vault already held them verbatim,
// and `entries` the refreshed vault (empty on a preview, which wrote nothing).
// A preview counts duplicates too, so what it shows is what a real run does.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub total: usize,
    pub imported: usize,
    pub skipped: usize,
    pub duplicates: usize,
    pub dry_run: bool,
    pub errors: Vec<RowErrorDto>,
    pub entries: Vec<EntryMetaDto>,
}

// What one blocking pass over the file produced: the parse, plus the sealed rows
// when it was a real run (a preview seals nothing). The plaintext rows are kept
// alongside the sealed ones because the duplicate check compares plaintext, and
// it can only run later — under the session lock, where the store is.
struct Parsed {
    total: usize,
    errors: Vec<RowErrorDto>,
    entries: Vec<ImportedEntry>,
    records: Vec<Record>,
}

// Parse a foreign export and either preview it (dry_run) or write it into the open
// vault. `format` is an explicit name or "auto" to detect by extension/content.
//
// The read, the parse and the seal loop are one hop onto the blocking pool: a
// 25 MB export is disk I/O followed by a CPU-bound pass over every row, and
// neither belongs on the IPC thread or on an async worker.
#[tauri::command]
pub async fn import_entries(
    path: String,
    format: String,
    dry_run: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ImportReport> {
    // Taken before the file work, so a locked vault is turned away at once
    // instead of after parsing. A preview writes nothing and so needs no cipher,
    // but it still reads a path the webview named and reports what is in it —
    // that is a read of the user's disk, and only an open vault may ask for one.
    // The epoch comes with the cipher: the write below is only accepted by the
    // session the cipher belongs to (see `Session::store_at`).
    let (cipher, epoch) = match dry_run {
        true => {
            state.session.lock().unwrap().key()?;
            (None, None)
        }
        false => {
            let session = state.session.lock().unwrap();
            (Some(session.payload_cipher()?), Some(session.epoch()))
        }
    };

    let emitter = app.clone();
    let parsed = super::blocking(move || -> Result<Parsed> {
        let bytes = storage::read_regular_file_capped(Path::new(&path), MAX_BYTES)?;

        let fmt = if format.eq_ignore_ascii_case("auto") {
            let name = Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            import::detect(&name, &bytes)
                .ok_or_else(|| Error::Other("could not detect format".into()))?
        } else {
            Format::from_name(&format)
                .ok_or_else(|| Error::Other(format!("unknown format: {format}")))?
        };

        let parsed = fmt.importer().parse(&bytes);
        if parsed.entries.len() > MAX_ENTRIES {
            return Err(Error::Other("too many entries to import".into()));
        }
        let total = parsed.entries.len();
        let errors: Vec<RowErrorDto> = parsed.errors.iter().map(RowErrorDto::from).collect();

        // Seal every plaintext entry (seal payload + build a Record), emitting
        // progress — the same seal helper the rest of the app uses.
        let mut records = Vec::new();
        if let Some(cipher) = cipher {
            records.reserve(total);
            for (i, imported) in parsed.entries.iter().enumerate() {
                let entry = imported_to_entry(imported);
                let payload = cipher.seal(&entry)?;
                records.push(migrate::build_record(&entry, payload)?);
                events::import_progress(&emitter, i + 1, total);
            }
        }
        Ok(Parsed {
            total,
            errors,
            entries: parsed.entries,
            records,
        })
    })
    .await?;

    // The store lives behind the session mutex and is only reachable from this
    // thread, so the duplicate check and the write happen together, here.
    let session = state.session.lock().unwrap();

    if dry_run {
        // The vault was open when the preview began (checked above), but the
        // parse ran outside the lock and a lock may have landed since. A
        // preview writes nothing, so a vault that closed under it is not an
        // error — there is merely nothing left to compare against, and it
        // reports no duplicates.
        let duplicates = match (session.store(), session.payload_cipher()) {
            (Ok(store), Ok(cipher)) => duplicate_flags(store, &cipher, &parsed.entries)?
                .iter()
                .filter(|dup| **dup)
                .count(),
            _ => 0,
        };
        return Ok(ImportReport {
            total: parsed.total,
            imported: 0,
            skipped: parsed.errors.len(),
            duplicates,
            dry_run: true,
            errors: parsed.errors,
            entries: Vec::new(),
        });
    }

    let store = match epoch {
        Some(epoch) => session.store_at(epoch)?,
        None => session.store()?,
    };
    let flags = duplicate_flags(store, &session.payload_cipher()?, &parsed.entries)?;
    let fresh: Vec<Record> = parsed
        .records
        .into_iter()
        .zip(flags)
        .filter(|(_, duplicate)| !*duplicate)
        .map(|(record, _)| record)
        .collect();
    // One transaction for the whole file: a crash partway through must leave the
    // vault as it was, not half-imported. `import` writes each record's own
    // timestamps verbatim, which is what we want — `build_record` already
    // stamped every row when it was sealed above.
    store.import(&fresh).map_err(store_err)?;
    crate::credential_identities::publish(&app);

    Ok(ImportReport {
        total: parsed.total,
        imported: fresh.len(),
        skipped: parsed.errors.len(),
        // Every parsed row was sealed, so whatever `total` did not survive the
        // filter was dropped as a duplicate.
        duplicates: parsed.total - fresh.len(),
        dry_run: false,
        errors: parsed.errors,
        entries: list_metas(store)?,
    })
}

// Which of `entries` the vault already holds, verbatim. Re-running an import —
// after a crash, or simply by accident — must not double the vault; but a row
// that merely resembles one already there (the password has since changed) is a
// real import, so the test is equality of the whole normalized entry and
// nothing fuzzier. `ImportedEntry` carries no ids; its bookkeeping (timestamps,
// the star) is cleared on both sides first, since a foreign file rarely has
// them and the vault always does — without that, `==` would never match.
//
// Candidates are narrowed on plaintext columns first — `list` reads no payload
// — so the only rows unsealed are the handful sharing a kind and a title with
// something in the file. (The url is not part of the key: it is compared in the
// equality below anyway, and the store's host derivation is private to it.)
fn duplicate_flags(
    store: &SqliteStore,
    cipher: &PayloadCipher,
    entries: &[ImportedEntry],
) -> Result<Vec<bool>> {
    let metas = store.list().map_err(store_err)?;
    let wanted: HashSet<(&str, &str)> = entries.iter().map(dedupe_key).collect();
    let mut candidates: HashMap<(&str, &str), Vec<ImportedEntry>> = HashMap::new();
    for meta in &metas {
        let key = (meta.kind.as_str(), meta.title.as_str());
        if !wanted.contains(&key) {
            continue;
        }
        if let Some(record) = store.get(&meta.id).map_err(store_err)? {
            candidates
                .entry(key)
                .or_default()
                .push(without_bookkeeping(entry_to_imported(
                    &cipher.unseal(&record.id, &record.payload)?,
                )));
        }
    }
    Ok(entries
        .iter()
        .map(|imported| {
            candidates
                .get(&dedupe_key(imported))
                .is_some_and(|rows| rows.contains(&without_bookkeeping(imported.clone())))
        })
        .collect())
}

// The fields that say when and how an entry was kept, not what it is.
fn without_bookkeeping(mut entry: ImportedEntry) -> ImportedEntry {
    entry.favorite = false;
    entry.created_at = None;
    entry.updated_at = None;
    entry.password_updated_at = None;
    entry
}

// The plaintext columns a duplicate must share before it is worth unsealing.
fn dedupe_key(entry: &ImportedEntry) -> (&str, &str) {
    (entry.kind.as_str(), entry.title.as_str())
}

// When a caller-chosen export destination is honoured, as a decision on its own
// so the rule can be read (and tested) without an environment around it.
const fn explicit_path_allowed(debug: bool, e2e: bool) -> bool {
    debug && e2e
}

// The same two gates `commands::e2e` applies, which cannot be borrowed from it:
// that whole module is `#[cfg(debug_assertions)]` and this one is not, so the
// debug check has to be a `cfg!` value here rather than a missing symbol.
// `ROWEL_E2E=1` is the second gate because `tauri dev` is a debug build too, and
// a developer's own run must not expose this to a stray `invoke`.
fn e2e_enabled() -> bool {
    explicit_path_allowed(
        cfg!(debug_assertions),
        std::env::var("ROWEL_E2E").as_deref() == Ok("1"),
    )
}

// Export the open vault to a third-party format. `path` is honoured only in an
// E2E build (see below); otherwise a save dialog is shown. `format` is
// "bitwarden", "cxf" or "csv".
#[tauri::command]
pub async fn export_entries(
    path: Option<String>,
    format: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>> {
    // Under the guard: read the sealed rows and take a *copy* of the payload
    // cipher. Unsealing every one of them is a pass over the whole vault, and
    // the session lock may not be held across that — the cipher is an owned
    // value precisely so it can leave with the records.
    let (cipher, records) = {
        let session = state.session.lock().unwrap();
        (session.payload_cipher()?, live_records(session.store()?)?)
    };

    let (bytes, ext) = super::blocking(move || {
        let entries = to_imported(&records, &cipher)?;
        Ok(match format.to_lowercase().as_str() {
            "bitwarden" => (import::export::to_bitwarden_json(&entries)?, "json"),
            "cxf" | "fido" => (import::export::to_cxf_json(&entries)?, "json"),
            "csv" => (
                import::export::to_generic_csv(&entries)
                    .map_err(|e| Error::Other(e.to_string()))?,
                "csv",
            ),
            other => return Err(Error::Other(format!("unknown export format: {other}"))),
        })
    })
    .await?;

    // An explicit path skips the dialog (the E2E suite exports to a temp file),
    // which makes it a webview-reachable "write the whole plaintext vault to
    // this path" — the exfiltration route the CSP exists to close. So it is the
    // suite's alone; every other build refuses it and the user's own export
    // goes through a dialog they chose the destination in.
    let dest = match path {
        Some(p) if e2e_enabled() => {
            let dest = save::with_extension(std::path::PathBuf::from(p), ext);
            save::write_and_scrub(&dest, bytes)?;
            Some(dest)
        }
        Some(_) => {
            return Err(Error::Other(
                "explicit export paths are only available to the e2e suite".into(),
            ))
        }
        None => save::save_export(&app, &format!("rowel-export.{ext}"), "Export", bytes).await?,
    };
    Ok(dest.map(|p| p.to_string_lossy().into_owned()))
}

// Stored records -> the plaintext rows an export is written from. Split out of
// the command so the unseal path can be tested without a Tauri app around it.
fn to_imported(records: &[Record], cipher: &PayloadCipher) -> Result<Vec<ImportedEntry>> {
    records
        .iter()
        .map(|r| {
            // The star lives in a column, not in the sealed payload, so it is
            // re-attached here — the same move `migrate::export_entry` makes
            // for a `.swftx` backup.
            let mut entry = cipher.unseal(&r.id, &r.payload)?;
            entry.favorite = r.favorite;
            Ok(entry_to_imported(&entry))
        })
        .collect()
}

// ImportedEntry -> a plaintext models::Entry, ready to be obscured + sealed.
fn imported_to_entry(imp: &ImportedEntry) -> Entry {
    let now = chrono::Utc::now();
    // The kind's own fields are set in the match below; everything else stays
    // at its default (None, so a field the kind does not own never serializes).
    let mut e = Entry {
        id: migrate::new_entry_id(),
        kind: imp.kind.as_str().to_string(),
        title: imp.title.clone(),
        note: imp.notes.clone(),
        tags: (!imp.tags.is_empty()).then(|| imp.tags.clone()),
        // Extras belong to no kind, so they are mapped here rather than in the
        // match below. None when there are none, for the same reason.
        extra: (!imp.extra.is_empty()).then(|| {
            imp.extra
                .iter()
                .map(|(label, value)| ExtraField {
                    label: label.clone(),
                    value: value.clone(),
                })
                .collect()
        }),
        // The star and the stamps the source carried, not this moment: an entry
        // stamped "now" on the way in is a newer copy of itself and wins every
        // last-writer-wins sync race against the vault it came from. Only a
        // source that has no dates of its own — or dates it cannot have — falls
        // back to now; see `no_later_than`.
        favorite: imp.favorite,
        created_at: Some(no_later_than(imp.created_at.as_deref(), now)),
        updated_at: Some(no_later_than(imp.updated_at.as_deref(), now)),
        password_updated_at: imp.password_updated_at.clone(),
        ..Default::default()
    };
    match imp.kind {
        EntryKind::Login => {
            e.username = imp.username.clone();
            e.password = imp.password.clone();
            e.website = imp.url.clone();
            e.email = imp.email.clone();
            e.otp = imp.otp.clone();
            e.passkeys = (!imp.passkeys.is_empty()).then(|| imp.passkeys.clone());
        }
        EntryKind::Card => {
            e.number = imp.card_number.clone();
            e.month = imp.card_month.clone();
            e.year = imp.card_year.clone();
            e.cvc = imp.card_cvc.clone();
            e.name = imp.cardholder.clone();
            e.pin = imp.card_pin.clone();
        }
        EntryKind::Identity => {
            e.doc_type = imp.doc_type.clone();
            e.number = imp.doc_number.clone();
            e.country = imp.doc_country.clone();
            e.name = imp.holder_name.clone();
            e.nationality = imp.doc_nationality.clone();
            e.birth_date = imp.doc_birth_date.clone();
            e.sex = imp.doc_sex.clone();
            e.issue_date = imp.doc_issue_date.clone();
            // `expiry_date` is the slot an API key uses too, so it is only
            // written here for the kind it belongs to.
            e.expiry_date = imp.doc_expiry_date.clone();
            e.authority = imp.doc_authority.clone();
            e.personal_number = imp.doc_personal_number.clone();
        }
        EntryKind::Ssh => {
            e.private_key = imp.ssh_private_key.clone();
            e.public_key = imp.ssh_public_key.clone();
            e.fingerprint = imp.ssh_fingerprint.clone();
            e.passphrase = imp.ssh_passphrase.clone();
        }
        EntryKind::Env => {
            e.body = imp.env_body.clone();
            e.file_name = imp.env_file_name.clone();
        }
        EntryKind::ApiKey => {
            e.api_key = imp.api_key.clone();
            e.base_url = imp.url.clone();
            e.environment = imp.api_environment.clone();
            e.scopes = imp.api_scopes.clone();
            e.expiry_date = imp.api_expires.clone();
        }
        EntryKind::Note => {}
    }
    e
}

// The stamp a source carried, kept verbatim — unless it is unreadable or lies
// in the future, when it is `now` instead. A foreign file is untrusted input,
// and sync settles a conflict purely by the greater `updated_at`
// (`SqliteStore::merge_records`): an entry that arrived dated 2099 would beat
// every edit anyone made to it afterwards, on every device, for good. The past
// is what we want to keep (see the caller); only the future is refused.
fn no_later_than(stamp: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> String {
    match stamp.and_then(|s| Some((s, chrono::DateTime::parse_from_rfc3339(s).ok()?))) {
        Some((verbatim, parsed)) if parsed <= now => verbatim.to_owned(),
        _ => now.to_rfc3339(),
    }
}

// A plaintext (exposed) models::Entry -> ImportedEntry for export.
fn entry_to_imported(e: &Entry) -> ImportedEntry {
    // A kind the app does not know is exported as the login it most resembles,
    // rather than refused — the same fallback the editor makes.
    let kind = EntryKind::parse(&e.kind).unwrap_or(EntryKind::Login);
    // `number` and `name` are shared slots, so they are only read into the
    // identity columns for an identity — a card must not export as one.
    let identity = kind == EntryKind::Identity;
    let api_key = kind == EntryKind::ApiKey;
    ImportedEntry {
        kind,
        title: e.title.clone(),
        username: e.username.clone(),
        password: e.password.clone(),
        // An API key's site is the API it is sent to.
        url: if api_key {
            e.base_url.clone()
        } else {
            e.website.clone()
        },
        notes: e.note.clone(),
        otp: e.otp.clone(),
        tags: e.tags.clone().unwrap_or_default(),
        email: e.email.clone(),
        card_number: (!identity).then(|| e.number.clone()).flatten(),
        card_month: e.month.clone(),
        card_year: e.year.clone(),
        card_cvc: e.cvc.clone(),
        cardholder: (!identity).then(|| e.name.clone()).flatten(),
        card_pin: e.pin.clone(),
        doc_type: e.doc_type.clone(),
        doc_number: identity.then(|| e.number.clone()).flatten(),
        doc_country: e.country.clone(),
        holder_name: identity.then(|| e.name.clone()).flatten(),
        doc_nationality: e.nationality.clone(),
        doc_birth_date: e.birth_date.clone(),
        doc_sex: e.sex.clone(),
        doc_issue_date: e.issue_date.clone(),
        // The other half of the shared `expiry_date`: a passport's, read only
        // for a passport (see `api_expires` below).
        doc_expiry_date: identity.then(|| e.expiry_date.clone()).flatten(),
        doc_authority: e.authority.clone(),
        doc_personal_number: e.personal_number.clone(),
        ssh_private_key: e.private_key.clone(),
        ssh_public_key: e.public_key.clone(),
        ssh_fingerprint: e.fingerprint.clone(),
        ssh_passphrase: e.passphrase.clone(),
        env_body: e.body.clone(),
        env_file_name: e.file_name.clone(),
        api_key: e.api_key.clone(),
        api_environment: e.environment.clone(),
        api_scopes: e.scopes.clone(),
        // `expiry_date` is shared with the identity, so it is only read here
        // for an API key — a passport must not export with one.
        api_expires: api_key.then(|| e.expiry_date.clone()).flatten(),
        passkeys: e.passkeys.clone().unwrap_or_default(),
        extra: e
            .extra
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|f| (f.label.clone(), f.value.clone()))
            .collect(),
        favorite: e.favorite,
        created_at: e.created_at.clone(),
        updated_at: e.updated_at.clone(),
        password_updated_at: e.password_updated_at.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{self, KdfParams, VaultKey};
    use crate::store::SqliteStore;

    // Cheap Argon2id params: this proves which cipher the export reads with,
    // not how expensive the KDF is.
    fn argon2_vault(path: &Path) -> (VaultKey, SqliteStore) {
        let params = KdfParams::argon2id(b"salt-for-the-export-test", 256, 1, 1);
        let key = VaultKey::Argon2 {
            master: crypto::derive(b"master-password", &params).unwrap(),
        };
        let store = SqliteStore::open(path, &*key.sqlcipher_key()).unwrap();
        (key, store)
    }

    // Both gates, not either: a release build the suite's env var happens to be
    // set in is still a release build, and a developer's `tauri dev` is still a
    // debug build.
    #[test]
    fn only_a_debug_build_running_the_suite_may_name_its_own_destination() {
        assert!(explicit_path_allowed(true, true));
        assert!(!explicit_path_allowed(true, false));
        assert!(!explicit_path_allowed(false, true));
        assert!(!explicit_path_allowed(false, false));
    }

    fn tmp_db() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rowel-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vault.db");
        let _ = std::fs::remove_file(&path);
        path
    }

    // Every vault this app creates seals payloads with the AEAD payload cipher,
    // so an export that reaches for the legacy cryptor reads nothing at all.
    #[test]
    fn exports_an_entry_sealed_the_way_save_entry_seals_it() {
        let path = tmp_db();
        let (key, store) = argon2_vault(&path);
        let entry: Entry = serde_json::from_value(serde_json::json!({
            "id": "1", "type": "login", "title": "Site",
            "website": "https://ex.com/login", "username": "alice", "password": "s3cret"
        }))
        .unwrap();

        let cipher = key.payload_cipher();
        let record = migrate::build_record(&entry, cipher.seal(&entry).unwrap()).unwrap();
        store.upsert(&record).unwrap();

        let records = live_records(&store).unwrap();
        let exported = to_imported(&records, &cipher).unwrap();

        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0].title, "Site");
        assert_eq!(exported[0].username.as_deref(), Some("alice"));
        assert_eq!(exported[0].password.as_deref(), Some("s3cret"));

        drop(store);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    fn login(password: &str) -> ImportedEntry {
        ImportedEntry {
            kind: EntryKind::Login,
            title: "Site".into(),
            username: Some("alice".into()),
            password: Some(password.into()),
            url: Some("https://ex.com".into()),
            ..Default::default()
        }
    }

    // Re-running the same import — after a crash, or by accident — must not
    // double the vault. Ids are fresh on every pass, so only the content can
    // say that a row is already there.
    #[test]
    fn a_second_import_of_the_same_rows_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (key, store) = argon2_vault(&dir.path().join("vault.db"));
        let cipher = key.payload_cipher();
        let rows = vec![
            login("s3cret"),
            ImportedEntry {
                kind: EntryKind::Note,
                title: "Note".into(),
                notes: Some("body".into()),
                ..Default::default()
            },
        ];
        let seal = |rows: &[ImportedEntry]| -> Vec<Record> {
            rows.iter()
                .map(|row| {
                    let entry = imported_to_entry(row);
                    migrate::build_record(&entry, cipher.seal(&entry).unwrap()).unwrap()
                })
                .collect()
        };

        assert_eq!(
            duplicate_flags(&store, &cipher, &rows).unwrap(),
            [false, false]
        );
        store.import(&seal(&rows)).unwrap();

        // Second pass: both rows are already there, so nothing is written.
        let flags = duplicate_flags(&store, &cipher, &rows).unwrap();
        assert_eq!(flags, [true, true]);
        let fresh: Vec<Record> = seal(&rows)
            .into_iter()
            .zip(flags)
            .filter(|(_, duplicate)| !*duplicate)
            .map(|(record, _)| record)
            .collect();
        store.import(&fresh).unwrap();
        assert_eq!(store.list().unwrap().len(), 2);
    }

    // Sync picks a conflict's winner by the greater `updated_at` alone, so a
    // file dated in the future would outrank every later edit, everywhere. A
    // stamp from the past is kept as written; the future — and a stamp that is
    // not a date at all — becomes now.
    #[test]
    fn a_future_or_unreadable_stamp_is_clamped_to_now_and_a_past_one_kept() {
        let before = chrono::Utc::now();
        let mut row = login("s3cret");
        row.created_at = Some("2020-01-02T03:04:05+00:00".into());
        row.updated_at = Some("2099-01-01T00:00:00+00:00".into());
        let entry = imported_to_entry(&row);
        assert_eq!(
            entry.created_at.as_deref(),
            Some("2020-01-02T03:04:05+00:00")
        );
        let updated =
            chrono::DateTime::parse_from_rfc3339(entry.updated_at.as_deref().unwrap()).unwrap();
        assert!(
            updated >= before && updated <= chrono::Utc::now(),
            "{updated}"
        );

        row.updated_at = Some("yesterday-ish".into());
        let entry = imported_to_entry(&row);
        assert!(
            chrono::DateTime::parse_from_rfc3339(entry.updated_at.as_deref().unwrap())
                .is_ok_and(|t| t >= before)
        );
    }

    // A near match is a real import: the same account with a rotated password is
    // news, not a re-run of the file it came from.
    #[test]
    fn a_row_that_only_resembles_a_stored_one_is_not_a_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let (key, store) = argon2_vault(&dir.path().join("vault.db"));
        let cipher = key.payload_cipher();
        let entry = imported_to_entry(&login("s3cret"));
        store
            .import(&[migrate::build_record(&entry, cipher.seal(&entry).unwrap()).unwrap()])
            .unwrap();

        assert_eq!(
            duplicate_flags(&store, &cipher, &[login("rotated")]).unwrap(),
            [false]
        );
    }
}
