//! CSV adapters. All share one reader + a header-name lookup, then each maps
//! columns its own way: generic/browser via aliases, LastPass and KeePass via
//! their fixed schemas. `row` numbers are 1-based over the source file (header is
//! row 1), so a reported error points at the real line.

use ::csv::{ReaderBuilder, StringRecord};

use super::{EntryKind, ImportResult, ImportedEntry, Importer};

// Alias sets for the generic/browser mapper (headers are lowercased + trimmed).
const TITLE: &[&str] = &["title", "name", "account", "item", "item name"];
const USERNAME: &[&str] = &[
    "username",
    "user",
    "login",
    "login_username",
    "user name",
    "email",
];
const PASSWORD: &[&str] = &["password", "pass", "login_password"];
const URL: &[&str] = &[
    "url",
    "urls",
    "website",
    "uri",
    "login_uri",
    "web site",
    "site",
    "link",
];
const NOTES: &[&str] = &["notes", "note", "extra", "comments", "comment"];
const OTP: &[&str] = &[
    "otp",
    "totp",
    "otpauth",
    "2fa",
    "token",
    "otp secret",
    "totp secret",
];
// Rowel's own columns (see `export::COLUMNS`): no foreign sheet has them, so
// each is its own name rather than an alias set.
const TYPE: &[&str] = &["type"];
const BODY: &[&str] = &["body"];
const FILE_NAME: &[&str] = &["file_name"];
const API_KEY: &[&str] = &["api_key"];
const ENVIRONMENT: &[&str] = &["environment"];
const SCOPES: &[&str] = &["scopes"];
const EXPIRES: &[&str] = &["expires"];
const CARD_NUMBER: &[&str] = &["card_number"];
const CARD_MONTH: &[&str] = &["card_month"];
const CARD_YEAR: &[&str] = &["card_year"];
const CARD_CVC: &[&str] = &["card_cvc"];
const CARDHOLDER: &[&str] = &["cardholder"];
const CARD_PIN: &[&str] = &["card_pin"];
const DOC_TYPE: &[&str] = &["doc_type"];
const DOC_NUMBER: &[&str] = &["doc_number"];
const DOC_COUNTRY: &[&str] = &["doc_country"];
const HOLDER_NAME: &[&str] = &["holder_name"];
const DOC_NATIONALITY: &[&str] = &["doc_nationality"];
const DOC_BIRTH_DATE: &[&str] = &["doc_birth_date"];
const DOC_SEX: &[&str] = &["doc_sex"];
const DOC_ISSUE_DATE: &[&str] = &["doc_issue_date"];
const DOC_EXPIRY_DATE: &[&str] = &["doc_expiry_date"];
const DOC_AUTHORITY: &[&str] = &["doc_authority"];
const DOC_PERSONAL_NUMBER: &[&str] = &["doc_personal_number"];
const SSH_PRIVATE_KEY: &[&str] = &["ssh_private_key"];
const SSH_PUBLIC_KEY: &[&str] = &["ssh_public_key"];
const SSH_FINGERPRINT: &[&str] = &["ssh_fingerprint"];
const SSH_PASSPHRASE: &[&str] = &["ssh_passphrase"];
const TAGS: &[&str] = &["tags"];
// `username` on its own: the alias set above also answers to `email`, which is
// a column of its own on our sheet and must not stand in for the username.
const OWN_USERNAME: &[&str] = &["username"];
const EMAIL: &[&str] = &["email"];
const FAVORITE: &[&str] = &["favorite"];
const CREATED_AT: &[&str] = &["created_at"];
const UPDATED_AT: &[&str] = &["updated_at"];
const PASSWORD_UPDATED_AT: &[&str] = &["password_updated_at"];
const CSV_VERSION: &[&str] = &[super::export::CSV_VERSION_HEADER];

// A parsed sheet: header names (normalized) and the data rows.
struct Rows {
    headers: Vec<String>,
    records: Vec<StringRecord>,
}

// Read CSV into rows, recording (never throwing) header and per-row failures.
fn read_rows(bytes: &[u8], result: &mut ImportResult) -> Option<Rows> {
    let mut rdr = ReaderBuilder::new()
        .flexible(true)
        .has_headers(true)
        .from_reader(bytes);
    let headers = match rdr.headers() {
        Ok(h) => h.iter().map(|s| s.trim().to_lowercase()).collect(),
        Err(e) => {
            result.push_err(0, format!("invalid CSV header: {e}"));
            return None;
        }
    };
    let mut records = Vec::new();
    for (i, rec) in rdr.records().enumerate() {
        match rec {
            Ok(r) => records.push(r),
            Err(e) => result.push_err(i + 2, format!("malformed row: {e}")),
        }
    }
    Some(Rows { headers, records })
}

// First non-empty cell whose header matches one of `aliases`, untrimmed: a
// `.env` file's leading indent and trailing newline are part of the file, and
// the entry promises to hand it back verbatim.
fn get_verbatim(headers: &[String], rec: &StringRecord, aliases: &[&str]) -> Option<String> {
    headers.iter().enumerate().find_map(|(idx, h)| {
        if !aliases.contains(&h.as_str()) {
            return None;
        }
        rec.get(idx).filter(|v| !v.is_empty()).map(String::from)
    })
}

// The same cell, trimmed — what every column but the file wants.
fn get(headers: &[String], rec: &StringRecord, aliases: &[&str]) -> Option<String> {
    get_verbatim(headers, rec, aliases)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

// Reverse spreadsheet escaping only for a row carrying Rowel's explicit
// format marker. A generic sheet's leading apostrophe is always literal data.
fn decoded(value: Option<String>, rowel: bool) -> Option<String> {
    value.map(|v| {
        if rowel {
            super::export::unsanitize_cell(&v)
        } else {
            v
        }
    })
}

fn get_decoded(
    headers: &[String],
    rec: &StringRecord,
    aliases: &[&str],
    rowel: bool,
) -> Option<String> {
    decoded(get_verbatim(headers, rec, aliases), rowel)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn tag_vec(group: Option<String>) -> Vec<String> {
    group
        .map(|g| g.trim_start_matches("Root/").trim().to_string())
        .filter(|g| !g.is_empty())
        .map(|g| vec![g])
        .unwrap_or_default()
}

/// Generic CSV mapped by header aliases. Chrome and Safari exports fit the same
/// aliases, so [`BrowserCsv`] just delegates here.
pub struct GenericCsv;

/// Chrome / Safari CSV — same header-aliased shape as generic.
pub struct BrowserCsv;

fn parse_aliased(bytes: &[u8]) -> ImportResult {
    let mut result = ImportResult::default();
    let Some(rows) = read_rows(bytes, &mut result) else {
        return result;
    };
    for (i, rec) in rows.records.iter().enumerate() {
        // Any marker at all, not this version's: successive versions only add
        // columns at the end, which read as absent when a sheet predates them,
        // so an older Rowel export still comes back as what it was.
        let rowel = get(&rows.headers, rec, CSV_VERSION).is_some();
        let cell = |aliases| get_decoded(&rows.headers, rec, aliases, rowel);
        let title = cell(TITLE).or_else(|| cell(URL)).or_else(|| cell(USERNAME));
        let Some(title) = title else {
            result.push_err(i + 2, "empty row");
            continue;
        };
        // Our own sheet names every row's kind and writes every kind's columns,
        // so it is read back by the `type` column alone — nothing is guessed
        // from a row's shape, which is how cards, notes, identities and keys
        // used to come back as logins with their own columns dropped.
        if rowel {
            match EntryKind::parse(cell(TYPE).unwrap_or_default().as_str()) {
                Some(kind) => result
                    .entries
                    .push(rowel_row(&rows.headers, rec, kind, title)),
                None => result.push_err(i + 2, "unknown entry type"),
            }
            continue;
        }
        // Below here the sheet is a foreign one, where every row is a login
        // unless its shape says otherwise. An `env` row has no login's shape —
        // no login column names the file — so it is one of the two kinds the
        // `type` column is read for, or the file would be dropped on the way
        // back in.
        if cell(TYPE).as_deref() == Some(EntryKind::Env.as_str()) {
            result.entries.push(ImportedEntry {
                kind: EntryKind::Env,
                title,
                notes: cell(NOTES),
                env_body: decoded(get_verbatim(&rows.headers, rec, BODY), rowel),
                env_file_name: cell(FILE_NAME),
                ..Default::default()
            });
            continue;
        }
        // An API key row has a login's shape — a URL, a secret — so it, too,
        // is told apart by the `type` column, or it would come back as a login
        // with an empty password.
        if cell(TYPE).as_deref() == Some(EntryKind::ApiKey.as_str()) {
            let mut entry = ImportedEntry {
                kind: EntryKind::ApiKey,
                title,
                url: cell(URL),
                notes: cell(NOTES),
                api_key: cell(API_KEY),
                api_scopes: cell(SCOPES),
                api_expires: cell(EXPIRES),
                ..Default::default()
            };
            entry.set_environment(cell(ENVIRONMENT));
            result.entries.push(entry);
            continue;
        }
        result.entries.push(ImportedEntry {
            kind: EntryKind::Login,
            title,
            username: cell(USERNAME),
            password: cell(PASSWORD),
            url: cell(URL),
            notes: cell(NOTES),
            otp: cell(OTP),
            ..Default::default()
        });
    }
    result
}

// One row of Rowel's own sheet, whose kind is already known: the columns that
// belong to no kind, then the kind's own. Every cell goes through the version-2
// unescaping, since the marker is what got us here.
fn rowel_row(
    headers: &[String],
    rec: &StringRecord,
    kind: EntryKind,
    title: String,
) -> ImportedEntry {
    let cell = |aliases| get_decoded(headers, rec, aliases, true);
    let mut entry = ImportedEntry {
        kind,
        title,
        notes: cell(NOTES),
        tags: cell(TAGS)
            .map(|t| {
                t.split(';')
                    .map(|tag| tag.trim().to_string())
                    .filter(|tag| !tag.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
        favorite: cell(FAVORITE).as_deref() == Some("true"),
        created_at: cell(CREATED_AT),
        updated_at: cell(UPDATED_AT),
        ..Default::default()
    };
    match kind {
        EntryKind::Login => {
            entry.username = cell(OWN_USERNAME);
            entry.password = cell(PASSWORD);
            entry.url = cell(URL);
            entry.otp = cell(OTP);
            entry.email = cell(EMAIL);
            entry.password_updated_at = cell(PASSWORD_UPDATED_AT);
        }
        EntryKind::Card => {
            entry.card_number = cell(CARD_NUMBER);
            entry.card_month = cell(CARD_MONTH);
            entry.card_year = cell(CARD_YEAR);
            entry.card_cvc = cell(CARD_CVC);
            entry.cardholder = cell(CARDHOLDER);
            entry.card_pin = cell(CARD_PIN);
        }
        EntryKind::Identity => {
            entry.doc_type = cell(DOC_TYPE);
            entry.doc_number = cell(DOC_NUMBER);
            entry.doc_country = cell(DOC_COUNTRY);
            entry.holder_name = cell(HOLDER_NAME);
            entry.doc_nationality = cell(DOC_NATIONALITY);
            entry.doc_birth_date = cell(DOC_BIRTH_DATE);
            entry.doc_sex = cell(DOC_SEX);
            entry.doc_issue_date = cell(DOC_ISSUE_DATE);
            entry.doc_expiry_date = cell(DOC_EXPIRY_DATE);
            entry.doc_authority = cell(DOC_AUTHORITY);
            entry.doc_personal_number = cell(DOC_PERSONAL_NUMBER);
        }
        EntryKind::Ssh => {
            entry.ssh_private_key = cell(SSH_PRIVATE_KEY);
            entry.ssh_public_key = cell(SSH_PUBLIC_KEY);
            entry.ssh_fingerprint = cell(SSH_FINGERPRINT);
            entry.ssh_passphrase = cell(SSH_PASSPHRASE);
        }
        EntryKind::Env => {
            // The file is the secret and the canonical form, so it is the one
            // cell read untrimmed — its indent and trailing newline are data.
            entry.env_body = decoded(get_verbatim(headers, rec, BODY), true);
            entry.env_file_name = cell(FILE_NAME);
        }
        EntryKind::ApiKey => {
            entry.url = cell(URL);
            entry.api_key = cell(API_KEY);
            entry.api_scopes = cell(SCOPES);
            entry.api_expires = cell(EXPIRES);
            entry.set_environment(cell(ENVIRONMENT));
        }
        EntryKind::Note => {}
    }
    entry
}

impl Importer for GenericCsv {
    fn parse(&self, bytes: &[u8]) -> ImportResult {
        parse_aliased(bytes)
    }
}

impl Importer for BrowserCsv {
    fn parse(&self, bytes: &[u8]) -> ImportResult {
        parse_aliased(bytes)
    }
}

/// LastPass CSV: `url,username,password,totp,extra,name,grouping,fav`. Secure
/// notes carry the sentinel url `http://sn`.
pub struct LastpassCsv;

impl Importer for LastpassCsv {
    fn parse(&self, bytes: &[u8]) -> ImportResult {
        let mut result = ImportResult::default();
        let Some(rows) = read_rows(bytes, &mut result) else {
            return result;
        };
        for (i, rec) in rows.records.iter().enumerate() {
            let h = &rows.headers;
            let url = get(h, rec, &["url"]);
            let title = get(h, rec, &["name"]).or_else(|| url.clone());
            let Some(title) = title else {
                result.push_err(i + 2, "empty row");
                continue;
            };
            let tags = tag_vec(get(h, rec, &["grouping"]));
            let notes = get(h, rec, &["extra"]);
            if url.as_deref() == Some("http://sn") {
                result.entries.push(ImportedEntry {
                    kind: EntryKind::Note,
                    title,
                    notes,
                    tags,
                    ..Default::default()
                });
            } else {
                result.entries.push(ImportedEntry {
                    kind: EntryKind::Login,
                    title,
                    username: get(h, rec, &["username"]),
                    password: get(h, rec, &["password"]),
                    url,
                    otp: get(h, rec, &["totp"]),
                    notes,
                    tags,
                    ..Default::default()
                });
            }
        }
        result
    }
}

/// KeePassXC CSV: `Group,Title,Username,Password,URL,Notes,TOTP,...`. The Group
/// path becomes a tag (its `Root/` prefix stripped).
pub struct KeepassCsv;

impl Importer for KeepassCsv {
    fn parse(&self, bytes: &[u8]) -> ImportResult {
        let mut result = ImportResult::default();
        let Some(rows) = read_rows(bytes, &mut result) else {
            return result;
        };
        for (i, rec) in rows.records.iter().enumerate() {
            let h = &rows.headers;
            let Some(title) = get(h, rec, &["title"]) else {
                result.push_err(i + 2, "row has no title");
                continue;
            };
            result.entries.push(ImportedEntry {
                kind: EntryKind::Login,
                title,
                username: get(h, rec, &["username"]),
                password: get(h, rec, &["password"]),
                url: get(h, rec, &["url"]),
                notes: get(h, rec, &["notes"]),
                otp: get(h, rec, &["totp"]),
                tags: tag_vec(get(h, rec, &["group"])),
                ..Default::default()
            });
        }
        result
    }
}
