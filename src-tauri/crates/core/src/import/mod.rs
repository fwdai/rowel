//! Third-party import/export. Turns a foreign export file into a list of
//! normalized [`ImportedEntry`] values plus per-row errors — and the reverse for
//! export. This module is **pure**: it only parses/serializes bytes. It never
//! encrypts, never touches the store, and knows nothing about the app's crypto or
//! Tauri. The command layer (`commands::import`) owns the seal + upsert.
//!
//! Adding a format = adding one [`Importer`] adapter and one [`Format`] variant.
//! The trait is byte-in / result-out, so a zipped format (1Password `.1pux`) slots
//! in later without reshaping anything.

mod bitwarden;
mod csv;
mod cxf;
pub mod export;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Deserializer};

/// Read a number a foreign exporter may have written as anything: an integer, a
/// string holding one, a float, or a member it simply left out. Never an error.
///
/// A strict `u64` member makes one odd value fail `from_slice` for the whole
/// document, which costs every entry in the file — the opposite of the contract
/// [`ImportResult`] states, where a bad row is recorded and the batch goes on.
/// So anything unreadable becomes `None` here and the caller decides what an
/// absent value means for its row.
pub(crate) fn lenient_u64<'de, D>(d: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::Number(n) => n.as_u64().or_else(|| {
            // `1.5e9` is a whole number in exponent form — a timestamp written
            // by a JavaScript exporter, not a fraction, so it is read too.
            n.as_f64()
                .filter(|f| f.fract() == 0.0 && *f >= 0.0 && *f <= u64::MAX as f64)
                .map(|f| f as u64)
        }),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

/// [`lenient_u64`] narrowed to a byte. A number too large to be one of the
/// small enumerations a format defines says no more than a non-numeric value
/// does, so it reads as absent as well.
pub(crate) fn lenient_u8<'de, D>(d: D) -> Result<Option<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(lenient_u64(d)?.and_then(|n| u8::try_from(n).ok()))
}

/// The kind of a normalized entry; maps 1:1 onto `models::Entry.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntryKind {
    #[default]
    Login,
    Note,
    Card,
    Identity,
    Ssh,
    Env,
    ApiKey,
}

impl EntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EntryKind::Login => "login",
            EntryKind::Note => "note",
            EntryKind::Card => "card",
            EntryKind::Identity => "identity",
            EntryKind::Ssh => "ssh",
            EntryKind::Env => "env",
            EntryKind::ApiKey => "apikey",
        }
    }

    /// The inverse of [`EntryKind::as_str`]. The `type` column of our own CSV
    /// and `models::Entry.kind` both name a kind this way, so both read it back
    /// through here rather than each keeping its own match to drift.
    pub fn parse(name: &str) -> Option<EntryKind> {
        [
            EntryKind::Login,
            EntryKind::Note,
            EntryKind::Card,
            EntryKind::Identity,
            EntryKind::Ssh,
            EntryKind::Env,
            EntryKind::ApiKey,
        ]
        .into_iter()
        .find(|k| k.as_str() == name)
    }
}

/// A normalized, plaintext entry — the shared intermediate for both import and
/// export. Maps cleanly onto `models::Entry` (the mapping lives at the command
/// boundary so this stays crypto/store-free).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportedEntry {
    pub kind: EntryKind,
    pub title: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub url: Option<String>,
    pub notes: Option<String>,
    pub otp: Option<String>,
    pub tags: Vec<String>,
    // The address a login signs in with when it is not the username (only
    // meaningful when kind == Login). No format has a member for it, so it
    // travels as a labelled field (see `export::labelled_fields`).
    pub email: Option<String>,
    // Card fields (only meaningful when kind == Card). The PIN has no member in
    // Bitwarden or CXF either and travels the same labelled way.
    pub card_number: Option<String>,
    pub card_month: Option<String>,
    pub card_year: Option<String>,
    pub card_cvc: Option<String>,
    pub cardholder: Option<String>,
    pub card_pin: Option<String>,
    // ID-document fields (only meaningful when kind == Identity). The first four
    // are the ones a foreign format has a member for; the rest of the document
    // rides as labelled fields, so the whole document survives a round-trip
    // through any of the three formats we write.
    pub doc_type: Option<String>,
    pub doc_number: Option<String>,
    pub doc_country: Option<String>,
    pub holder_name: Option<String>,
    pub doc_nationality: Option<String>,
    pub doc_birth_date: Option<String>,
    pub doc_sex: Option<String>,
    pub doc_issue_date: Option<String>,
    pub doc_expiry_date: Option<String>,
    pub doc_authority: Option<String>,
    pub doc_personal_number: Option<String>,
    // SSH key fields (only meaningful when kind == Ssh). The private key is the
    // credential; the public line and fingerprint are derivable from it but are
    // carried where a format has room, so a round-trip needs no key parsing.
    pub ssh_private_key: Option<String>,
    pub ssh_public_key: Option<String>,
    pub ssh_fingerprint: Option<String>,
    pub ssh_passphrase: Option<String>,
    // `.env` file fields (only meaningful when kind == Env). The body is the
    // file, verbatim — it is the secret and the canonical form, so it must
    // cross a format byte for byte or not at all. The file name is not secret
    // and only rides along so the file can come back under its own name.
    pub env_body: Option<String>,
    pub env_file_name: Option<String>,
    // API key fields (only meaningful when kind == ApiKey). The token is the
    // credential; the base URL it is sent to travels in `url`, the slot every
    // format already has for a site. Environment, scopes and expiry are plain
    // text and ride wherever a format has room for a labelled field.
    pub api_key: Option<String>,
    pub api_environment: Option<String>,
    pub api_scopes: Option<String>,
    pub api_expires: Option<String>,
    // WebAuthn passkeys (only meaningful when kind == Login). Empty when the
    // source format carries none, which is the case for every CSV dialect.
    pub passkeys: Vec<ImportedPasskey>,
    // Free-form label/value pairs, in source order — meaningful on every kind.
    // Empty when the source carries none; only Bitwarden has somewhere to put
    // them (its custom `fields`), so CSV and CXF always leave this empty.
    pub extra: Vec<(String, String)>,
    // Entry state that belongs to no kind. The star and the three timestamps
    // are carried so an export is a faithful copy and a re-import is not
    // mistaken for a fresh edit — an entry stamped "now" on the way in wins
    // every last-writer-wins sync race against the copy it came from.
    pub favorite: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub password_updated_at: Option<String>,
}

/// The two environments the app's switch has; anything else has no segment.
pub const ENVIRONMENTS: [&str; 2] = ["test", "production"];

impl ImportedEntry {
    /// Where an imported environment goes: onto the switch when it is one of
    /// the two the app knows (in any case), and otherwise into the extras under
    /// the same label — so a `staging` stays in sight as a custom field rather
    /// than in a slot no view shows and the next flick of the switch overwrites.
    pub fn set_environment(&mut self, value: Option<String>) {
        let Some(value) = value else { return };
        let known = value.trim().to_lowercase();
        if ENVIRONMENTS.contains(&known.as_str()) {
            self.api_environment = Some(known);
        } else {
            self.extra
                .push((export::ENVIRONMENT_LABEL.to_owned(), value));
        }
    }
}

/// A passkey as an import parses it. It is the app's own [`crate::models::Passkey`]:
/// the fields are identical and base64url values cross unchanged, so nothing
/// is gained by a second struct and a pair of copy functions between them.
pub type ImportedPasskey = crate::models::Passkey;

/// Treat empty strings as absent so blank export fields never become `""`
/// secrets. Shared by every parser that reads optional text.
pub(crate) fn non_empty(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.is_empty())
}

/// A per-row parse failure. `row` is 1-based in the source file so it points a
/// user at the offending line. A bad row is recorded here, never thrown.
#[derive(Debug, Clone, PartialEq)]
pub struct RowError {
    pub row: usize,
    pub message: String,
}

/// The outcome of parsing a file: the entries that parsed plus the rows that
/// didn't. One malformed row never aborts the batch.
#[derive(Debug, Default, PartialEq)]
pub struct ImportResult {
    pub entries: Vec<ImportedEntry>,
    pub errors: Vec<RowError>,
}

impl ImportResult {
    fn push_err(&mut self, row: usize, message: impl Into<String>) {
        self.errors.push(RowError {
            row,
            message: message.into(),
        });
    }
}

/// Byte-in / result-out. The whole contract for a format adapter.
pub trait Importer {
    fn parse(&self, bytes: &[u8]) -> ImportResult;
}

/// The formats we can import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Bitwarden,
    Cxf, // FIDO Credential Exchange Format — the other JSON shape we read
    GenericCsv,
    BrowserCsv, // Chrome / Safari share a header-aliased shape
    LastpassCsv,
    KeepassCsv,
}

impl Format {
    /// Resolve an explicit format name from the frontend.
    pub fn from_name(name: &str) -> Option<Format> {
        match name.trim().to_lowercase().as_str() {
            "bitwarden" => Some(Format::Bitwarden),
            "cxf" | "fido" => Some(Format::Cxf),
            "csv" | "generic" | "generic_csv" => Some(Format::GenericCsv),
            "chrome" | "safari" | "browser" | "browser_csv" => Some(Format::BrowserCsv),
            "lastpass" | "lastpass_csv" => Some(Format::LastpassCsv),
            "keepass" | "keepass_csv" => Some(Format::KeepassCsv),
            _ => None,
        }
    }

    /// The adapter for this format.
    pub fn importer(self) -> Box<dyn Importer> {
        match self {
            Format::Bitwarden => Box::new(bitwarden::Bitwarden),
            Format::Cxf => Box::new(cxf::Cxf),
            Format::GenericCsv => Box::new(csv::GenericCsv),
            Format::BrowserCsv => Box::new(csv::BrowserCsv),
            Format::LastpassCsv => Box::new(csv::LastpassCsv),
            Format::KeepassCsv => Box::new(csv::KeepassCsv),
        }
    }
}

/// Best-effort format detection by file name then content — a convenience so the
/// UI can offer "auto". Explicit selection always wins upstream.
pub fn detect(name: &str, bytes: &[u8]) -> Option<Format> {
    let lower = name.to_lowercase();
    if lower.ends_with(".json") {
        return Some(detect_json(bytes));
    }
    if lower.ends_with(".csv") {
        return Some(detect_csv(bytes));
    }
    // Fall back to content sniffing when the extension is unhelpful.
    if bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{') {
        return Some(detect_json(bytes));
    }
    if !bytes.is_empty() {
        return Some(detect_csv(bytes));
    }
    None
}

// Both JSON formats we read are top-level objects; CXF is the one that declares
// a `version` object and an `accounts` array. Parsed once, then thrown away —
// an explicit choice from the UI skips this entirely.
fn detect_json(bytes: &[u8]) -> Format {
    let Ok(doc) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Format::Bitwarden;
    };
    let cxf = doc.get("accounts").is_some_and(serde_json::Value::is_array)
        && doc.get("version").is_some_and(serde_json::Value::is_object);
    if cxf {
        Format::Cxf
    } else {
        Format::Bitwarden
    }
}

// Pick a CSV dialect from the header row's column names.
fn detect_csv(bytes: &[u8]) -> Format {
    let header = String::from_utf8_lossy(bytes);
    let first = header.lines().next().unwrap_or("").to_lowercase();
    let has = |c: &str| first.split(',').any(|h| h.trim().trim_matches('"') == c);
    if has("grouping") || (has("extra") && has("name") && has("url")) {
        Format::LastpassCsv
    } else if has("group") && has("title") {
        Format::KeepassCsv
    } else {
        Format::GenericCsv
    }
}
