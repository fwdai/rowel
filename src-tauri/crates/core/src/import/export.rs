//! Serialize normalized entries out to a file. Three formats: Bitwarden-compatible
//! JSON (re-importable here and by Bitwarden), a FIDO Credential Exchange Format
//! document, and a generic CSV. CSV cells are sanitized against spreadsheet
//! formula injection.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Serialize;
use serde_json::{json, Value};

use super::bitwarden::{FIELD_HIDDEN, FIELD_TEXT, KEY_ALGORITHM, KEY_CURVE, KEY_TYPE};
use super::{EntryKind, ImportedEntry, ImportedPasskey};
use crate::app::APP_NAME;
use crate::otp::{self, OtpParams};

/// The exporter's relying-party id in a CXF document: the project's domain.
pub const EXPORTER_RP_ID: &str = "rowel.app";

/// Bitwarden `type` codes. Bitwarden has no item for a `.env` file, so one goes
/// out as a secure note carrying the file text — never as an empty login. Nor
/// for an API key, which goes out as the login it most resembles: the token as
/// the password, the base URL as the site.
fn bw_type(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::Login | EntryKind::ApiKey => 1,
        EntryKind::Note | EntryKind::Env => 2,
        EntryKind::Card => 3,
        EntryKind::Identity => 4,
        EntryKind::Ssh => 5,
    }
}

/// The label under which the passphrase travels where a format has no member
/// for it: a hidden custom field in Bitwarden, a custom-fields credential in
/// CXF. The importers look for the same label, case-insensitively.
pub const PASSPHRASE_LABEL: &str = "Passphrase";
/// CXF has no members for the public line and fingerprint, so they ride in the
/// same custom-fields credential under these labels.
pub const PUBLIC_KEY_LABEL: &str = "Public key";
pub const FINGERPRINT_LABEL: &str = "Fingerprint";
/// An `.env` entry exports as a note whose text is the file; the file's name
/// has no member in either format and rides as a custom field under this label.
/// The entry's own note would otherwise collide with the body in `notes`, so it
/// travels the same way.
pub const FILE_NAME_LABEL: &str = "file_name";
pub const NOTE_LABEL: &str = "Note";
/// An API key's environment, scopes and expiry have no member in either format
/// and ride as labelled fields; the CXF importer reads the first two back.
pub const ENVIRONMENT_LABEL: &str = "Environment";
pub const SCOPES_LABEL: &str = "Scopes";
pub const EXPIRES_LABEL: &str = "Expires";
/// The rest of what an entry holds that neither Bitwarden nor CXF has a member
/// for: a login's second address, a card's PIN, the parts of an ID document
/// beyond the number, the star, and when the password was last changed. Each
/// travels as a labelled field and is read back under the same label — see
/// [`labelled_fields`] and [`set_labelled`], the two halves of that trip.
pub const EMAIL_LABEL: &str = "Email";
pub const CARD_PIN_LABEL: &str = "PIN";
pub const NATIONALITY_LABEL: &str = "Nationality";
pub const BIRTH_DATE_LABEL: &str = "Date of birth";
pub const SEX_LABEL: &str = "Sex";
pub const ISSUE_DATE_LABEL: &str = "Issue date";
/// Distinct from [`EXPIRES_LABEL`], which is an API key's: a passport and a
/// token both expire, and the two must not land in each other's slot.
pub const EXPIRY_DATE_LABEL: &str = "Expiry date";
pub const AUTHORITY_LABEL: &str = "Authority";
pub const PERSONAL_NUMBER_LABEL: &str = "Personal number";
pub const PASSWORD_UPDATED_LABEL: &str = "Password updated";
pub const FAVORITE_LABEL: &str = "Favorite";

/// The values that have no member in any format we write, paired with the label
/// they travel under and whether they are secret (a hidden Bitwarden field, a
/// concealed CXF one). One table so a field cannot be written under one label
/// and looked for under another. The star and the personal number are not here:
/// Bitwarden has a member for each (`favorite`, `ssn`) and only CXF has to label
/// them, which it does itself.
pub fn labelled_fields(e: &ImportedEntry) -> Vec<(&'static str, &str, bool)> {
    [
        (EMAIL_LABEL, &e.email, false),
        (CARD_PIN_LABEL, &e.card_pin, true),
        (NATIONALITY_LABEL, &e.doc_nationality, false),
        (BIRTH_DATE_LABEL, &e.doc_birth_date, false),
        (SEX_LABEL, &e.doc_sex, false),
        (ISSUE_DATE_LABEL, &e.doc_issue_date, false),
        (EXPIRY_DATE_LABEL, &e.doc_expiry_date, false),
        (AUTHORITY_LABEL, &e.doc_authority, false),
        (PASSWORD_UPDATED_LABEL, &e.password_updated_at, false),
    ]
    .into_iter()
    .filter_map(|(label, value, secret)| value.as_deref().map(|v| (label, v, secret)))
    .collect()
}

/// The inverse of [`labelled_fields`]: fill the slots back in from whatever the
/// importer can offer under a label. `take` is how each format finds one — a
/// Bitwarden importer removes it from the custom fields it rode in, a CXF one
/// reads it out of the `custom-fields` credential — so the slot list is written
/// once and the two directions cannot drift.
pub fn set_labelled(entry: &mut ImportedEntry, mut take: impl FnMut(&str) -> Option<String>) {
    // Only when the label is there at all: a format with a member of its own
    // for the star has already set it.
    if let Some(value) = take(FAVORITE_LABEL) {
        entry.favorite = value.eq_ignore_ascii_case("true");
    }
    for (label, slot) in [
        (EMAIL_LABEL, &mut entry.email),
        (CARD_PIN_LABEL, &mut entry.card_pin),
        (NATIONALITY_LABEL, &mut entry.doc_nationality),
        (BIRTH_DATE_LABEL, &mut entry.doc_birth_date),
        (SEX_LABEL, &mut entry.doc_sex),
        (ISSUE_DATE_LABEL, &mut entry.doc_issue_date),
        (EXPIRY_DATE_LABEL, &mut entry.doc_expiry_date),
        (AUTHORITY_LABEL, &mut entry.doc_authority),
        (PERSONAL_NUMBER_LABEL, &mut entry.doc_personal_number),
        (PASSWORD_UPDATED_LABEL, &mut entry.password_updated_at),
    ] {
        if let Some(value) = take(label) {
            *slot = Some(value);
        }
    }
}

/// Every label [`set_labelled`] claims from a file's custom fields, in the order
/// it asks. Learned by asking it — a scratch entry and a closure that only notes
/// the label — so an exporter guarding against a collision (see
/// [`to_bitwarden_json`]) cannot drift from what the importer will take.
pub fn claimed_labels() -> Vec<String> {
    let mut labels = Vec::new();
    set_labelled(&mut ImportedEntry::default(), |label| {
        labels.push(label.to_owned());
        None
    });
    labels
}

/// Serialize to Bitwarden's unencrypted JSON export shape.
pub fn to_bitwarden_json(entries: &[ImportedEntry]) -> serde_json::Result<Vec<u8>> {
    let claimed = claimed_labels();
    let items: Vec<_> = entries
        .iter()
        .map(|e| {
            let mut item = json!({
                "type": bw_type(e.kind),
                "name": e.title,
                "notes": e.notes,
                // Bitwarden has a member for each of these, so they need no
                // label: the star it also calls `favorite`, and its two dates
                // are RFC 3339 like ours.
                "favorite": e.favorite,
                "creationDate": e.created_at,
                "revisionDate": e.updated_at,
            });
            // Everything Bitwarden has no member for goes into its custom
            // fields under our labels — FIRST, because the importer takes the
            // first field wearing a label. A user's own extra may share one
            // ("Email" is a natural thing to call a field); written after ours,
            // it stays theirs on the way back instead of landing in our slot.
            let ours = labelled_fields(e);
            for (label, value, secret) in &ours {
                let kind = if *secret { FIELD_HIDDEN } else { FIELD_TEXT };
                push_field(&mut item, label, value, kind);
            }
            // That only holds if ours is there to be taken first. A label the
            // importer claims that we had nothing to write under — the email is
            // unset, the star is off and Bitwarden has a member for it anyway —
            // still gets a field of ours, empty, when the user has one wearing
            // it: the importer takes the empty one (an empty value sets no
            // slot) and theirs stays theirs. Without a colliding extra nothing
            // is written, so the usual export is unchanged.
            for label in &claimed {
                let written = ours.iter().any(|(l, _, _)| l == label);
                let collides = e.extra.iter().any(|(l, _)| l.eq_ignore_ascii_case(label));
                if !written && collides {
                    push_field(&mut item, label, "", FIELD_TEXT);
                }
            }
            // Then the user's extras, in their order. `fields` is only ever
            // written when there is something to write, so an export of a
            // vault without any is byte-identical to before.
            for (label, value) in &e.extra {
                push_field(&mut item, label, value, FIELD_TEXT);
            }
            match e.kind {
                EntryKind::Login => {
                    item["login"] = json!({
                        "username": e.username,
                        "password": e.password,
                        "totp": e.otp,
                        "uris": e.url.as_ref().map(|u| vec![json!({ "uri": u })]).unwrap_or_default(),
                    });
                    // Only written when there is something to write, so an
                    // export of a passkey-less vault is unchanged.
                    if !e.passkeys.is_empty() {
                        item["login"]["fido2Credentials"] =
                            json!(e.passkeys.iter().map(fido2_credential).collect::<Vec<_>>());
                    }
                }
                EntryKind::Card => {
                    item["card"] = json!({
                        "cardholderName": e.cardholder,
                        "number": e.card_number,
                        "expMonth": e.card_month,
                        "expYear": e.card_year,
                        "code": e.card_cvc,
                    });
                }
                EntryKind::Identity => {
                    let (first, last) = split_name(e.holder_name.as_deref());
                    let mut identity = json!({
                        "firstName": first,
                        "lastName": last,
                        "country": e.doc_country,
                        // Bitwarden's one member for a national identifier;
                        // the rest of the document rides as labelled fields.
                        "ssn": e.doc_personal_number,
                    });
                    // Bitwarden holds exactly two document numbers, so the
                    // document's own type picks which slot this one goes in.
                    identity[license_slot(e)] = json!(e.doc_number);
                    item["identity"] = identity;
                }
                EntryKind::Ssh => {
                    item["sshKey"] = json!({
                        "privateKey": e.ssh_private_key,
                        "publicKey": e.ssh_public_key,
                        "keyFingerprint": e.ssh_fingerprint,
                    });
                    // Bitwarden's SSH item has no passphrase member, so it goes
                    // in a hidden custom field rather than nowhere.
                    if let Some(passphrase) = &e.ssh_passphrase {
                        push_field(&mut item, PASSPHRASE_LABEL, passphrase, FIELD_HIDDEN);
                    }
                }
                EntryKind::Env => {
                    // The file text is the secret, so it takes the note's
                    // body; the entry's own note and the file name have no
                    // member of their own and follow as custom fields. One-way:
                    // the importer reads a type-2 item back as a note, since
                    // nothing marks it as having been a file.
                    item["notes"] = json!(e.env_body);
                    if let Some(note) = &e.notes {
                        push_field(&mut item, NOTE_LABEL, note, FIELD_TEXT);
                    }
                    if let Some(name) = &e.env_file_name {
                        push_field(&mut item, FILE_NAME_LABEL, name, FIELD_TEXT);
                    }
                }
                EntryKind::ApiKey => {
                    // The token as the password and the base URL as the site,
                    // so Bitwarden fills it where the key is used; what has no
                    // member follows as custom fields. One-way: it comes back
                    // as the login it looks like.
                    item["login"] = json!({
                        "username": Value::Null,
                        "password": e.api_key,
                        "totp": Value::Null,
                        "uris": e.url.as_ref().map(|u| vec![json!({ "uri": u })]).unwrap_or_default(),
                    });
                    for (label, value) in [
                        (ENVIRONMENT_LABEL, &e.api_environment),
                        (SCOPES_LABEL, &e.api_scopes),
                        (EXPIRES_LABEL, &e.api_expires),
                    ] {
                        if let Some(value) = value {
                            push_field(&mut item, label, value, FIELD_TEXT);
                        }
                    }
                }
                EntryKind::Note => {}
            }
            item
        })
        .collect();

    #[derive(Serialize)]
    struct Export {
        encrypted: bool,
        folders: Vec<()>,
        items: Vec<serde_json::Value>,
    }
    serde_json::to_vec_pretty(&Export {
        encrypted: false,
        folders: vec![],
        items,
    })
}

/// Append one custom field to a Bitwarden item, starting the list if the extras
/// did not already.
fn push_field(item: &mut Value, name: &str, value: &str, field_type: u8) {
    let field = json!({ "name": name, "value": value, "type": field_type });
    match item["fields"].as_array_mut() {
        Some(fields) => fields.push(field),
        None => item["fields"] = json!([field]),
    }
}

/// Which of Bitwarden's two document-number members carries this document.
fn license_slot(e: &ImportedEntry) -> &'static str {
    if e.doc_type.as_deref() == Some("driver_license") {
        "licenseNumber"
    } else {
        "passportNumber"
    }
}

/// A full name split into Bitwarden's first/last pair: first word against the
/// rest. Lossy by nature — "ADA KING LOVELACE" cannot say which part is which —
/// so the app keeps the whole name and only the export is split.
fn split_name(name: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else {
        return (None, None);
    };
    match name.split_once(char::is_whitespace) {
        Some((first, rest)) => (
            Some(first.to_owned()),
            Some(rest.trim_start().to_owned()).filter(|r| !r.is_empty()),
        ),
        None => (Some(name.to_owned()), None),
    }
}

/// One Bitwarden `fido2Credentials` element. Every value is a string there,
/// `counter` included; the key material is passed through verbatim.
fn fido2_credential(p: &ImportedPasskey) -> serde_json::Value {
    json!({
        "credentialId": p.credential_id,
        "keyType": KEY_TYPE,
        "keyAlgorithm": KEY_ALGORITHM,
        "keyCurve": KEY_CURVE,
        "keyValue": p.private_key,
        "rpId": p.rp_id,
        "rpName": p.rp_name,
        "userHandle": p.user_handle,
        "userName": p.user_name,
        "userDisplayName": p.user_display_name,
        "counter": p.counter.to_string(),
        "discoverable": "true",
        "creationDate": p.created_at,
    })
}

/// Serialize to a FIDO Credential Exchange Format 1.0 document — the inverse of
/// [`super::cxf`]. One account holding one item per entry; each value the app
/// has becomes a credential on that item. Optional members are omitted rather
/// than written empty, as the format asks.
///
/// Spec: <https://fidoalliance.org/specs/cx/cxf-v1.0-ps-20250814.html>
pub fn to_cxf_json(entries: &[ImportedEntry]) -> serde_json::Result<Vec<u8>> {
    let items: Vec<Value> = entries.iter().map(cxf_item).collect();
    serde_json::to_vec_pretty(&json!({
        "version": { "major": 1, "minor": 0 },
        // An RP id is a domain, so the exporter is identified by the project's
        // site rather than by the app name.
        "exporterRpId": EXPORTER_RP_ID,
        "exporterDisplayName": APP_NAME,
        "timestamp": now_secs(),
        "accounts": [{
            "id": random_id(),
            "username": "",
            "email": "",
            "collections": [],
            "items": items,
        }],
    }))
}

/// One CXF `Item`. An entry maps to a single item carrying every credential it
/// has: a login is basic-auth plus a passkey each, plus TOTP and a note when
/// set; notes ride along on cards too, since CXF has nowhere else to put them.
fn cxf_item(e: &ImportedEntry) -> Value {
    let mut credentials: Vec<Value> = Vec::new();
    match e.kind {
        EntryKind::Login => {
            let mut basic = json!({ "type": "basic-auth" });
            put(&mut basic, "username", editable("string", &e.username));
            put(
                &mut basic,
                "password",
                editable("concealed-string", &e.password),
            );
            credentials.push(basic);
            credentials.extend(e.passkeys.iter().map(cxf_passkey));
            if let Some(secret) = &e.otp {
                // CXF spells the parameters out beside the seed, so the stored
                // value is taken apart here. Something we cannot read goes out
                // verbatim with the defaults — the same guess as before, and a
                // reader that understands it has lost nothing.
                let p = otp::parse(secret).unwrap_or_else(|_| OtpParams {
                    secret: secret.clone(),
                    ..Default::default()
                });
                credentials.push(json!({
                    "type": "totp",
                    "secret": p.secret,
                    "period": p.period,
                    "digits": p.digits,
                    "algorithm": p.algorithm.as_str(),
                }));
            }
        }
        EntryKind::Card => {
            let mut card = json!({ "type": "credit-card" });
            put(
                &mut card,
                "number",
                editable("concealed-string", &e.card_number),
            );
            put(&mut card, "fullName", editable("string", &e.cardholder));
            put(
                &mut card,
                "verificationNumber",
                editable("concealed-string", &e.card_cvc),
            );
            put(
                &mut card,
                "expiryDate",
                editable("year-month", &expiry_date(e)),
            );
            credentials.push(card);
        }
        EntryKind::Ssh => {
            // The private key is written as the app holds it (OpenSSH PEM),
            // which is what `ssh` itself reads. The parts CXF's `ssh-key` has
            // no member for travel in a custom-fields credential beside it.
            let mut key = json!({ "type": "ssh-key", "keyType": ssh_key_type(e) });
            put(
                &mut key,
                "privateKey",
                editable("concealed-string", &e.ssh_private_key),
            );
            put(&mut key, "keyComment", editable("string", &ssh_comment(e)));
            credentials.push(key);

            let fields: Vec<Value> = [
                (PUBLIC_KEY_LABEL, "string", &e.ssh_public_key),
                (FINGERPRINT_LABEL, "string", &e.ssh_fingerprint),
                (PASSPHRASE_LABEL, "concealed-string", &e.ssh_passphrase),
            ]
            .into_iter()
            .filter_map(|(label, field_type, value)| {
                value
                    .as_deref()
                    .map(|v| json!({ "fieldType": field_type, "label": label, "value": v }))
            })
            .collect();
            if !fields.is_empty() {
                credentials.push(json!({
                    "type": "custom-fields",
                    "id": random_id(),
                    "label": "SSH key",
                    "fields": fields,
                }));
            }
        }
        EntryKind::Env => {
            // CXF has no credential for a file, so the text goes out as a
            // `note` — first, so an importer that keeps one note per item
            // keeps the file. One-way, as with Bitwarden: it comes back as a
            // note. The file name rides in a custom-fields credential beside
            // it, the way the ssh-key's extra parts do.
            if let Some(body) = &e.env_body {
                credentials
                    .push(json!({ "type": "note", "content": editable_value("string", body) }));
            }
            if let Some(name) = &e.env_file_name {
                credentials.push(json!({
                    "type": "custom-fields",
                    "id": random_id(),
                    "label": "Env file",
                    "fields": [{ "fieldType": "string", "label": FILE_NAME_LABEL, "value": name }],
                }));
            }
        }
        EntryKind::ApiKey => {
            // CXF has an `api-key` credential of its own: the token, the URL
            // it is sent to and the date it lapses. Environment and scopes have
            // no member there and travel in a custom-fields credential beside
            // it, the way the ssh-key's extra parts do.
            let mut key = json!({ "type": "api-key" });
            put(&mut key, "key", editable("concealed-string", &e.api_key));
            put(&mut key, "url", editable("string", &e.url));
            put(&mut key, "expiryDate", editable("date", &e.api_expires));
            credentials.push(key);

            let fields: Vec<Value> = [
                (ENVIRONMENT_LABEL, &e.api_environment),
                (SCOPES_LABEL, &e.api_scopes),
            ]
            .into_iter()
            .filter_map(|(label, value)| {
                value
                    .as_deref()
                    .map(|v| json!({ "fieldType": "string", "label": label, "value": v }))
            })
            .collect();
            if !fields.is_empty() {
                credentials.push(json!({
                    "type": "custom-fields",
                    "id": random_id(),
                    "label": "API key",
                    "fields": fields,
                }));
            }
        }
        // CXF has purpose-built `passport`/`drivers-license`/`identity-document`
        // credentials, but nothing that covers all five document types the app
        // holds; until they are mapped one by one, an identity exports as its
        // title and note like a secure note does.
        EntryKind::Identity | EntryKind::Note => {}
    }
    if let Some(notes) = &e.notes {
        credentials.push(json!({ "type": "note", "content": editable_value("string", notes) }));
    }
    // What CXF has no credential member for — a login's email, a card's PIN,
    // the rest of a document, the star — rides in a custom-fields credential
    // under the labels the importer reads back, the way the ssh-key's extra
    // parts do. Written only when there is something to write, so an export of
    // a vault that uses none of them is unchanged.
    let mut fields: Vec<Value> = labelled_fields(e)
        .into_iter()
        .map(|(label, value, secret)| {
            let field_type = if secret { "concealed-string" } else { "string" };
            json!({ "fieldType": field_type, "label": label, "value": value })
        })
        .collect();
    // Bitwarden has a member for each of these two; CXF has neither.
    if e.favorite {
        fields.push(json!({ "fieldType": "string", "label": FAVORITE_LABEL, "value": "true" }));
    }
    if let Some(number) = &e.doc_personal_number {
        fields.push(
            json!({ "fieldType": "string", "label": PERSONAL_NUMBER_LABEL, "value": number }),
        );
    }
    if !fields.is_empty() {
        credentials.push(json!({
            "type": "custom-fields",
            "id": random_id(),
            "label": APP_NAME,
            "fields": fields,
        }));
    }

    let mut item = json!({
        "id": random_id(),
        "title": e.title,
        "credentials": credentials,
    });
    // CXF dates an item in Unix seconds. An entry that has no date of its own
    // is written without one rather than stamped with today's, so a round-trip
    // does not invent history the vault never had.
    put(
        &mut item,
        "creationAt",
        unix_secs(&e.created_at).map(Value::from),
    );
    put(
        &mut item,
        "modifiedAt",
        unix_secs(&e.updated_at).map(Value::from),
    );
    put(
        &mut item,
        "scope",
        e.url
            .as_ref()
            .map(|u| json!({ "urls": [u], "androidApps": [] })),
    );
    put(
        &mut item,
        "tags",
        (!e.tags.is_empty()).then(|| json!(e.tags)),
    );
    item
}

/// One CXF `passkey` credential. `fido2Extensions` is optional and the app
/// keeps none, so it is left out; the counter has nowhere to go in CXF.
fn cxf_passkey(p: &ImportedPasskey) -> Value {
    json!({
        "type": "passkey",
        "credentialId": p.credential_id,
        "rpId": p.rp_id,
        "username": p.user_name,
        "userDisplayName": p.user_display_name,
        "userHandle": p.user_handle,
        "key": p.private_key,
    })
}

/// CXF's `keyType` is the OpenSSH algorithm name — the first token of the
/// public line. The app only generates ed25519, so that is the fallback for a
/// pasted key whose public line was never filled in.
fn ssh_key_type(e: &ImportedEntry) -> &str {
    e.ssh_public_key
        .as_deref()
        .and_then(|line| line.split_whitespace().next())
        .unwrap_or("ssh-ed25519")
}

/// The public line's trailing comment (`ssh-ed25519 AAAA… alice@laptop`), when
/// there is one.
fn ssh_comment(e: &ImportedEntry) -> Option<String> {
    let mut parts = e.ssh_public_key.as_deref()?.splitn(3, ' ');
    let (_, _, comment) = (parts.next()?, parts.next()?, parts.next()?);
    let comment = comment.trim();
    (!comment.is_empty()).then(|| comment.to_owned())
}

/// CXF wants `YYYY-MM`; the app stores month and year loose (`1`, `01`, `30`,
/// `2030`), so both are normalized. A card missing either has no expiry date.
fn expiry_date(e: &ImportedEntry) -> Option<String> {
    let (month, year) = (e.card_month.as_deref()?, e.card_year.as_deref()?);
    let year = if year.len() == 2 {
        format!("20{year}")
    } else {
        year.to_owned()
    };
    Some(format!("{year}-{month:0>2}"))
}

/// A CXF `EditableField` — the wrapper every user-visible value sits in.
fn editable_value(field_type: &str, value: &str) -> Value {
    json!({ "fieldType": field_type, "value": value })
}

fn editable(field_type: &str, value: &Option<String>) -> Option<Value> {
    value.as_deref().map(|v| editable_value(field_type, v))
}

/// Set `key` only when there is something to set: CXF's optional members are
/// absent, never present-and-empty.
fn put(object: &mut Value, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        object[key] = value;
    }
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// The app stores RFC 3339 strings; CXF dates in Unix seconds. A stamp we
/// cannot parse is treated as no stamp rather than as the epoch.
fn unix_secs(value: &Option<String>) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value.as_deref()?)
        .ok()
        .map(|t| t.timestamp())
}

/// CXF ids are `b64url`: the app's usual 16 random bytes, in the encoding the
/// format asks for.
fn random_id() -> String {
    URL_SAFE_NO_PAD.encode(crate::crypto::random_id_bytes())
}

const COLUMNS: &[&str] = &[
    "type",
    "title",
    "username",
    "password",
    "url",
    "notes",
    "otp",
    "card_number",
    "card_month",
    "card_year",
    "card_cvc",
    "cardholder",
    "doc_type",
    "doc_number",
    "doc_country",
    "holder_name",
    "ssh_private_key",
    "ssh_public_key",
    "ssh_fingerprint",
    "ssh_passphrase",
    "api_key",
    "environment",
    "scopes",
    "expires",
    "body",
    "file_name",
    "tags",
    // Added in version 3. They are last (bar the marker) so a version-2 sheet
    // is this one with the tail missing, which reads back as absent.
    "email",
    "card_pin",
    "doc_nationality",
    "doc_birth_date",
    "doc_sex",
    "doc_issue_date",
    "doc_expiry_date",
    "doc_authority",
    "doc_personal_number",
    "favorite",
    "created_at",
    "updated_at",
    "password_updated_at",
    CSV_VERSION_HEADER,
];

/// Marks rows produced by Rowel so the importer may reverse spreadsheet
/// escaping without guessing whether a leading apostrophe was user data.
pub const CSV_VERSION_HEADER: &str = "_rowel_csv_version";
/// 3 added a column for every field that had none (the star, the timestamps,
/// the rest of a document). The importer reads any marked sheet, so a 2 still
/// comes back whole — its missing columns simply read as absent.
pub const CSV_VERSION: &str = "3";

/// Serialize to a generic CSV. Every cell is passed through [`sanitize_cell`].
pub fn to_generic_csv(entries: &[ImportedEntry]) -> csv::Result<Vec<u8>> {
    let mut wtr = csv::Writer::from_writer(vec![]);
    wtr.write_record(COLUMNS)?;
    for e in entries {
        let row = [
            e.kind.as_str().to_string(),
            e.title.clone(),
            e.username.clone().unwrap_or_default(),
            e.password.clone().unwrap_or_default(),
            e.url.clone().unwrap_or_default(),
            e.notes.clone().unwrap_or_default(),
            e.otp.clone().unwrap_or_default(),
            e.card_number.clone().unwrap_or_default(),
            e.card_month.clone().unwrap_or_default(),
            e.card_year.clone().unwrap_or_default(),
            e.card_cvc.clone().unwrap_or_default(),
            e.cardholder.clone().unwrap_or_default(),
            e.doc_type.clone().unwrap_or_default(),
            e.doc_number.clone().unwrap_or_default(),
            e.doc_country.clone().unwrap_or_default(),
            e.holder_name.clone().unwrap_or_default(),
            e.ssh_private_key.clone().unwrap_or_default(),
            e.ssh_public_key.clone().unwrap_or_default(),
            e.ssh_fingerprint.clone().unwrap_or_default(),
            e.ssh_passphrase.clone().unwrap_or_default(),
            e.api_key.clone().unwrap_or_default(),
            e.api_environment.clone().unwrap_or_default(),
            e.api_scopes.clone().unwrap_or_default(),
            e.api_expires.clone().unwrap_or_default(),
            e.env_body.clone().unwrap_or_default(),
            e.env_file_name.clone().unwrap_or_default(),
            e.tags.join(";"),
            e.email.clone().unwrap_or_default(),
            e.card_pin.clone().unwrap_or_default(),
            e.doc_nationality.clone().unwrap_or_default(),
            e.doc_birth_date.clone().unwrap_or_default(),
            e.doc_sex.clone().unwrap_or_default(),
            e.doc_issue_date.clone().unwrap_or_default(),
            e.doc_expiry_date.clone().unwrap_or_default(),
            e.doc_authority.clone().unwrap_or_default(),
            e.doc_personal_number.clone().unwrap_or_default(),
            e.favorite.to_string(),
            e.created_at.clone().unwrap_or_default(),
            e.updated_at.clone().unwrap_or_default(),
            e.password_updated_at.clone().unwrap_or_default(),
            CSV_VERSION.to_string(),
        ];
        wtr.write_record(row.iter().map(|c| sanitize_cell(c)))?;
    }
    wtr.flush()?;
    wtr.into_inner()
        .map_err(|e| csv::Error::from(std::io::Error::other(e.to_string())))
}

/// Neutralize spreadsheet formula injection with a reversible encoding. A
/// formula-looking cell gains a leading apostrophe; an apostrophe already in
/// the data is doubled so decoding a versioned Rowel row is unambiguous.
pub fn sanitize_cell(cell: &str) -> String {
    if cell.starts_with('\'') || starts_like_a_formula(cell) {
        format!("'{cell}")
    } else {
        cell.to_string()
    }
}

/// The inverse for a row explicitly marked with [`CSV_VERSION`]. Unversioned
/// generic CSV never passes through this function: its apostrophes are data.
pub fn unsanitize_cell(cell: &str) -> String {
    cell.strip_prefix('\'').unwrap_or(cell).to_string()
}

fn starts_like_a_formula(cell: &str) -> bool {
    matches!(
        cell.chars().next(),
        Some('=') | Some('+') | Some('-') | Some('@') | Some('\t') | Some('\r')
    )
}
