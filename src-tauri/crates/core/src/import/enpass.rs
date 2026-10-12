//! Enpass 6 JSON export (`{"folders":[...],"items":[...]}`). An item is a
//! `category` plus a flat list of typed `fields`; the app's slots are filled by
//! field *type* (`username`, `password`, `ccNumber`, …), never by label, which
//! Enpass writes in the user's language. Whatever has no slot travels as a
//! labelled extra, concealed when Enpass marked it sensitive. Folders are
//! nested by `parent_uuid` and become one path tag per folder the item sits in.
//!
//! The one exception is an ID document (`license.driving`, `travel.passport`):
//! its fields are all plain `text`/`date`/`numeric`, so the template says what
//! the item is and the English labels say which field is which. A localized
//! export still comes back as the right document kind, with its fields as
//! extras rather than in the slots.

use std::collections::HashMap;

use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::{non_empty, rfc3339, EntryKind, ImportResult, ImportedEntry, Importer};
use crate::models::{ExtraField, PasswordHistoryItem, PASSWORD_HISTORY_CAP};

pub struct Enpass;

#[derive(Deserialize)]
struct Export {
    #[serde(default)]
    folders: Vec<Folder>,
    #[serde(default)]
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Folder {
    #[serde(default)]
    uuid: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    parent_uuid: Option<String>,
}

#[derive(Deserialize)]
struct Item {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    template_type: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    fields: Vec<Field>,
    /// The uuids of the folders the item is filed in.
    #[serde(default)]
    folders: Vec<String>,
    // Flags are 0/1 integers; timestamps are epoch seconds. All lenient: one
    // odd value must not cost the whole file.
    #[serde(default, deserialize_with = "super::lenient_u64")]
    favorite: Option<u64>,
    #[serde(default, deserialize_with = "super::lenient_u64")]
    trashed: Option<u64>,
    #[serde(default, rename = "createdAt", deserialize_with = "super::lenient_u64")]
    created_at: Option<u64>,
    #[serde(default, deserialize_with = "super::lenient_u64")]
    updated_at: Option<u64>,
}

#[derive(Deserialize)]
struct Field {
    #[serde(default)]
    label: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    value: Option<String>,
    #[serde(default, deserialize_with = "super::lenient_u64")]
    sensitive: Option<u64>,
    #[serde(default, deserialize_with = "super::lenient_u64")]
    deleted: Option<u64>,
    #[serde(default, deserialize_with = "super::lenient_u64")]
    value_updated_at: Option<u64>,
    /// Previous values of a password field, with when each was replaced.
    #[serde(default)]
    history: Vec<History>,
}

#[derive(Deserialize)]
struct History {
    #[serde(default, deserialize_with = "lenient_string")]
    value: Option<String>,
    #[serde(default, deserialize_with = "super::lenient_u64")]
    updated_at: Option<u64>,
}

/// A field value is a string, but a numeric field may be written as a number;
/// anything else reads as absent rather than failing the document.
fn lenient_string<'de, D>(d: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Value::deserialize(d)? {
        Value::String(s) => Some(s),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    })
}

const ON: Option<u64> = Some(1);

impl Field {
    // Enpass's own machinery, not a value: a section heading, or an app
    // association (`.Android#`). Neither is carried.
    fn is_structural(&self) -> bool {
        match self.kind.as_deref() {
            Some("section") => true,
            Some(kind) => kind.starts_with('.'),
            None => false,
        }
    }

    // A value Enpass conceals, or one that is a secret by its type whether or
    // not the flag was set on it.
    fn is_secret(&self) -> bool {
        self.sensitive == ON
            || matches!(
                self.kind.as_deref(),
                Some("password" | "pin" | "totp" | "ccCvc" | "ccPin" | "ccTxnpassword")
            )
    }

    fn into_extra(self) -> ExtraField {
        ExtraField {
            secret: self.is_secret(),
            label: self.label.unwrap_or_default(),
            value: self.value.unwrap_or_default(),
        }
    }
}

// Enpass lists previous passwords oldest first; the vault keeps them newest
// first and no more than its cap, so a second import of the same file matches
// the rows already there (the duplicate check compares history too).
fn password_history(history: Vec<History>) -> Vec<PasswordHistoryItem> {
    let mut items: Vec<(u64, PasswordHistoryItem)> = history
        .into_iter()
        .filter_map(|h| {
            let at = h.updated_at.unwrap_or_default();
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

// `MM/YY` or `MM/YYYY`, which is how Enpass writes a card's expiry; a two-digit
// year is this century's. Anything else is not an expiry we can split.
fn card_expiry(value: &str) -> Option<(String, String)> {
    let (month, year) = value.trim().split_once('/')?;
    let month = month.trim();
    let year = year.trim();
    if month.is_empty() || !month.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !year.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let year = match year.len() {
        2 => format!("20{year}"),
        4 => year.to_owned(),
        _ => return None,
    };
    Some((month.to_owned(), year))
}

/// Folders by uuid, so an item's folder list resolves to paths.
struct Folders(HashMap<String, Folder>);

impl Folders {
    fn new(folders: Vec<Folder>) -> Self {
        Folders(
            folders
                .into_iter()
                .filter_map(|f| Some((non_empty(f.uuid.clone())?, f)))
                .collect(),
        )
    }

    // `Parent/Child` for a nested folder. A folder the file does not list, or
    // a cycle in parents, ends the path where it is rather than looping.
    fn path(&self, uuid: &str) -> Option<String> {
        let mut parts = Vec::new();
        let mut next = Some(uuid.to_owned());
        while let Some(id) = next.take() {
            let folder = self.0.get(&id)?;
            parts.push(folder.title.clone().unwrap_or_default());
            if parts.len() > 32 {
                break;
            }
            next = non_empty(folder.parent_uuid.clone()).filter(|p| p != &id);
        }
        parts.reverse();
        let path = parts.join("/");
        (!path.is_empty()).then_some(path)
    }
}

/// What an item is, by its `category` — or, when that is missing, by the
/// template family (`login.default` → `login`).
fn category(item: &Item) -> String {
    item.category
        .clone()
        .or_else(|| {
            item.template_type
                .as_deref()
                .map(|t| t.split('.').next().unwrap_or_default().to_owned())
        })
        .unwrap_or_default()
        .to_lowercase()
}

/// The app's `doc_type` for the templates that are an ID document, by the
/// template's name — the category alone cannot tell a driving licence from a
/// software licence, or a passport from a boarding pass.
fn document_type(template_type: Option<&str>) -> Option<&'static str> {
    match template_type?.trim().to_lowercase().as_str() {
        "license.driving" => Some("driver_license"),
        "travel.passport" => Some("passport"),
        _ => None,
    }
}

impl Importer for Enpass {
    fn parse(&self, bytes: &[u8]) -> ImportResult {
        let mut result = ImportResult::default();
        let export: Export = match serde_json::from_slice(bytes) {
            Ok(e) => e,
            Err(e) => {
                result.push_err(0, format!("invalid Enpass JSON: {e}"));
                return result;
            }
        };
        let folders = Folders::new(export.folders);

        for (i, item) in export.items.into_iter().enumerate() {
            let row = i + 1;
            // What is in the trash was deleted by the user; it is not brought
            // back by an import, and it is not an error either.
            if item.trashed == ON {
                continue;
            }
            let category = category(&item);
            let document = document_type(item.template_type.as_deref());
            let fields: Vec<Field> = item
                .fields
                .into_iter()
                .filter(|f| f.deleted != ON && !f.is_structural())
                .collect();
            let tags = item
                .folders
                .iter()
                .filter_map(|id| folders.path(id))
                .collect();
            let base = ImportedEntry {
                title: item.title.unwrap_or_default(),
                notes: non_empty(item.note),
                tags,
                favorite: item.favorite.is_some_and(|f| f > 0),
                created_at: item.created_at.and_then(rfc3339),
                updated_at: item.updated_at.and_then(rfc3339),
                ..Default::default()
            };
            let has_password = fields.iter().any(|f| {
                f.kind.as_deref() == Some("password") && non_empty(f.value.clone()).is_some()
            });
            let entry = match (category.as_str(), document) {
                ("login" | "password", _) => login(base, fields),
                ("creditcard", _) => card(base, fields),
                ("note", _) => note(base, fields),
                (_, Some(doc_type)) => identity(base, fields, doc_type),
                // Enpass's identity (a person and their addresses), finance,
                // computer and the rest have no shape of their own here: one
                // that holds a password is a login to the site it names, and
                // anything else is a note that keeps every field it had.
                _ if has_password => login(base, fields),
                _ => note(base, fields),
            };
            if entry.title.is_empty() {
                result.push_err(row, "item has no title");
                continue;
            }
            result.entries.push(entry);
        }
        result
    }
}

// The first field of each type fills the slot; a second one of the same type
// (another website, a security answer typed as a password) stays an extra. A
// template lists every field whether or not it was filled in, so a blank one
// is not carried anywhere.
fn login(mut entry: ImportedEntry, fields: Vec<Field>) -> ImportedEntry {
    entry.kind = EntryKind::Login;
    for field in fields {
        let Some(value) = non_empty(field.value.clone()) else {
            continue;
        };
        let slot = match field.kind.as_deref() {
            Some("username") => &mut entry.username,
            Some("email") => &mut entry.email,
            Some("password") => &mut entry.password,
            Some("url") => &mut entry.url,
            Some("totp") => &mut entry.otp,
            _ => {
                entry.extra.push(field.into_extra());
                continue;
            }
        };
        if slot.is_some() {
            entry.extra.push(field.into_extra());
            continue;
        }
        *slot = Some(value);
        if field.kind.as_deref() == Some("password") {
            entry.password_updated_at = field.value_updated_at.and_then(rfc3339);
            entry.password_history = password_history(field.history);
        }
    }
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
        let Some(value) = non_empty(field.value.clone()) else {
            continue;
        };
        let slot = match field.kind.as_deref() {
            Some("ccName") => &mut entry.cardholder,
            Some("ccNumber") => &mut entry.card_number,
            Some("ccCvc") => &mut entry.card_cvc,
            Some("ccPin") => &mut entry.card_pin,
            Some("ccExpiry") if entry.card_month.is_none() => {
                match card_expiry(&value) {
                    Some((month, year)) => {
                        entry.card_month = Some(month);
                        entry.card_year = Some(year);
                    }
                    None => entry.extra.push(field.into_extra()),
                }
                continue;
            }
            _ => {
                entry.extra.push(field.into_extra());
                continue;
            }
        };
        if slot.is_none() {
            *slot = Some(value);
        } else {
            entry.extra.push(field.into_extra());
        }
    }
    entry
}

// An ID document's fields are all generic types, so the slots are filled by
// the template's English labels; the first one wearing a label wins, and
// anything unmatched (class, restrictions, height, an address) is an extra.
// Dates cross as Enpass wrote them: its `01/06/1987` is in the user's own
// pattern, which the app also leaves as typed rather than guessing the order.
fn identity(mut entry: ImportedEntry, fields: Vec<Field>, doc_type: &str) -> ImportedEntry {
    entry.kind = EntryKind::Identity;
    entry.doc_type = Some(doc_type.to_owned());
    for field in fields {
        let Some(value) = non_empty(field.value.clone()) else {
            continue;
        };
        let label = field
            .label
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        let slot = match label.as_str() {
            "number" => &mut entry.doc_number,
            "name" | "full name" => &mut entry.holder_name,
            "gender" | "sex" => &mut entry.doc_sex,
            "birth date" | "date of birth" => &mut entry.doc_birth_date,
            "country" | "issuing country" => &mut entry.doc_country,
            "nationality" => &mut entry.doc_nationality,
            "issued on" | "issue date" => &mut entry.doc_issue_date,
            "expiry date" | "expires on" => &mut entry.doc_expiry_date,
            "authority" | "issuing authority" | "issued by" | "place of issue" => {
                &mut entry.doc_authority
            }
            _ => {
                entry.extra.push(field.into_extra());
                continue;
            }
        };
        if slot.is_some() {
            entry.extra.push(field.into_extra());
            continue;
        }
        *slot = Some(value);
    }
    entry
}

// Every field is kept: a note's fields are the user's own, and so are those of
// a template the app has no kind for.
fn note(mut entry: ImportedEntry, fields: Vec<Field>) -> ImportedEntry {
    entry.kind = EntryKind::Note;
    entry.extra = fields
        .into_iter()
        .filter(|f| non_empty(f.value.clone()).is_some())
        .map(Field::into_extra)
        .collect();
    entry
}
