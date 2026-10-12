//! 1Password's own export, `.1pux` — "1Password Unencrypted Export": a zip
//! holding `export.data`, a JSON document of every account, vault and item in
//! the clear, beside a `files/` folder of attachments. Nothing in it is
//! encrypted; it is the full export, where the CSV 1Password also writes
//! carries logins alone (see `csv::OnepasswordCsv`).
//!
//! An item is a category (a three-digit id) plus the `loginFields` a web form
//! captured (each designated `username` or `password`) and `sections` of typed
//! fields. The app's slots are filled by a field's `id`, which 1Password fixes
//! in English whatever the UI language, never by its `title`. Whatever has no
//! slot travels as a labelled extra, concealed when 1Password conceals it.
//! Tags come across as tags; in an export of more than one vault, the vault's
//! name is a tag too, so the items keep their homes apart.
//!
//! Attachments are not read: an item that has any still imports, and the row
//! carries a warning, so the user knows to keep the export file for them. Nor
//! need the archive be read whole for that: [`unpack`] takes `export.data`
//! out of any reader, so the command layer hands a file on disk straight to
//! it and the attachments beside the JSON never pass through memory.

use std::io::{Cursor, Read, Seek};

use serde::Deserialize;
use serde_json::Value;

use super::{lenient_string, non_empty, rfc3339, EntryKind, ImportResult, ImportedEntry, Importer};
use crate::models::{ExtraField, PasswordHistoryItem, PASSWORD_HISTORY_CAP};

/// The `.1pux` archive — or its `export.data` alone, when a caller has
/// already taken it out with [`unpack`].
pub struct Onepux;

/// Whichever of its two exports 1Password wrote, told apart by content: the
/// archive by its signature, the JSON by its first byte, and otherwise the
/// CSV — so the one "1Password" choice in the UI takes either file.
pub struct Onepassword;

/// The local-file header every zip starts with.
pub fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
}

fn is_json(bytes: &[u8]) -> bool {
    bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{')
}

/// How much of `export.data` is read out of the archive. A zip can claim any
/// size for an entry, and a hostile one must not be allowed to fill memory.
const MAX_DATA_BYTES: u64 = 128 * 1024 * 1024;

// Every list is read leniently, as the other JSON adapters read theirs: one
// odd member must not cost every item in the file.
#[derive(Deserialize)]
struct Export {
    #[serde(default, deserialize_with = "super::null_to_default")]
    accounts: Vec<Account>,
}

#[derive(Deserialize)]
struct Account {
    #[serde(default, deserialize_with = "super::null_to_default")]
    vaults: Vec<Vault>,
}

#[derive(Deserialize)]
struct Vault {
    #[serde(default, deserialize_with = "super::null_to_default")]
    attrs: VaultAttrs,
    #[serde(default, deserialize_with = "super::null_to_default")]
    items: Vec<Item>,
}

#[derive(Deserialize, Default)]
struct VaultAttrs {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct Item {
    #[serde(default, rename = "categoryUuid")]
    category: Option<String>,
    /// `active`, `archived` — or `trashed`, which an export can carry.
    #[serde(default)]
    state: Option<String>,
    #[serde(default, rename = "favIndex", deserialize_with = "super::lenient_u64")]
    fav_index: Option<u64>,
    #[serde(default, rename = "createdAt", deserialize_with = "super::lenient_u64")]
    created_at: Option<u64>,
    #[serde(default, rename = "updatedAt", deserialize_with = "super::lenient_u64")]
    updated_at: Option<u64>,
    #[serde(default, deserialize_with = "super::null_to_default")]
    overview: Overview,
    #[serde(default, deserialize_with = "super::null_to_default")]
    details: Details,
}

#[derive(Deserialize, Default)]
struct Overview {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default, deserialize_with = "super::null_to_default")]
    urls: Vec<Url>,
    #[serde(default, deserialize_with = "super::null_to_default")]
    tags: Vec<String>,
}

#[derive(Deserialize)]
struct Url {
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Deserialize, Default)]
struct Details {
    #[serde(
        default,
        rename = "loginFields",
        deserialize_with = "super::null_to_default"
    )]
    login_fields: Vec<LoginField>,
    /// A Password item's one secret lives here rather than in a field.
    #[serde(default, deserialize_with = "lenient_string")]
    password: Option<String>,
    #[serde(default, rename = "notesPlain")]
    notes: Option<String>,
    #[serde(default, deserialize_with = "super::null_to_default")]
    sections: Vec<Section>,
    #[serde(
        default,
        rename = "passwordHistory",
        deserialize_with = "super::null_to_default"
    )]
    password_history: Vec<History>,
    /// A Document item's file. Not imported — only its presence is needed, to
    /// say so in the report.
    #[serde(default, rename = "documentAttributes")]
    document: Option<Value>,
}

/// What a web form gave 1Password: `designation` says which of them sign in.
#[derive(Deserialize)]
struct LoginField {
    #[serde(default)]
    name: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    value: Option<String>,
    /// `T` text, `P` password, `E` email, `C` checkbox, among others.
    #[serde(default, rename = "fieldType")]
    field_type: Option<String>,
    #[serde(default)]
    designation: Option<String>,
}

#[derive(Deserialize)]
struct Section {
    #[serde(default, deserialize_with = "super::null_to_default")]
    fields: Vec<Field>,
}

#[derive(Deserialize)]
struct Field {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    /// `{"<kind>": payload}` — the kind says how the payload reads.
    #[serde(default)]
    value: Option<Value>,
}

#[derive(Deserialize)]
struct History {
    #[serde(default, deserialize_with = "lenient_string")]
    value: Option<String>,
    #[serde(default, deserialize_with = "super::lenient_u64")]
    time: Option<u64>,
}

impl Field {
    fn typed(&self) -> Option<(&str, &Value)> {
        let (kind, payload) = self.value.as_ref()?.as_object()?.iter().next()?;
        Some((kind.as_str(), payload))
    }

    fn kind(&self) -> &str {
        self.typed().map(|(kind, _)| kind).unwrap_or_default()
    }

    fn id(&self) -> &str {
        self.id.as_deref().unwrap_or_default()
    }

    // What the user sees the field called; the fixed id when the title is
    // blank, which still names it.
    fn label(&self) -> String {
        non_empty(self.title.clone())
            .or_else(|| self.id.clone())
            .unwrap_or_default()
    }

    // A kind that is a secret wherever it sits: a private key or a card
    // number on a note is still one, and lands concealed.
    fn is_secret(&self) -> bool {
        matches!(
            self.kind(),
            "concealed" | "totp" | "sshKey" | "creditCardNumber"
        )
    }

    fn is_file(&self) -> bool {
        self.kind() == "file"
    }

    /// The value as text, read by its kind. `None` for one that has no text
    /// (a file, a reference to another item) or is blank.
    fn text(&self) -> Option<String> {
        let (kind, payload) = self.typed()?;
        let text = match kind {
            "date" => iso_date(payload.as_i64()?)?,
            "monthYear" => {
                let (year, month) = month_year(payload)?;
                format!("{year}-{month}")
            }
            "address" => address(payload)?,
            "email" => match payload {
                Value::Object(email) => email.get("email_address")?.as_str()?.to_owned(),
                other => plain(other)?,
            },
            "sshKey" => ssh_private_key(payload)?,
            "file" | "reference" => return None,
            _ => plain(payload)?,
        };
        non_empty(Some(text))
    }

    fn into_extra(self) -> Option<ExtraField> {
        Some(ExtraField {
            secret: self.is_secret(),
            value: self.text()?,
            label: self.label(),
        })
    }

    /// The provider 1Password notes beside an address (`gmail`, say), which
    /// the email slot has no room for: a labelled extra of its own.
    fn email_provider(&self) -> Option<ExtraField> {
        let (kind, payload) = self.typed()?;
        if kind != "email" {
            return None;
        }
        let provider = non_empty(payload.get("provider")?.as_str().map(str::to_owned))?;
        Some(ExtraField {
            label: format!("{} provider", self.label()),
            value: provider,
            secret: false,
        })
    }
}

/// The key itself. 1Password writes it twice, as `privateKey` and again in
/// the `metadata` beside it, in PKCS#8 both times in the exports seen so far.
/// An OpenSSH block is taken from either place when one is there, as it is
/// already the form the app holds; otherwise the PKCS#8 one, which
/// [`openssh_key`] converts once the item is an SSH entry.
fn ssh_private_key(payload: &Value) -> Option<String> {
    let block = |v: Option<&Value>| non_empty(v?.as_str().map(str::to_owned));
    let top = block(payload.get("privateKey"));
    let meta = block(payload.get("metadata").and_then(|m| m.get("privateKey")));
    let openssh = |key: &Option<String>| key.as_deref().is_some_and(crate::ssh::is_openssh);
    if openssh(&top) {
        top
    } else if openssh(&meta) {
        meta
    } else {
        top.or(meta)
    }
}

/// An SSH entry's key in the form the kind holds: a PKCS#8 block is
/// converted to OpenSSH, and the public line and fingerprint 1Password did
/// not supply come from the conversion. An RSA key stays as it came, which
/// OpenSSH reads; `Err` says why any other key stays PKCS#8.
fn openssh_key(entry: &mut ImportedEntry) -> Result<(), String> {
    let Some(key) = entry.ssh_private_key.as_deref() else {
        return Ok(());
    };
    if !crate::ssh::is_pkcs8(key) {
        return Ok(());
    }
    let Some(converted) = crate::ssh::from_pkcs8(key)? else {
        return Ok(());
    };
    entry.ssh_private_key = Some(converted.private_key);
    entry.ssh_public_key.get_or_insert(converted.public_key);
    entry.ssh_fingerprint.get_or_insert(converted.fingerprint);
    Ok(())
}

/// A saved form checkbox as 1Password records it: a tick, or nothing.
fn checked(value: Option<&str>) -> bool {
    matches!(
        value.unwrap_or_default().trim().to_lowercase().as_str(),
        "✓" | "✔" | "true" | "1" | "on" | "yes" | "checked"
    )
}

fn plain(payload: &Value) -> Option<String> {
    match payload {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A 1Password date is an epoch second (negative before 1970, as a birth date
/// often is), to the `YYYY-MM-DD` the app's date slots hold.
fn iso_date(secs: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(secs, 0).map(|t| t.format("%Y-%m-%d").to_string())
}

/// A `monthYear` is the integer `YYYYMM`, as a card's expiry is written.
fn month_year(payload: &Value) -> Option<(String, String)> {
    let n = match payload {
        Value::Number(n) => n.as_u64()?,
        Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    let (year, month) = (n / 100, n % 100);
    ((1000..10000).contains(&year) && (1..=12).contains(&month))
        .then(|| (year.to_string(), format!("{month:02}")))
}

/// An address is an object of parts; the ones that are filled in, on one line.
fn address(payload: &Value) -> Option<String> {
    let parts: Vec<&str> = ["street", "city", "state", "zip", "country"]
        .iter()
        .filter_map(|key| payload.get(key)?.as_str())
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

// 1Password lists previous passwords oldest first; the vault keeps them newest
// first and no more than its cap, so a second import of the same file matches
// the rows already there (the duplicate check compares history too).
fn password_history(history: Vec<History>) -> Vec<PasswordHistoryItem> {
    let mut items: Vec<(u64, PasswordHistoryItem)> = history
        .into_iter()
        .filter_map(|h| {
            let at = h.time.unwrap_or_default();
            Some((
                at,
                PasswordHistoryItem {
                    password: non_empty(h.value)?,
                    replaced_at: rfc3339(at).unwrap_or_default(),
                },
            ))
        })
        .collect();
    items.sort_by_key(|item| std::cmp::Reverse(item.0));
    items
        .into_iter()
        .map(|(_, item)| item)
        .take(PASSWORD_HISTORY_CAP)
        .collect()
}

/// `export.data` out of the archive, or why it could not be read. The reader
/// is whatever holds the archive — the bytes already in memory, or the file
/// on disk, whose attachments are then never read at all.
pub fn unpack<R: Read + Seek>(archive: R) -> Result<Vec<u8>, String> {
    let mut archive = zip::ZipArchive::new(archive).map_err(|e| e.to_string())?;
    let file = archive.by_name("export.data").map_err(|e| match e {
        zip::result::ZipError::FileNotFound => "no export.data in the archive".to_owned(),
        // A codec this build does not read, say — not a missing file.
        other => other.to_string(),
    })?;
    let mut data = Vec::new();
    file.take(MAX_DATA_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_DATA_BYTES {
        return Err("export.data is too large".to_owned());
    }
    Ok(data)
}

impl Importer for Onepassword {
    fn parse(&self, bytes: &[u8]) -> ImportResult {
        if is_zip(bytes) || is_json(bytes) {
            Onepux.parse(bytes)
        } else {
            super::csv::OnepasswordCsv.parse(bytes)
        }
    }
}

impl Importer for Onepux {
    fn parse(&self, bytes: &[u8]) -> ImportResult {
        let mut result = ImportResult::default();
        let data = if is_zip(bytes) {
            match unpack(Cursor::new(bytes)) {
                Ok(data) => data,
                Err(e) => {
                    result.push_err(0, format!("not a 1Password export: {e}"));
                    return result;
                }
            }
        } else {
            bytes.to_vec()
        };
        let export: Export = match serde_json::from_slice(&data) {
            Ok(e) => e,
            Err(e) => {
                result.push_err(0, format!("invalid 1Password export: {e}"));
                return result;
            }
        };

        // A vault's name only tells items apart when there is more than one;
        // "Personal" on every entry of a one-vault export says nothing.
        let vaults = export
            .accounts
            .iter()
            .map(|a| a.vaults.len())
            .sum::<usize>();
        let name_vaults = vaults > 1;

        let mut row = 0;
        for vault in export.accounts.into_iter().flat_map(|a| a.vaults) {
            let vault_name = non_empty(vault.attrs.name).filter(|_| name_vaults);
            for item in vault.items {
                row += 1;
                // What is in the trash was deleted by the user; it is not
                // brought back by an import, and it is not an error either.
                // An archived item is a live record kept out of the main
                // list, whereas the app's Archive holds deleted entries, so
                // it comes in as an ordinary entry rather than a tombstone.
                if item
                    .state
                    .as_deref()
                    .is_some_and(|s| s.eq_ignore_ascii_case("trashed"))
                {
                    continue;
                }
                let category = item.category.unwrap_or_default();
                let mut tags = item.overview.tags;
                tags.extend(vault_name.clone());
                let urls = item.overview.urls;
                let url = non_empty(item.overview.url)
                    .or_else(|| urls.iter().find_map(|u| non_empty(u.url.clone())));
                // Every site the item names, for the kinds that have no slot
                // for one and for the sites beyond the one that fills it.
                let websites = websites(url.as_deref(), urls);
                let details = item.details;
                let attachments = usize::from(details.document.is_some())
                    + details
                        .sections
                        .iter()
                        .flat_map(|s| &s.fields)
                        .filter(|f| f.is_file())
                        .count();
                // A link to another item cannot be followed here; like a
                // file, it is left behind and said so.
                let links = details
                    .sections
                    .iter()
                    .flat_map(|s| &s.fields)
                    .filter(|f| f.kind() == "reference")
                    .count();
                let providers: Vec<ExtraField> = details
                    .sections
                    .iter()
                    .flat_map(|s| &s.fields)
                    .filter_map(Field::email_provider)
                    .collect();
                let fields: Vec<Field> = details
                    .sections
                    .into_iter()
                    .flat_map(|s| s.fields)
                    .filter(|f| !f.is_file())
                    .collect();
                let base = ImportedEntry {
                    title: item.overview.title.unwrap_or_default(),
                    notes: non_empty(details.notes),
                    tags,
                    favorite: item.fav_index.is_some_and(|f| f > 0),
                    created_at: item.created_at.and_then(rfc3339),
                    updated_at: item.updated_at.and_then(rfc3339),
                    ..Default::default()
                };
                let login_fields = details.login_fields;
                let has_password = non_empty(details.password.clone()).is_some()
                    || login_fields.iter().any(|f| {
                        f.designation.as_deref() == Some("password")
                            && non_empty(f.value.clone()).is_some()
                    })
                    || fields.iter().any(|f| is_password(f) && f.text().is_some());
                let site = Site {
                    url,
                    login_fields,
                    password: details.password,
                    history: details.password_history,
                };
                let mut entry = match category.as_str() {
                    "002" => card(base, fields),
                    "003" | "006" => note(base, fields),
                    "103" => identity(base, fields, "driver_license"),
                    "106" => identity(base, fields, "passport"),
                    "112" => api_key(base, fields, site.url),
                    "114" => ssh(base, fields),
                    "001" | "005" => login(base, fields, site),
                    // Identity (a person and their addresses), Bank Account,
                    // Server, Email Account and the rest have no shape of
                    // their own here: one that holds a password is a login to
                    // the site it names, and anything else is a note that
                    // keeps every field it had.
                    _ if has_password => login(base, fields, site),
                    _ => note(base, fields),
                };
                let kept = entry.url.clone();
                entry.extra.extend(
                    websites
                        .into_iter()
                        .filter(|site| kept.as_deref() != Some(site.value.as_str())),
                );
                entry.extra.extend(providers);
                if entry.title.is_empty() {
                    result.push_err(row, "item has no title");
                    continue;
                }
                // OpenSSH reads a PKCS#8 key for RSA and ECDSA but not for
                // Ed25519, the kind most are, so a key is converted here; one
                // that cannot be is kept as it came, and the row says so.
                let kept_pkcs8 = match entry.kind {
                    EntryKind::Ssh => openssh_key(&mut entry).err(),
                    _ => None,
                };
                result.entries.push(entry);
                if let Some(why) = kept_pkcs8 {
                    result.push_warn(row, format!("private key kept in PKCS#8 form: {why}"));
                }
                if attachments > 0 {
                    result.push_warn(
                        row,
                        format!("{attachments} attachment(s) not imported; keep the export"),
                    );
                }
                if links > 0 {
                    result.push_warn(
                        row,
                        format!("{links} link(s) to other items not imported; keep the export"),
                    );
                }
            }
        }
        result
    }
}

/// The item's sites as labelled extras, each once: the listed ones under
/// their own labels, then the primary if the list did not already name it.
fn websites(primary: Option<&str>, urls: Vec<Url>) -> Vec<ExtraField> {
    let mut out: Vec<ExtraField> = Vec::new();
    let listed = urls
        .into_iter()
        .filter_map(|u| Some((non_empty(u.label), non_empty(u.url)?)));
    let primary = primary.map(|u| (None, u.to_owned()));
    for (label, value) in listed.chain(primary) {
        if out.iter().any(|site| site.value == value) {
            continue;
        }
        out.push(ExtraField {
            label: label.unwrap_or_else(|| "website".to_owned()),
            value,
            secret: false,
        });
    }
    out
}

/// What a login signs in to and with, from outside the sections.
struct Site {
    url: Option<String>,
    login_fields: Vec<LoginField>,
    password: Option<String>,
    history: Vec<History>,
}

// A concealed field that is the account's password — `password` on a Server
// or Database, `wireless_password` on a router.
fn is_password(field: &Field) -> bool {
    field.kind() == "concealed" && field.id().contains("password")
}

// The designated form fields sign in; a Password item keeps its secret on the
// item itself. The first field of each kind fills the slot; a second one (a
// recovery address) stays an extra, as do the item's other sites (see
// [`websites`]). A field 1Password left blank is not carried anywhere.
fn login(mut entry: ImportedEntry, fields: Vec<Field>, site: Site) -> ImportedEntry {
    entry.kind = EntryKind::Login;
    entry.url = site.url.clone();
    for form in site.login_fields {
        // A saved checkbox is kept as the yes or no it recorded — unticked as
        // much as ticked, which is what an unnamed one has no way to say.
        let checkbox = form.field_type.as_deref() == Some("C");
        let value = if checkbox {
            if non_empty(form.name.clone()).is_none() {
                continue;
            }
            Some(checked(form.value.as_deref()).to_string())
        } else {
            non_empty(form.value)
        };
        let Some(value) = value else {
            continue;
        };
        let slot = match form.designation.as_deref() {
            Some("username") => &mut entry.username,
            Some("password") => &mut entry.password,
            _ => {
                entry.extra.push(ExtraField {
                    secret: form.field_type.as_deref() == Some("P"),
                    label: form.name.unwrap_or_default(),
                    value,
                });
                continue;
            }
        };
        if slot.is_none() {
            *slot = Some(value);
        } else {
            entry.extra.push(ExtraField {
                secret: form.designation.as_deref() == Some("password"),
                label: form.name.unwrap_or_default(),
                value,
            });
        }
    }
    if entry.password.is_none() {
        entry.password = non_empty(site.password);
    }
    for field in fields {
        let Some(value) = field.text() else {
            continue;
        };
        let slot = match (field.id(), field.kind()) {
            ("username", _) => &mut entry.username,
            (_, "totp") => &mut entry.otp,
            (_, "email") => &mut entry.email,
            (_, "url") => &mut entry.url,
            _ if is_password(&field) => &mut entry.password,
            _ => {
                entry.extra.extend(field.into_extra());
                continue;
            }
        };
        if slot.is_none() {
            *slot = Some(value);
        } else {
            entry.extra.extend(field.into_extra());
        }
    }
    entry.password_history = password_history(site.history);
    if entry.title.is_empty() {
        entry.title = entry
            .url
            .clone()
            .or_else(|| entry.username.clone())
            .unwrap_or_default();
    }
    entry
}

fn card(mut entry: ImportedEntry, fields: Vec<Field>) -> ImportedEntry {
    entry.kind = EntryKind::Card;
    for field in fields {
        let Some(value) = field.text() else {
            continue;
        };
        let slot = match field.id() {
            "cardholder" => &mut entry.cardholder,
            "ccnum" => &mut entry.card_number,
            "cvv" => &mut entry.card_cvc,
            "pin" => &mut entry.card_pin,
            "expiry" if entry.card_month.is_none() => {
                match field.typed().and_then(|(_, payload)| month_year(payload)) {
                    Some((year, month)) => {
                        entry.card_month = Some(month);
                        entry.card_year = Some(year);
                    }
                    None => entry.extra.extend(field.into_extra()),
                }
                continue;
            }
            _ => {
                entry.extra.extend(field.into_extra());
                continue;
            }
        };
        if slot.is_none() {
            *slot = Some(value);
        } else {
            entry.extra.extend(field.into_extra());
        }
    }
    entry
}

// A passport's and a driving licence's fields share their ids where they mean
// the same thing. A date slot only holds a day (`YYYY-MM-DD`): a licence's
// expiry is a month, so it stays the labelled extra it was, where it still
// reads, rather than a guess at a day.
fn identity(mut entry: ImportedEntry, fields: Vec<Field>, doc_type: &str) -> ImportedEntry {
    entry.kind = EntryKind::Identity;
    entry.doc_type = Some(doc_type.to_owned());
    for field in fields {
        let Some(value) = field.text() else {
            continue;
        };
        let day = field.kind() == "date";
        let slot = match field.id() {
            "number" => &mut entry.doc_number,
            "fullname" => &mut entry.holder_name,
            "sex" => &mut entry.doc_sex,
            "nationality" => &mut entry.doc_nationality,
            "country" | "issuing_country" => &mut entry.doc_country,
            "issuing_authority" => &mut entry.doc_authority,
            "birthdate" if day => &mut entry.doc_birth_date,
            "issue_date" if day => &mut entry.doc_issue_date,
            "expiry_date" if day => &mut entry.doc_expiry_date,
            _ => {
                entry.extra.extend(field.into_extra());
                continue;
            }
        };
        if slot.is_none() {
            *slot = Some(value);
        } else {
            entry.extra.extend(field.into_extra());
        }
    }
    entry
}

// An API Credential: the secret, the host it is sent to, and when it lapses.
// The username 1Password keeps beside it has no slot and rides as an extra.
fn api_key(mut entry: ImportedEntry, fields: Vec<Field>, url: Option<String>) -> ImportedEntry {
    entry.kind = EntryKind::ApiKey;
    entry.url = url;
    for field in fields {
        let Some(value) = field.text() else {
            continue;
        };
        let slot = match field.id() {
            "credential" => &mut entry.api_key,
            "hostname" => &mut entry.url,
            "expires" if field.kind() == "date" => &mut entry.api_expires,
            _ => {
                entry.extra.extend(field.into_extra());
                continue;
            }
        };
        if slot.is_none() {
            *slot = Some(value);
        } else {
            entry.extra.extend(field.into_extra());
        }
    }
    entry
}

// An SSH Key item is one `sshKey` field: the private key, with the public
// line, fingerprint and key type 1Password derived from it alongside.
fn ssh(mut entry: ImportedEntry, fields: Vec<Field>) -> ImportedEntry {
    entry.kind = EntryKind::Ssh;
    for field in fields {
        if entry.ssh_private_key.is_none() && field.kind() == "sshKey" {
            let Some((_, payload)) = field.typed() else {
                continue;
            };
            let text = |key: &str| {
                non_empty(
                    payload
                        .get("metadata")
                        .and_then(|m| m.get(key))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                )
            };
            entry.ssh_private_key = field.text();
            entry.ssh_public_key = text("publicKey");
            entry.ssh_fingerprint = text("fingerprint");
            if let Some(key_type) = text("keyType") {
                entry.extra.push(ExtraField {
                    label: "Key type".to_owned(),
                    value: key_type,
                    secret: false,
                });
            }
            continue;
        }
        entry.extra.extend(field.into_extra());
    }
    entry
}

// Every field is kept: a note's fields are the user's own, and so are those of
// a category the app has no kind for.
fn note(mut entry: ImportedEntry, fields: Vec<Field>) -> ImportedEntry {
    entry.kind = EntryKind::Note;
    entry.extra = fields.into_iter().filter_map(Field::into_extra).collect();
    entry
}
