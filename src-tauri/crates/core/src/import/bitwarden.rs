//! Bitwarden unencrypted JSON export (`{"items":[...]}`). Item `type` is 1=login,
//! 2=secure note, 3=card, 4=identity, 5=SSH key. We map 1–5; anything else is a
//! row error.

use serde::Deserialize;

use super::export::PASSPHRASE_LABEL;
use super::{non_empty, EntryKind, ImportResult, ImportedEntry, ImportedPasskey, Importer};

pub struct Bitwarden;

/// The only passkey shape we support, on the way in and on the way out.
pub const KEY_TYPE: &str = "public-key";
pub const KEY_ALGORITHM: &str = "ECDSA";
pub const KEY_CURVE: &str = "P-256";

#[derive(Deserialize)]
struct Export {
    #[serde(default)]
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    // Lenient: an item whose `type` is missing, unknown or written as a string
    // is one bad row, and a strict member would fail the whole document.
    #[serde(default, rename = "type", deserialize_with = "super::lenient_u64")]
    kind: Option<u64>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    login: Option<Login>,
    #[serde(default)]
    card: Option<Card>,
    #[serde(default)]
    identity: Option<Identity>,
    #[serde(default, rename = "sshKey")]
    ssh_key: Option<SshKey>,
    #[serde(default)]
    fields: Vec<Field>,
    /// The star and the two dates Bitwarden keeps on every item, whatever its
    /// type — read here so a re-import is a copy rather than a fresh entry.
    #[serde(default)]
    favorite: bool,
    #[serde(default, rename = "creationDate")]
    creation_date: Option<String>,
    #[serde(default, rename = "revisionDate")]
    revision_date: Option<String>,
}

/// Bitwarden's SSH key item: the three members are exactly the three the app
/// stores. It has no passphrase member; ours travels as a hidden custom field.
#[derive(Default, Deserialize)]
struct SshKey {
    #[serde(default, rename = "privateKey")]
    private_key: Option<String>,
    #[serde(default, rename = "publicKey")]
    public_key: Option<String>,
    #[serde(default, rename = "keyFingerprint")]
    key_fingerprint: Option<String>,
}

/// A Bitwarden custom field. `type` is 0 = text, 1 = hidden, 2 = boolean,
/// 3 = linked; the first two are values a person typed and become extra fields,
/// the other two are Bitwarden's own machinery and are dropped. An absent type
/// reads as text, which is what an unset one means there.
#[derive(Deserialize)]
struct Field {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default, rename = "type", deserialize_with = "super::lenient_u8")]
    kind: Option<u8>,
}

pub const FIELD_TEXT: u8 = 0;
pub const FIELD_HIDDEN: u8 = 1;

// A row with neither a name nor a value says nothing, so it is not carried.
fn extras(fields: Vec<Field>) -> Vec<(String, String)> {
    fields
        .into_iter()
        .filter(|f| matches!(f.kind.unwrap_or(FIELD_TEXT), FIELD_TEXT | FIELD_HIDDEN))
        .map(|f| (f.name.unwrap_or_default(), f.value.unwrap_or_default()))
        .filter(|(name, value)| !name.is_empty() || !value.is_empty())
        .collect()
}

#[derive(Deserialize)]
struct Login {
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    totp: Option<String>,
    #[serde(default)]
    uris: Vec<Uri>,
    #[serde(default, rename = "fido2Credentials")]
    fido2_credentials: Vec<Fido2Credential>,
}

// Every value in a Bitwarden fido2Credentials element is a JSON string,
// `counter` included — but a number is accepted too, so one odd producer
// cannot fail the whole file.
#[derive(Deserialize)]
struct Fido2Credential {
    #[serde(default, rename = "credentialId")]
    credential_id: Option<String>,
    #[serde(default, rename = "keyAlgorithm")]
    key_algorithm: Option<String>,
    #[serde(default, rename = "keyCurve")]
    key_curve: Option<String>,
    #[serde(default, rename = "keyValue")]
    key_value: Option<String>,
    #[serde(default, rename = "rpId")]
    rp_id: Option<String>,
    #[serde(default, rename = "rpName")]
    rp_name: Option<String>,
    #[serde(default, rename = "userHandle")]
    user_handle: Option<String>,
    #[serde(default, rename = "userName")]
    user_name: Option<String>,
    #[serde(default, rename = "userDisplayName")]
    user_display_name: Option<String>,
    #[serde(default)]
    counter: Option<serde_json::Value>,
    #[serde(default, rename = "creationDate")]
    creation_date: Option<String>,
}

impl Fido2Credential {
    // Absent algorithm/curve means "the default", which is the only one we
    // support; a stated one that differs is a credential we cannot use. Nor is
    // one missing its id, relying party or key — there is nothing to sign with.
    fn is_usable(&self) -> bool {
        let ok = |v: &Option<String>, want: &str| match v.as_deref() {
            Some(v) => v == want,
            None => true,
        };
        let present = |v: &Option<String>| v.as_deref().is_some_and(|s| !s.is_empty());
        ok(&self.key_algorithm, KEY_ALGORITHM)
            && ok(&self.key_curve, KEY_CURVE)
            && present(&self.credential_id)
            && present(&self.rp_id)
            && present(&self.key_value)
    }

    // A missing or non-numeric counter starts the credential at zero rather
    // than failing the import.
    fn counter(&self) -> u32 {
        match &self.counter {
            Some(serde_json::Value::String(s)) => s.parse().unwrap_or(0),
            Some(serde_json::Value::Number(n)) => {
                n.as_u64().and_then(|n| n.try_into().ok()).unwrap_or(0)
            }
            _ => 0,
        }
    }
}

impl From<Fido2Credential> for ImportedPasskey {
    fn from(c: Fido2Credential) -> Self {
        let counter = c.counter();
        ImportedPasskey {
            credential_id: c.credential_id.unwrap_or_default(),
            rp_id: c.rp_id.unwrap_or_default(),
            rp_name: non_empty(c.rp_name),
            user_handle: c.user_handle.unwrap_or_default(),
            user_name: c.user_name.unwrap_or_default(),
            user_display_name: c.user_display_name.unwrap_or_default(),
            private_key: c.key_value.unwrap_or_default(),
            counter,
            created_at: non_empty(c.creation_date),
        }
    }
}

#[derive(Deserialize)]
struct Uri {
    #[serde(default)]
    uri: Option<String>,
}

#[derive(Deserialize)]
struct Card {
    #[serde(default, rename = "cardholderName")]
    cardholder_name: Option<String>,
    #[serde(default)]
    number: Option<String>,
    #[serde(default, rename = "expMonth")]
    exp_month: Option<String>,
    #[serde(default, rename = "expYear")]
    exp_year: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

// Bitwarden's identity item is a whole address book; only the members the app
// has somewhere to put are read — the rest of our own document travels as
// labelled custom fields. Two document numbers, so which one is set is also
// what says whether this is a licence or a passport.
#[derive(Default, Deserialize)]
struct Identity {
    #[serde(default, rename = "firstName")]
    first_name: Option<String>,
    #[serde(default, rename = "middleName")]
    middle_name: Option<String>,
    #[serde(default, rename = "lastName")]
    last_name: Option<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default, rename = "passportNumber")]
    passport_number: Option<String>,
    #[serde(default, rename = "licenseNumber")]
    license_number: Option<String>,
    /// Bitwarden's one national-identifier member; the app's personal number.
    #[serde(default)]
    ssn: Option<String>,
}

impl Identity {
    // The app keeps one full name, so the three parts are joined back up in
    // reading order — the inverse of the split the exporter does.
    fn full_name(&self) -> Option<String> {
        let parts: Vec<&str> = [&self.first_name, &self.middle_name, &self.last_name]
            .iter()
            .filter_map(|p| p.as_deref())
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect();
        (!parts.is_empty()).then(|| parts.join(" "))
    }
}

impl Importer for Bitwarden {
    fn parse(&self, bytes: &[u8]) -> ImportResult {
        let mut result = ImportResult::default();
        let export: Export = match serde_json::from_slice(bytes) {
            Ok(e) => e,
            Err(e) => {
                result.push_err(0, format!("invalid Bitwarden JSON: {e}"));
                return result;
            }
        };

        for (i, item) in export.items.into_iter().enumerate() {
            let row = i + 1;
            let title = item.name.clone().unwrap_or_default();
            // Custom fields are not a kind's business: every item type may
            // carry them, so they are read once, before the kind is decided.
            let mut extra = extras(item.fields);
            // What belongs to no kind either: the star and the dates Bitwarden
            // has members for, and the values it has none for, which are taken
            // back out of the custom fields they rode in so they do not show up
            // twice. Each arm below starts from this rather than the default.
            let mut base = ImportedEntry {
                favorite: item.favorite,
                created_at: non_empty(item.creation_date),
                updated_at: non_empty(item.revision_date),
                ..Default::default()
            };
            super::export::set_labelled(&mut base, |label| take_labelled(&mut extra, label));
            match item.kind {
                Some(1) => {
                    let login = item.login.unwrap_or(Login {
                        username: None,
                        password: None,
                        totp: None,
                        uris: vec![],
                        fido2_credentials: vec![],
                    });
                    // An unusable credential is dropped on its own — the login
                    // and its other passkeys still import.
                    let mut passkeys = Vec::new();
                    for cred in login.fido2_credentials {
                        if cred.is_usable() {
                            passkeys.push(cred.into());
                        } else {
                            result.push_err(row, "unsupported or incomplete passkey");
                        }
                    }
                    result.entries.push(ImportedEntry {
                        kind: EntryKind::Login,
                        title,
                        username: non_empty(login.username),
                        password: non_empty(login.password),
                        url: login.uris.into_iter().find_map(|u| non_empty(u.uri)),
                        notes: non_empty(item.notes),
                        otp: non_empty(login.totp),
                        passkeys,
                        extra,
                        ..base
                    });
                }
                Some(2) => result.entries.push(ImportedEntry {
                    kind: EntryKind::Note,
                    title,
                    notes: non_empty(item.notes),
                    extra,
                    ..base
                }),
                Some(3) => {
                    let card = item.card.unwrap_or(Card {
                        cardholder_name: None,
                        number: None,
                        exp_month: None,
                        exp_year: None,
                        code: None,
                    });
                    result.entries.push(ImportedEntry {
                        kind: EntryKind::Card,
                        title,
                        notes: non_empty(item.notes),
                        card_number: non_empty(card.number),
                        card_month: non_empty(card.exp_month),
                        card_year: non_empty(card.exp_year),
                        card_cvc: non_empty(card.code),
                        cardholder: non_empty(card.cardholder_name),
                        extra,
                        ..base
                    });
                }
                Some(4) => {
                    let identity = item.identity.unwrap_or_default();
                    // A licence number says "driver_license"; anything else —
                    // including an item with neither number — is a passport,
                    // which is what Bitwarden's own field is named after.
                    let licence = non_empty(identity.license_number.clone());
                    let doc_type = if licence.is_some() {
                        "driver_license"
                    } else {
                        "passport"
                    };
                    result.entries.push(ImportedEntry {
                        kind: EntryKind::Identity,
                        title,
                        notes: non_empty(item.notes),
                        doc_type: Some(doc_type.to_owned()),
                        doc_number: licence.or_else(|| non_empty(identity.passport_number.clone())),
                        doc_country: non_empty(identity.country.clone()),
                        holder_name: identity.full_name(),
                        // Bitwarden's own member wins; a file that carries the
                        // number as a labelled field instead still keeps it.
                        doc_personal_number: non_empty(identity.ssn.clone())
                            .or_else(|| base.doc_personal_number.clone()),
                        extra,
                        ..base
                    });
                }
                Some(5) => {
                    let key = item.ssh_key.unwrap_or_default();
                    // The passphrase is the one field that is ours, not the
                    // user's: it is taken back out of the extras it rode in.
                    let (passphrase, extra): (Vec<_>, Vec<_>) = extra
                        .into_iter()
                        .partition(|(label, _)| label.eq_ignore_ascii_case(PASSPHRASE_LABEL));
                    result.entries.push(ImportedEntry {
                        kind: EntryKind::Ssh,
                        title,
                        notes: non_empty(item.notes),
                        ssh_private_key: non_empty(key.private_key),
                        ssh_public_key: non_empty(key.public_key),
                        ssh_fingerprint: non_empty(key.key_fingerprint),
                        ssh_passphrase: passphrase
                            .into_iter()
                            .next()
                            .and_then(|(_, v)| non_empty(Some(v))),
                        extra,
                        ..base
                    });
                }
                None => result.push_err(row, "item has no type"),
                Some(other) => result.push_err(row, format!("unsupported item type {other}")),
            }
        }
        result
    }
}

// Remove the custom field labelled `label` (case-insensitively) and hand back
// its value: a field of ours is not the user's, so it leaves the extras when it
// is claimed — the same move the SSH passphrase makes in its own arm.
fn take_labelled(extra: &mut Vec<(String, String)>, label: &str) -> Option<String> {
    let at = extra
        .iter()
        .position(|(l, _)| l.eq_ignore_ascii_case(label))?;
    non_empty(Some(extra.remove(at).1))
}
