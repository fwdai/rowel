use super::export::{
    sanitize_cell, to_bitwarden_json, to_cxf_json, to_generic_csv, unsanitize_cell, CSV_VERSION,
};
use super::{detect, EntryKind, Format, ImportedEntry, ImportedPasskey, Importer};

fn parse(fmt: Format, bytes: &[u8]) -> super::ImportResult {
    fmt.importer().parse(bytes)
}

#[test]
fn bitwarden_maps_every_item_type_and_flags_unsupported() {
    let json = br#"{"items":[
      {"type":1,"name":"GitHub","notes":"n","login":{"username":"octo","password":"pw","totp":"OTP","uris":[{"uri":"https://github.com"}]}},
      {"type":2,"name":"Secret","notes":"body"},
      {"type":3,"name":"Visa","card":{"cardholderName":"A B","number":"4111","expMonth":"01","expYear":"30","code":"123"}},
      {"type":4,"name":"Passport","identity":{"firstName":"Ada","lastName":"Lovelace","country":"GBR","passportNumber":"X1234567"}},
      {"type":9,"name":"Whatever"}
    ]}"#;
    let r = parse(Format::Bitwarden, json);
    assert_eq!(r.entries.len(), 4);
    assert_eq!(r.errors.len(), 1);
    assert_eq!(r.errors[0].row, 5);

    let login = &r.entries[0];
    assert_eq!(login.kind, EntryKind::Login);
    assert_eq!(login.title, "GitHub");
    assert_eq!(login.username.as_deref(), Some("octo"));
    assert_eq!(login.url.as_deref(), Some("https://github.com"));
    assert_eq!(login.otp.as_deref(), Some("OTP"));

    assert_eq!(r.entries[1].kind, EntryKind::Note);
    let card = &r.entries[2];
    assert_eq!(card.kind, EntryKind::Card);
    assert_eq!(card.card_number.as_deref(), Some("4111"));
    assert_eq!(card.cardholder.as_deref(), Some("A B"));

    let identity = &r.entries[3];
    assert_eq!(identity.kind, EntryKind::Identity);
    assert_eq!(identity.doc_type.as_deref(), Some("passport"));
    assert_eq!(identity.doc_number.as_deref(), Some("X1234567"));
    assert_eq!(identity.doc_country.as_deref(), Some("GBR"));
    assert_eq!(identity.holder_name.as_deref(), Some("Ada Lovelace"));
}

// The per-row error contract covers the item type itself: an unknown number and
// a missing member each cost their own row, not the file. A type written as a
// string is still a type — Bitwarden-shaped exports come from many producers.
#[test]
fn bitwarden_reports_an_odd_item_type_per_row() {
    let json = br#"{"items":[
      {"type":1,"name":"GitHub","login":{"username":"octo"}},
      {"type":300,"name":"Odd"},
      {"name":"Typeless"},
      {"type":"2","name":"Stringly"}
    ]}"#;
    let r = parse(Format::Bitwarden, json);
    assert_eq!(r.entries.len(), 2);
    assert_eq!(r.entries[0].title, "GitHub");
    assert_eq!(r.entries[1].kind, EntryKind::Note);
    assert_eq!(r.entries[1].title, "Stringly");
    assert_eq!(r.errors.len(), 2);
    assert_eq!(r.errors[0].row, 2);
    assert!(r.errors[0].message.contains("300"));
    assert_eq!(r.errors[1].row, 3);
}

// A licence number is the only thing in a Bitwarden identity that says what the
// document is, so it also decides the type.
#[test]
fn bitwarden_reads_a_licence_number_as_a_driver_licence() {
    let json = br#"{"items":[
      {"type":4,"name":"Licence","identity":{"firstName":"Ada","middleName":"King","lastName":"Lovelace","licenseNumber":"D-99"}}
    ]}"#;
    let r = parse(Format::Bitwarden, json);
    assert!(r.errors.is_empty());
    let identity = &r.entries[0];
    assert_eq!(identity.doc_type.as_deref(), Some("driver_license"));
    assert_eq!(identity.doc_number.as_deref(), Some("D-99"));
    assert_eq!(identity.holder_name.as_deref(), Some("Ada King Lovelace"));
}

// The identity is written into Bitwarden's own members, so it comes back as the
// same document — bar the first/last split, which the full name absorbs again.
#[test]
fn round_trip_bitwarden_identity() {
    let entries = vec![
        ImportedEntry {
            kind: EntryKind::Identity,
            title: "Passport".into(),
            doc_type: Some("passport".into()),
            doc_number: Some("X1234567".into()),
            doc_country: Some("GBR".into()),
            holder_name: Some("ADA LOVELACE".into()),
            ..Default::default()
        },
        ImportedEntry {
            kind: EntryKind::Identity,
            title: "Licence".into(),
            doc_type: Some("driver_license".into()),
            doc_number: Some("D-99".into()),
            holder_name: Some("Mononym".into()),
            ..Default::default()
        },
    ];
    let bytes = to_bitwarden_json(&entries).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(out["items"][0]["type"], 4);
    assert_eq!(out["items"][0]["identity"]["passportNumber"], "X1234567");
    assert_eq!(out["items"][1]["identity"]["licenseNumber"], "D-99");

    let back = parse(Format::Bitwarden, &bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries, entries);
}

// Bitwarden's custom fields are the extras: text and hidden are values a person
// typed, boolean and linked are its own machinery and have nothing to carry.
#[test]
fn bitwarden_reads_text_and_hidden_fields_as_extras() {
    let json = br#"{"items":[
      {"type":4,"name":"Licence","identity":{"licenseNumber":"D-99"},"fields":[
        {"name":"Categories","value":"B, BE","type":0},
        {"name":"Restrictions","value":"01","type":1},
        {"name":"Favourite","value":"true","type":2},
        {"name":"Linked","value":"3","type":3},
        {"name":"","value":"","type":0},
        {"name":"Issuer note"}
      ]}
    ]}"#;
    let r = parse(Format::Bitwarden, json);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(
        r.entries[0].extra,
        vec![
            ("Categories".into(), "B, BE".into()),
            ("Restrictions".into(), "01".into()),
            // No type at all reads as text, and a value-less field is still a
            // label the user wrote.
            ("Issuer note".into(), String::new()),
        ]
    );
}

// Extras belong to no kind, so a login keeps them too — and comes back with the
// same pairs in the same order.
#[test]
fn round_trip_bitwarden_extras() {
    let entries = vec![ImportedEntry {
        kind: EntryKind::Login,
        title: "Acme".into(),
        username: Some("alice".into()),
        extra: vec![
            ("Account ID".into(), "42".into()),
            ("Blood type".into(), "O+".into()),
        ],
        ..Default::default()
    }];
    let bytes = to_bitwarden_json(&entries).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(out["items"][0]["fields"][0]["name"], "Account ID");
    assert_eq!(out["items"][0]["fields"][0]["type"], 0);
    assert_eq!(out["items"][0]["fields"][1]["value"], "O+");

    let back = parse(Format::Bitwarden, &bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries, entries);
}

// An export of a vault with no extras never mentions custom fields.
#[test]
fn bitwarden_export_omits_fields_without_extras() {
    let entries = vec![ImportedEntry {
        kind: EntryKind::Note,
        title: "Wifi".into(),
        notes: Some("on the router".into()),
        ..Default::default()
    }];
    let bytes = to_bitwarden_json(&entries).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(out["items"][0].get("fields").is_none());
}

#[test]
fn bitwarden_maps_fido2_credentials_onto_passkeys() {
    let json = br#"{"items":[
      {"type":1,"name":"Acme","login":{"username":"alice","fido2Credentials":[
        {"credentialId":"Y3JlZDE","keyType":"public-key","keyAlgorithm":"ECDSA","keyCurve":"P-256",
         "keyValue":"cGsx","rpId":"acme.test","rpName":"Acme","userHandle":"dWgx","userName":"alice",
         "userDisplayName":"Alice","counter":"7","discoverable":"true",
         "creationDate":"2024-01-01T00:00:00.000Z"},
        {"credentialId":"Y3JlZDI","keyValue":"cGsy","rpId":"other.test","userHandle":"dWgy",
         "userName":"bob","userDisplayName":"Bob"}
      ]}}
    ]}"#;
    let r = parse(Format::Bitwarden, json);
    assert!(r.errors.is_empty());
    let passkeys = &r.entries[0].passkeys;
    assert_eq!(passkeys.len(), 2);
    assert_eq!(
        passkeys[0],
        ImportedPasskey {
            credential_id: "Y3JlZDE".into(),
            rp_id: "acme.test".into(),
            rp_name: Some("Acme".into()),
            user_handle: "dWgx".into(),
            user_name: "alice".into(),
            user_display_name: "Alice".into(),
            private_key: "cGsx".into(),
            counter: 7,
            created_at: Some("2024-01-01T00:00:00.000Z".into()),
        }
    );
    // Optional fields absent: no rpName/creationDate, counter defaults to 0.
    assert_eq!(passkeys[1].credential_id, "Y3JlZDI");
    assert_eq!(passkeys[1].private_key, "cGsy");
    assert_eq!(passkeys[1].rp_name, None);
    assert_eq!(passkeys[1].created_at, None);
    assert_eq!(passkeys[1].counter, 0);
}

// An unusable credential — wrong algorithm, or missing its id / rp / key — is
// dropped on its own, one error each; the login and its good passkey import.
#[test]
fn bitwarden_drops_unsupported_or_incomplete_passkeys() {
    let json = br#"{"items":[
      {"type":1,"name":"Acme","login":{"username":"alice","fido2Credentials":[
        {"credentialId":"a","keyAlgorithm":"RSA","keyValue":"k","rpId":"acme.test"},
        {"credentialId":"b","keyAlgorithm":"ECDSA","keyCurve":"P-256","keyValue":"k","rpId":"acme.test"},
        {"credentialId":"c","rpId":"acme.test"},
        {"credentialId":"","keyValue":"k","rpId":"acme.test"},
        {"credentialId":"d","keyValue":"k"}
      ]}}
    ]}"#;
    let r = parse(Format::Bitwarden, json);
    assert_eq!(r.entries.len(), 1);
    assert_eq!(r.entries[0].username.as_deref(), Some("alice"));
    assert_eq!(r.entries[0].passkeys.len(), 1);
    assert_eq!(r.entries[0].passkeys[0].credential_id, "b");
    assert_eq!(r.errors.len(), 4);
    assert!(r
        .errors
        .iter()
        .all(|e| e.row == 1 && e.message.contains("passkey")));
}

// Bitwarden writes `counter` as a string; a numeric one is taken as-is rather
// than failing the whole file.
#[test]
fn bitwarden_accepts_a_numeric_passkey_counter() {
    let json = br#"{"items":[
      {"type":1,"name":"Acme","login":{"fido2Credentials":[
        {"credentialId":"a","keyValue":"k","rpId":"acme.test","counter":3}
      ]}}
    ]}"#;
    let r = parse(Format::Bitwarden, json);
    assert!(r.errors.is_empty());
    assert_eq!(r.entries[0].passkeys[0].counter, 3);
}

#[test]
fn bitwarden_login_without_fido2_credentials_has_no_passkeys() {
    let json = br#"{"items":[{"type":1,"name":"Acme","login":{"username":"alice"}}]}"#;
    let r = parse(Format::Bitwarden, json);
    assert!(r.errors.is_empty());
    assert!(r.entries[0].passkeys.is_empty());
}

#[test]
fn generic_csv_maps_by_header_aliases() {
    let csv = b"name,url,username,password,note\nGitHub,https://github.com,octo,pw,hello\n";
    let r = parse(Format::GenericCsv, csv);
    assert_eq!(r.entries.len(), 1);
    assert!(r.errors.is_empty());
    let e = &r.entries[0];
    assert_eq!(e.title, "GitHub");
    assert_eq!(e.username.as_deref(), Some("octo"));
    assert_eq!(e.password.as_deref(), Some("pw"));
    assert_eq!(e.notes.as_deref(), Some("hello"));
}

#[test]
fn browser_csv_maps_safari_headers() {
    let csv = b"Title,URL,Username,Password,Notes,OTPAuth\nMail,https://mail.com,me,secret,,SEED\n";
    let r = parse(Format::BrowserCsv, csv);
    assert_eq!(r.entries.len(), 1);
    let e = &r.entries[0];
    assert_eq!(e.title, "Mail");
    assert_eq!(e.url.as_deref(), Some("https://mail.com"));
    assert_eq!(e.otp.as_deref(), Some("SEED"));
}

#[test]
fn lastpass_csv_splits_logins_and_secure_notes() {
    let csv = b"url,username,password,totp,extra,name,grouping,fav\n\
        https://site.com,user,pw,SEED,,My Site,Personal,0\n\
        http://sn,,,,note body,My Note,Personal,0\n";
    let r = parse(Format::LastpassCsv, csv);
    assert_eq!(r.entries.len(), 2);
    assert_eq!(r.entries[0].kind, EntryKind::Login);
    assert_eq!(r.entries[0].tags, vec!["Personal".to_string()]);
    assert_eq!(r.entries[0].otp.as_deref(), Some("SEED"));
    assert_eq!(r.entries[1].kind, EntryKind::Note);
    assert_eq!(r.entries[1].notes.as_deref(), Some("note body"));
}

#[test]
fn keepass_csv_maps_group_to_tag() {
    let csv = b"Group,Title,Username,Password,URL,Notes,TOTP\n\
        Root/Web,GitHub,octo,pw,https://github.com,note,SEED\n";
    let r = parse(Format::KeepassCsv, csv);
    assert_eq!(r.entries.len(), 1);
    let e = &r.entries[0];
    assert_eq!(e.title, "GitHub");
    assert_eq!(e.tags, vec!["Web".to_string()]);
    assert_eq!(e.otp.as_deref(), Some("SEED"));
}

// A malformed/empty row is reported but never aborts the batch.
#[test]
fn malformed_row_does_not_abort_batch() {
    let csv = b"name,url,username,password\n\
        Good,https://a.com,u,p\n\
        ,,,\n\
        Also,https://b.com,u2,p2\n";
    let r = parse(Format::GenericCsv, csv);
    assert_eq!(r.entries.len(), 2);
    assert_eq!(r.errors.len(), 1);
    assert_eq!(r.errors[0].row, 3); // the empty middle row
    assert_eq!(r.entries[0].title, "Good");
    assert_eq!(r.entries[1].title, "Also");
}

// Dry-run reports the would-be count and every row error without any side effect
// (the parser itself never writes — the command just skips the store on dry_run).
#[test]
fn dry_run_counts_match_parse() {
    let json = br#"{"items":[
      {"type":1,"name":"A","login":{"username":"a"}},
      {"type":9,"name":"bad"},
      {"type":1,"name":"B","login":{"username":"b"}}
    ]}"#;
    let r = parse(Format::Bitwarden, json);
    assert_eq!(r.entries.len(), 2); // imported_would_be
    assert_eq!(r.errors.len(), 1);
}

#[test]
fn sanitize_neutralizes_formula_cells() {
    assert_eq!(sanitize_cell("=cmd|'/c calc'"), "'=cmd|'/c calc'");
    assert_eq!(sanitize_cell("+1"), "'+1");
    assert_eq!(sanitize_cell("-1"), "'-1");
    assert_eq!(sanitize_cell("@x"), "'@x");
    assert_eq!(sanitize_cell("\treally"), "'\treally");
    assert_eq!(sanitize_cell("'literal"), "''literal");
    assert_eq!(sanitize_cell("safe"), "safe");
    assert_eq!(sanitize_cell(""), "");
    for cell in ["=x", "+x", "-x", "@x", "\tx", "\rx", "'=x", "''x"] {
        assert_eq!(unsanitize_cell(&sanitize_cell(cell)), cell);
    }
}

#[test]
fn csv_export_sanitizes_injection() {
    let entries = vec![ImportedEntry {
        kind: EntryKind::Login,
        title: "=HYPERLINK(\"http://evil\")".into(),
        username: Some("+44".into()),
        ..Default::default()
    }];
    let out = String::from_utf8(to_generic_csv(&entries).unwrap()).unwrap();
    assert!(out.contains("'=HYPERLINK"));
    assert!(out.contains("'+44"));
}

// Export -> import round-trips logins through both formats.
#[test]
fn round_trip_generic_csv() {
    let entries = vec![ImportedEntry {
        kind: EntryKind::Login,
        title: "=Acme".into(),
        username: Some("+neo".into()),
        password: Some("'=trinity".into()),
        url: Some("https://acme.test".into()),
        notes: Some("'@hi".into()),
        otp: Some("-SEED".into()),
        ..Default::default()
    }];
    let bytes = to_generic_csv(&entries).unwrap();
    let back = super::csv::GenericCsv.parse(&bytes);
    assert!(back.errors.is_empty());
    assert_eq!(back.entries.len(), 1);
    let e = &back.entries[0];
    assert_eq!(e.title, "=Acme");
    assert_eq!(e.username.as_deref(), Some("+neo"));
    assert_eq!(e.password.as_deref(), Some("'=trinity"));
    assert_eq!(e.notes.as_deref(), Some("'@hi"));
    assert_eq!(e.otp.as_deref(), Some("-SEED"));
}

#[test]
fn round_trip_bitwarden_json() {
    let entries = vec![
        ImportedEntry {
            kind: EntryKind::Login,
            title: "Acme".into(),
            username: Some("neo".into()),
            password: Some("trinity".into()),
            url: Some("https://acme.test".into()),
            otp: Some("SEED".into()),
            ..Default::default()
        },
        ImportedEntry {
            kind: EntryKind::Card,
            title: "Visa".into(),
            card_number: Some("4111".into()),
            card_cvc: Some("123".into()),
            cardholder: Some("Neo".into()),
            ..Default::default()
        },
    ];
    let bytes = to_bitwarden_json(&entries).unwrap();
    let back = super::bitwarden::Bitwarden.parse(&bytes);
    assert!(back.errors.is_empty());
    assert_eq!(back.entries.len(), 2);
    assert_eq!(back.entries[0].username.as_deref(), Some("neo"));
    assert_eq!(back.entries[1].card_number.as_deref(), Some("4111"));
}

fn passkey() -> ImportedPasskey {
    ImportedPasskey {
        credential_id: "Y3JlZDE".into(),
        rp_id: "acme.test".into(),
        rp_name: Some("Acme".into()),
        user_handle: "dWgx".into(),
        user_name: "alice".into(),
        user_display_name: "Alice".into(),
        private_key: "cGsx".into(),
        counter: 42,
        created_at: Some("2024-01-01T00:00:00.000Z".into()),
    }
}

fn login_with_passkeys() -> Vec<ImportedEntry> {
    vec![ImportedEntry {
        kind: EntryKind::Login,
        title: "Acme".into(),
        username: Some("alice".into()),
        passkeys: vec![passkey()],
        ..Default::default()
    }]
}

#[test]
fn bitwarden_export_writes_fido2_credentials_with_the_supported_constants() {
    let bytes = to_bitwarden_json(&login_with_passkeys()).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let cred = &out["items"][0]["login"]["fido2Credentials"][0];
    assert_eq!(cred["keyType"], "public-key");
    assert_eq!(cred["keyAlgorithm"], "ECDSA");
    assert_eq!(cred["keyCurve"], "P-256");
    assert_eq!(cred["keyValue"], "cGsx");
    assert_eq!(cred["counter"], "42"); // a string, as Bitwarden writes it
    assert_eq!(cred["discoverable"], "true");
    assert_eq!(cred["creationDate"], "2024-01-01T00:00:00.000Z");
}

// No passkeys means no key at all, so a pre-passkey export is byte-identical.
#[test]
fn bitwarden_export_omits_fido2_credentials_without_passkeys() {
    let entries = vec![ImportedEntry {
        kind: EntryKind::Login,
        title: "Acme".into(),
        ..Default::default()
    }];
    let bytes = to_bitwarden_json(&entries).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(out["items"][0]["login"].get("fido2Credentials").is_none());
    assert!(!String::from_utf8(bytes).unwrap().contains("fido2"));
}

#[test]
fn round_trip_bitwarden_passkeys() {
    let entries = login_with_passkeys();
    let bytes = to_bitwarden_json(&entries).unwrap();
    let back = super::bitwarden::Bitwarden.parse(&bytes);
    assert!(back.errors.is_empty());
    assert_eq!(back.entries[0].passkeys, entries[0].passkeys);
}

// A CXF item carries a list of credentials, so all five types can share one
// item. `creationAt` (Unix seconds) dates the passkeys on it.
#[test]
fn cxf_maps_every_credential_type_on_one_item() {
    let json = br#"{
      "version": {"major": 1, "minor": 0},
      "exporterRpId": "example.com", "exporterDisplayName": "Example", "timestamp": 1700000000,
      "accounts": [{"id": "YWNjdA", "username": "", "email": "", "collections": [], "items": [
        {"id": "aXRlbTE", "creationAt": 1700000000, "modifiedAt": 1700000000,
         "title": "GitHub", "favorite": false, "tags": ["Work"],
         "scope": {"urls": ["https://github.com"], "androidApps": []},
         "credentials": [
           {"type": "basic-auth",
            "username": {"fieldType": "string", "value": "octo"},
            "password": {"fieldType": "concealed-string", "value": "pw"}},
           {"type": "passkey", "credentialId": "Y3JlZDE", "rpId": "github.com",
            "username": "octo", "userDisplayName": "Octo", "userHandle": "dWgx",
            "key": "cGsx", "fido2Extensions": {}},
           {"type": "totp", "secret": "SEED", "period": 30, "digits": 6,
            "algorithm": "sha1", "issuer": "GitHub", "username": "octo"},
           {"type": "note", "content": {"fieldType": "string", "value": "body"}},
           {"type": "future-thing", "whatever": 1}
         ]},
        {"id": "aXRlbTI", "title": "Visa", "credentials": [
           {"type": "credit-card",
            "number": {"fieldType": "concealed-string", "value": "4111111111111111"},
            "fullName": {"fieldType": "string", "value": "A B"},
            "cardType": {"fieldType": "string", "value": "Visa"},
            "verificationNumber": {"fieldType": "concealed-string", "value": "123"},
            "expiryDate": {"fieldType": "year-month", "value": "2030-01"},
            "pin": {"fieldType": "concealed-string", "value": "1234"}}
         ]}
      ]}]
    }"#;
    let r = parse(Format::Cxf, json);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(r.entries.len(), 2);

    let login = &r.entries[0];
    assert_eq!(login.kind, EntryKind::Login);
    assert_eq!(login.title, "GitHub");
    assert_eq!(login.username.as_deref(), Some("octo"));
    assert_eq!(login.password.as_deref(), Some("pw"));
    assert_eq!(login.url.as_deref(), Some("https://github.com"));
    assert_eq!(login.otp.as_deref(), Some("SEED"));
    assert_eq!(login.notes.as_deref(), Some("body"));
    assert_eq!(login.tags, vec!["Work".to_string()]);
    assert_eq!(
        login.passkeys,
        vec![ImportedPasskey {
            credential_id: "Y3JlZDE".into(),
            rp_id: "github.com".into(),
            rp_name: None,
            user_handle: "dWgx".into(),
            user_name: "octo".into(),
            user_display_name: "Octo".into(),
            private_key: "cGsx".into(),
            counter: 0,
            // 1700000000 seconds, as the item dates it.
            created_at: Some("2023-11-14T22:13:20+00:00".into()),
        }]
    );

    let card = &r.entries[1];
    assert_eq!(card.kind, EntryKind::Card);
    assert_eq!(card.card_number.as_deref(), Some("4111111111111111"));
    assert_eq!(card.cardholder.as_deref(), Some("A B"));
    assert_eq!(card.card_month.as_deref(), Some("01"));
    assert_eq!(card.card_year.as_deref(), Some("2030"));
    assert_eq!(card.card_cvc.as_deref(), Some("123"));
}

// Nothing but passkeys: still a login, named after the credential since there
// is no basic-auth to name it.
#[test]
fn cxf_passkey_only_item_becomes_a_passkey_only_login() {
    let json = br#"{"version":{"major":1,"minor":0},"accounts":[{"items":[
      {"id":"aQ","title":"Acme","credentials":[
        {"type":"passkey","credentialId":"Y3JlZDE","rpId":"acme.test","username":"alice",
         "userDisplayName":"Alice","userHandle":"dWgx","key":"cGsx"}
      ]}
    ]}]}"#;
    let r = parse(Format::Cxf, json);
    assert!(r.errors.is_empty());
    let e = &r.entries[0];
    assert_eq!(e.kind, EntryKind::Login);
    assert_eq!(e.username.as_deref(), Some("alice"));
    assert_eq!(e.password, None);
    assert_eq!(e.passkeys.len(), 1);
    assert_eq!(e.passkeys[0].created_at, None); // no creationAt on the item
}

// An item whose only credential is a note is a note; an item with nothing we
// can map — an unknown type, or an `ssh-key` with no key in it — is a row
// error, numbered across accounts.
#[test]
fn cxf_maps_note_only_items_and_flags_unmappable_ones() {
    let json = br#"{"version":{"major":1,"minor":0},"accounts":[
      {"items":[{"id":"aQ","title":"Secret","credentials":[
        {"type":"note","content":{"fieldType":"string","value":"body"}}]}]},
      {"items":[{"id":"aQ","title":"Mystery","credentials":[{"type":"wifi"},{"type":"ssh-key"}]}]}
    ]}"#;
    let r = parse(Format::Cxf, json);
    assert_eq!(r.entries.len(), 1);
    assert_eq!(r.entries[0].kind, EntryKind::Note);
    assert_eq!(r.entries[0].notes.as_deref(), Some("body"));
    assert_eq!(r.errors.len(), 1);
    assert_eq!(r.errors[0].row, 2); // second item overall
}

// A passkey without an id, relying party or key cannot sign; it is dropped on
// its own and the item still imports.
#[test]
fn cxf_skips_a_bad_passkey_but_imports_the_item() {
    let json = br#"{"version":{"major":1,"minor":0},"accounts":[{"items":[
      {"id":"aQ","title":"Acme","credentials":[
        {"type":"basic-auth","username":{"fieldType":"string","value":"alice"}},
        {"type":"passkey","rpId":"acme.test","username":"alice","key":"cGsx"},
        {"type":"passkey","credentialId":"Y3JlZDI","rpId":"acme.test","key":"cGsy"}
      ]}
    ]}]}"#;
    let r = parse(Format::Cxf, json);
    assert_eq!(r.entries.len(), 1);
    assert_eq!(r.entries[0].username.as_deref(), Some("alice"));
    assert_eq!(r.entries[0].passkeys.len(), 1);
    assert_eq!(r.entries[0].passkeys[0].credential_id, "Y3JlZDI");
    assert_eq!(r.errors.len(), 1);
    assert_eq!(r.errors[0].row, 1);
    assert!(r.errors[0].message.contains("passkey"));
}

// `creationAt` reaches us from JavaScript exporters as `1.5e9` — a whole number
// in exponent form. Reading it is the point; failing the document over it is not.
#[test]
fn cxf_reads_a_float_creation_date() {
    let json = br#"{"version":{"major":1,"minor":0},"accounts":[{"items":[
      {"id":"aQ","title":"Acme","creationAt":1.5e9,"credentials":[
        {"type":"passkey","credentialId":"Y3JlZDE","rpId":"acme.test","key":"cGsx"}
      ]}
    ]}]}"#;
    let r = parse(Format::Cxf, json);
    assert!(r.errors.is_empty());
    assert_eq!(r.entries.len(), 1);
    assert!(r.entries[0].passkeys[0]
        .created_at
        .as_deref()
        .is_some_and(|d| d.starts_with("2017-")));
}

// Older drafts wrote bare strings where 1.0 wraps values in an EditableField,
// and kept `urls` on the item rather than under `scope`.
#[test]
fn cxf_accepts_bare_field_values_and_item_level_urls() {
    let json = br#"{"version":{"major":1,"minor":0},"accounts":[{"items":[
      {"id":"aQ","title":"Acme","urls":["https://acme.test"],"credentials":[
        {"type":"basic-auth","username":"alice","password":"pw"}
      ]}
    ]}]}"#;
    let r = parse(Format::Cxf, json);
    assert!(r.errors.is_empty());
    assert_eq!(r.entries[0].username.as_deref(), Some("alice"));
    assert_eq!(r.entries[0].password.as_deref(), Some("pw"));
    assert_eq!(r.entries[0].url.as_deref(), Some("https://acme.test"));
}

#[test]
fn cxf_export_writes_a_version_1_0_document() {
    let bytes = to_cxf_json(&login_with_passkeys()).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(out["version"]["major"], 1);
    assert_eq!(out["version"]["minor"], 0);
    assert_eq!(out["exporterDisplayName"], crate::app::APP_NAME);
    assert_eq!(out["exporterRpId"], super::export::EXPORTER_RP_ID);
    assert!(out["timestamp"].is_i64());

    let creds = &out["accounts"][0]["items"][0]["credentials"];
    assert_eq!(creds[0]["type"], "basic-auth");
    // An absent value is an absent member, not an empty one.
    assert!(creds[0].get("password").is_none());
    assert_eq!(creds[1]["type"], "passkey");
    assert_eq!(creds[1]["credentialId"], "Y3JlZDE");
    assert_eq!(creds[1]["key"], "cGsx");
    assert_eq!(creds[1]["userHandle"], "dWgx");
}

// Export -> import round-trips everything CXF can carry. `counter` and
// `rp_name` have no home in the format, so they come back at their defaults.
#[test]
fn round_trip_cxf() {
    let entries = vec![
        ImportedEntry {
            kind: EntryKind::Login,
            title: "Acme".into(),
            username: Some("neo".into()),
            password: Some("trinity".into()),
            url: Some("https://acme.test".into()),
            notes: Some("hi".into()),
            otp: Some("SEED".into()),
            tags: vec!["Work".into()],
            passkeys: vec![passkey()],
            // CXF dates the item, and the passkeys on it with it — so the entry
            // needs a date of its own for there to be one to inherit.
            created_at: Some("2024-01-02T03:04:05+00:00".into()),
            ..Default::default()
        },
        ImportedEntry {
            kind: EntryKind::Card,
            title: "Visa".into(),
            notes: Some("spare".into()),
            card_number: Some("4111".into()),
            card_month: Some("01".into()),
            card_year: Some("2030".into()),
            card_cvc: Some("123".into()),
            cardholder: Some("Neo".into()),
            ..Default::default()
        },
        ImportedEntry {
            kind: EntryKind::Note,
            title: "Secret".into(),
            notes: Some("body".into()),
            ..Default::default()
        },
    ];
    let bytes = to_cxf_json(&entries).unwrap();
    let back = parse(Format::Cxf, &bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries.len(), 3);

    let expected: Vec<ImportedEntry> = entries
        .iter()
        .cloned()
        .map(|mut e| {
            for p in &mut e.passkeys {
                p.rp_name = None;
                p.counter = 0;
                // The passkey takes the item's date, which is the entry's.
                p.created_at = None;
            }
            e
        })
        .collect();
    let actual: Vec<ImportedEntry> = back
        .entries
        .into_iter()
        .map(|mut e| {
            for p in &mut e.passkeys {
                assert!(p.created_at.is_some());
                p.created_at = None;
            }
            e
        })
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn from_name_resolves_cxf() {
    assert_eq!(Format::from_name("cxf"), Some(Format::Cxf));
    assert_eq!(Format::from_name("CXF"), Some(Format::Cxf));
    assert_eq!(Format::from_name("fido"), Some(Format::Cxf));
    assert_eq!(Format::from_name("nope"), None);
}

// Both formats are JSON objects, so the sniff has to look inside: CXF declares
// a `version` object and an `accounts` array, Bitwarden neither.
#[test]
fn detect_tells_cxf_from_bitwarden_json() {
    let cxf = br#"{"version":{"major":1,"minor":0},"accounts":[]}"#;
    assert_eq!(detect("export.json", cxf), Some(Format::Cxf));
    assert_eq!(detect("noext", cxf), Some(Format::Cxf));
    assert_eq!(
        detect("bw.json", br#"{"encrypted":false,"items":[]}"#),
        Some(Format::Bitwarden)
    );
    // `accounts` alone (or an unparseable file) is not enough.
    assert_eq!(
        detect("x.json", br#"{"accounts":[]}"#),
        Some(Format::Bitwarden)
    );
    assert_eq!(detect("broken.json", b"{not json"), Some(Format::Bitwarden));
}

#[test]
fn detect_by_extension_and_content() {
    assert_eq!(detect("export.json", b"{}"), Some(Format::Bitwarden));
    assert_eq!(
        detect(
            "lp.csv",
            b"url,username,password,totp,extra,name,grouping,fav\n"
        ),
        Some(Format::LastpassCsv)
    );
    assert_eq!(
        detect("kp.csv", b"Group,Title,Username,Password,URL\n"),
        Some(Format::KeepassCsv)
    );
    assert_eq!(
        detect("x.csv", b"name,url,username,password\n"),
        Some(Format::GenericCsv)
    );
    assert_eq!(
        detect("noext", b"  {\"items\":[]}"),
        Some(Format::Bitwarden)
    );
}

fn ssh_entry() -> ImportedEntry {
    ImportedEntry {
        kind: EntryKind::Ssh,
        title: "Deploy key".into(),
        notes: Some("prod".into()),
        tags: vec!["infra".into()],
        ssh_private_key: Some(
            "-----BEGIN OPENSSH PRIVATE KEY-----\nc2VjcmV0\n-----END OPENSSH PRIVATE KEY-----\n"
                .into(),
        ),
        ssh_public_key: Some("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI alice@laptop".into()),
        ssh_fingerprint: Some("SHA256:abc".into()),
        ssh_passphrase: Some("hunter2".into()),
        ..Default::default()
    }
}

// An SSH key is Bitwarden's own item type 5, so it comes back as the same key.
// The passphrase has no member there and rides as a hidden custom field — and
// is taken back out of the extras on the way in.
#[test]
fn round_trip_bitwarden_ssh_key() {
    let entries = vec![ssh_entry()];
    let bytes = to_bitwarden_json(&entries).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(out["items"][0]["type"], 5);
    assert_eq!(out["items"][0]["sshKey"]["keyFingerprint"], "SHA256:abc");
    assert_eq!(out["items"][0]["fields"][0]["name"], "Passphrase");
    assert_eq!(out["items"][0]["fields"][0]["type"], 1);
    assert!(out["items"][0].get("login").is_none());

    let back = parse(Format::Bitwarden, &bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    let mut expected = ssh_entry();
    expected.tags.clear(); // Bitwarden items carry no tags
    assert_eq!(back.entries, vec![expected]);
}

// CXF's `ssh-key` carries the private key; the public line, fingerprint and
// passphrase travel in a custom-fields credential and are read back by label.
#[test]
fn round_trip_cxf_ssh_key() {
    let entries = vec![ssh_entry()];
    let bytes = to_cxf_json(&entries).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let creds = &out["accounts"][0]["items"][0]["credentials"];
    assert_eq!(creds[0]["type"], "ssh-key");
    assert_eq!(creds[0]["keyType"], "ssh-ed25519");
    assert_eq!(creds[0]["keyComment"]["value"], "alice@laptop");
    assert_eq!(creds[0]["privateKey"]["fieldType"], "concealed-string");
    assert_eq!(creds[1]["type"], "custom-fields");
    assert_eq!(creds[1]["fields"][2]["label"], "Passphrase");
    assert_eq!(creds[2]["type"], "note");

    let back = parse(Format::Cxf, &bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries, entries);
}

// A pasted key with no public line still exports: the key type falls back to
// the only algorithm the app generates, and no comment is written.
#[test]
fn cxf_ssh_key_without_public_line() {
    let entries = vec![ImportedEntry {
        ssh_public_key: None,
        ssh_fingerprint: None,
        ssh_passphrase: None,
        ..ssh_entry()
    }];
    let bytes = to_cxf_json(&entries).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let creds = &out["accounts"][0]["items"][0]["credentials"];
    assert_eq!(creds[0]["keyType"], "ssh-ed25519");
    assert!(creds[0].get("keyComment").is_none());
    // Nothing to put in custom fields, so there is no such credential.
    assert_eq!(creds[1]["type"], "note");
}

// The generic CSV carries every SSH field in a column of its own.
#[test]
fn generic_csv_carries_ssh_fields() {
    let out = String::from_utf8(to_generic_csv(&[ssh_entry()]).unwrap()).unwrap();
    let mut lines = out.lines();
    let header = lines.next().unwrap();
    assert!(header.contains("ssh_private_key,ssh_public_key,ssh_fingerprint,ssh_passphrase"));
    let row = lines.collect::<Vec<_>>().join("\n");
    assert!(row.starts_with("ssh,Deploy key,"));
    assert!(row.contains("BEGIN OPENSSH PRIVATE KEY"));
    assert!(row.contains("SHA256:abc"));
    assert!(row.contains("hunter2"));
}

fn apikey_entry() -> ImportedEntry {
    ImportedEntry {
        kind: EntryKind::ApiKey,
        title: "Coupler.io".into(),
        notes: Some("billing".into()),
        url: Some("https://api.coupler.io/v1".into()),
        api_key: Some("cpl_live_4f9a0b3c".into()),
        api_environment: Some("production".into()),
        api_scopes: Some("read:data write:data".into()),
        api_expires: Some("2027-01-01".into()),
        ..Default::default()
    }
}

// The generic CSV is our own, so every API key field has a column and the row
// comes back as the key it was — told apart from a login by the `type` column.
#[test]
fn round_trip_generic_csv_apikey() {
    let bytes = to_generic_csv(&[apikey_entry()]).unwrap();
    let out = String::from_utf8(bytes.clone()).unwrap();
    let header = out.lines().next().unwrap();
    assert!(header.contains("ssh_passphrase,api_key,environment,scopes,expires,body"));
    assert!(out.contains("\napikey,Coupler.io,"));
    assert!(out.contains("cpl_live_4f9a0b3c"));

    let back = super::csv::GenericCsv.parse(&bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries, vec![apikey_entry()]);
}

// CXF's `api-key` carries the token, the URL and the expiry; environment and
// scopes travel in a custom-fields credential and are read back by label.
#[test]
fn round_trip_cxf_api_key() {
    let entries = vec![apikey_entry()];
    let bytes = to_cxf_json(&entries).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let creds = &out["accounts"][0]["items"][0]["credentials"];
    assert_eq!(creds[0]["type"], "api-key");
    assert_eq!(creds[0]["key"]["fieldType"], "concealed-string");
    assert_eq!(creds[0]["key"]["value"], "cpl_live_4f9a0b3c");
    assert_eq!(creds[0]["url"]["value"], "https://api.coupler.io/v1");
    assert_eq!(creds[0]["expiryDate"]["fieldType"], "date");
    assert_eq!(creds[0]["expiryDate"]["value"], "2027-01-01");
    assert_eq!(creds[1]["type"], "custom-fields");
    assert_eq!(creds[1]["fields"][0]["label"], "Environment");
    assert_eq!(creds[1]["fields"][1]["label"], "Scopes");
    assert_eq!(creds[2]["type"], "note");

    let back = parse(Format::Cxf, &bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries, entries);
}

// A token with nothing else known about it is still a key: no URL, no expiry,
// and no custom-fields credential to carry nothing in.
#[test]
fn cxf_api_key_alone() {
    let entries = vec![ImportedEntry {
        url: None,
        api_environment: None,
        api_scopes: None,
        api_expires: None,
        notes: None,
        ..apikey_entry()
    }];
    let bytes = to_cxf_json(&entries).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let creds = &out["accounts"][0]["items"][0]["credentials"];
    assert_eq!(creds.as_array().unwrap().len(), 1);
    assert!(creds[0].get("url").is_none());
    assert!(creds[0].get("expiryDate").is_none());

    let back = parse(Format::Cxf, &bytes);
    assert_eq!(back.entries, entries);
}

// An environment the switch has no segment for is neither dropped nor hidden:
// it comes back as a custom field, in sight. A known one is taken in any case.
#[test]
fn imported_environment_lands_on_the_switch_or_in_the_extras() {
    let mut staging = apikey_entry();
    staging.api_environment = Some("staging".into());
    let back = parse(Format::Cxf, &to_cxf_json(&[staging]).unwrap());
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries[0].api_environment, None);
    assert_eq!(
        back.entries[0].extra,
        vec![("Environment".to_string(), "staging".to_string())]
    );

    let bytes = b"type,title,api_key,environment\napikey,Coupler.io,cpl_live_abc,Production\n";
    let back = super::csv::GenericCsv.parse(bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(
        back.entries[0].api_environment.as_deref(),
        Some("production")
    );
    assert!(back.entries[0].extra.is_empty());
}

// Bitwarden has no item for an API key, so one goes out as a login with the
// token for a password and the base URL for a site; the rest ride as custom
// fields. One-way: it comes back as the login it looks like, token intact.
#[test]
fn bitwarden_exports_apikey_as_a_login() {
    let bytes = to_bitwarden_json(&[apikey_entry()]).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let item = &out["items"][0];
    assert_eq!(item["type"], 1);
    assert_eq!(item["login"]["password"], "cpl_live_4f9a0b3c");
    assert_eq!(item["login"]["uris"][0]["uri"], "https://api.coupler.io/v1");
    assert_eq!(item["fields"][0]["name"], "Environment");
    assert_eq!(item["fields"][0]["value"], "production");
    assert_eq!(item["fields"][1]["name"], "Scopes");
    assert_eq!(item["fields"][2]["name"], "Expires");
    assert_eq!(item["fields"][2]["value"], "2027-01-01");

    let back = parse(Format::Bitwarden, &bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries[0].kind, EntryKind::Login);
    assert_eq!(
        back.entries[0].password.as_deref(),
        Some("cpl_live_4f9a0b3c")
    );
    assert_eq!(
        back.entries[0].url.as_deref(),
        Some("https://api.coupler.io/v1")
    );
}

// A file with everything a CSV cell could trip on: a comment, an indented line,
// a blank line, a quoted value spanning a newline, one CRLF line ending, and a
// trailing newline that trimming would eat.
const ENV_BODY: &str =
    "# api\n  export API_KEY='abc' # inline\n\nCERT=\"line one\nline two\"\r\nURL=${HOST}/v1\n";

fn env_entry() -> ImportedEntry {
    ImportedEntry {
        kind: EntryKind::Env,
        title: "api · production".into(),
        notes: Some("rotated quarterly".into()),
        env_body: Some(ENV_BODY.into()),
        env_file_name: Some(".env.production".into()),
        ..Default::default()
    }
}

// The generic CSV is our own, so the file goes in a column of its own and comes
// back byte for byte — the writer quotes a cell with newlines in it, and the
// reader gives it back untrimmed.
#[test]
fn round_trip_generic_csv_env() {
    let bytes = to_generic_csv(&[env_entry()]).unwrap();
    let out = String::from_utf8(bytes.clone()).unwrap();
    let header = out.lines().next().unwrap();
    assert!(header.contains(",body,file_name,tags"));
    assert!(header.ends_with("_rowel_csv_version"));
    assert!(out.trim_end().ends_with(CSV_VERSION));
    assert!(out.contains("\nenv,api · production,"));
    assert!(
        out.contains("\"# api\n"),
        "body is quoted, not split across rows"
    );

    let back = super::csv::GenericCsv.parse(&bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries, vec![env_entry()]);
    assert_eq!(back.entries[0].env_body.as_deref(), Some(ENV_BODY));
}

// Formula-looking bodies are guarded and genuine apostrophes are escaped, so
// every ambiguous prefix round-trips under the versioned Rowel dialect.
#[test]
fn round_trip_generic_csv_env_with_a_formula_looking_first_line() {
    for body in [
        "=A\n",
        "+A\n",
        "-A\n",
        "@A\n",
        "\tA=1\n",
        "\rA=1\n",
        "'=A\n",
        "'+A\n",
        "'-A\n",
        "'@A\n",
        "'\tA\n",
        "'\rA\n",
        "''quoted\n",
    ] {
        let mut e = env_entry();
        e.env_body = Some(body.into());
        let bytes = to_generic_csv(&[e.clone()]).unwrap();
        let back = super::csv::GenericCsv.parse(&bytes);
        assert_eq!(back.entries, vec![e], "{body:?}");
    }
}

// A generic sheet has no provenance marker, so an apostrophe that resembles
// Rowel's spreadsheet guard remains literal rather than being guessed away.
#[test]
fn unversioned_generic_csv_keeps_a_literal_apostrophe_in_an_env_body() {
    let bytes = b"type,title,body,file_name\nenv,Literal,\"'=VALUE\nA=1\",.env\n";
    let back = super::csv::GenericCsv.parse(bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries[0].env_body.as_deref(), Some("'=VALUE\nA=1"));
}

// Bitwarden has no item for a file, so an env entry goes out as a secure note
// whose text is the file; the name and the entry's own note ride as custom
// fields. One-way: it comes back as the note it looks like.
#[test]
fn bitwarden_exports_env_as_a_secure_note() {
    let bytes = to_bitwarden_json(&[env_entry()]).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let item = &out["items"][0];
    assert_eq!(item["type"], 2);
    assert_eq!(item["name"], "api · production");
    assert_eq!(item["notes"], ENV_BODY);
    assert!(item.get("login").is_none());
    assert_eq!(item["fields"][0]["name"], "Note");
    assert_eq!(item["fields"][0]["value"], "rotated quarterly");
    assert_eq!(item["fields"][1]["name"], "file_name");
    assert_eq!(item["fields"][1]["value"], ".env.production");
    assert_eq!(item["fields"][1]["type"], 0);

    let back = parse(Format::Bitwarden, &bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries[0].kind, EntryKind::Note);
    assert_eq!(back.entries[0].notes.as_deref(), Some(ENV_BODY));
}

// CXF likewise: the file is the item's first `note` credential, the name a
// custom field beside it, the entry's own note after. Read back, it is a note.
#[test]
fn cxf_exports_env_as_a_note() {
    let bytes = to_cxf_json(&[env_entry()]).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let creds = &out["accounts"][0]["items"][0]["credentials"];
    assert_eq!(creds[0]["type"], "note");
    assert_eq!(creds[0]["content"]["value"], ENV_BODY);
    assert_eq!(creds[1]["type"], "custom-fields");
    assert_eq!(creds[1]["fields"][0]["label"], "file_name");
    assert_eq!(creds[1]["fields"][0]["value"], ".env.production");
    assert_eq!(creds[2]["type"], "note");
    assert_eq!(creds[2]["content"]["value"], "rotated quarterly");
    assert!(creds.as_array().unwrap().len() == 3);

    let back = parse(Format::Cxf, &bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries[0].kind, EntryKind::Note);
    assert_eq!(back.entries[0].notes.as_deref(), Some(ENV_BODY));
}

// The bug this guards against: an unknown kind used to fall through to "login"
// and export as an empty one, taking the file with it.
#[test]
fn env_never_exports_as_a_login() {
    let entries = [env_entry()];
    let csv = String::from_utf8(to_generic_csv(&entries).unwrap()).unwrap();
    assert!(!csv.contains("\nlogin,"));

    let bw: serde_json::Value =
        serde_json::from_slice(&to_bitwarden_json(&entries).unwrap()).unwrap();
    assert_ne!(bw["items"][0]["type"], 1);

    let cxf: serde_json::Value = serde_json::from_slice(&to_cxf_json(&entries).unwrap()).unwrap();
    let kinds: Vec<&str> = cxf["accounts"][0]["items"][0]["credentials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["type"].as_str().unwrap())
        .collect();
    assert!(!kinds.contains(&"basic-auth"), "{kinds:?}");
}

// The bug this guards against: a seed's digits, period and algorithm were
// exported as the defaults and ignored on the way back in, so a re-imported
// 8-digit SHA-256 enrolment generated codes its site would refuse.
#[test]
fn cxf_round_trips_non_default_totp_parameters() {
    let stored = "otpauth://totp/?secret=JBSWY3DPEHPK3PXP&digits=8&period=60&algorithm=SHA256";
    let entries = [ImportedEntry {
        kind: EntryKind::Login,
        title: "Acme".into(),
        password: Some("pw".into()),
        otp: Some(stored.into()),
        ..Default::default()
    }];
    let bytes = to_cxf_json(&entries).unwrap();

    // CXF spells the parameters out beside the seed rather than in a URI.
    let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let totp = doc["accounts"][0]["items"][0]["credentials"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "totp")
        .unwrap()
        .clone();
    assert_eq!(totp["secret"], "JBSWY3DPEHPK3PXP");
    assert_eq!(totp["digits"], 8);
    assert_eq!(totp["period"], 60);
    assert_eq!(totp["algorithm"], "sha256");

    let back = parse(Format::Cxf, &bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries[0].otp.as_deref(), Some(stored));
}

fn cxf_login_with_totp(totp_members: &str) -> Vec<u8> {
    format!(
        r#"{{"version":{{"major":1,"minor":0}},"accounts":[{{"items":[
          {{"id":"aQ","title":"Acme","credentials":[
            {{"type":"basic-auth","username":{{"fieldType":"string","value":"alice"}}}},
            {{"type":"totp","secret":"JBSWY3DPEHPK3PXP"{totp_members}}}
          ]}}
        ]}}]}}"#
    )
    .into_bytes()
}

// The bug this guards against: a `totp` whose `algorithm`, `period` or
// `digits` we could not generate for was imported with the defaults in their
// place — a seed that produces plausible codes the site refuses. CXF has the
// importer ignore a TOTP with an algorithm it does not know; an unreadable or
// out-of-range period or digit count is treated the same. The credential is
// skipped and reported, and the rest of the item still imports.
#[test]
fn cxf_skips_a_totp_with_parameters_it_cannot_generate_for() {
    for (what, member) in [
        ("algorithm", r#","algorithm":"md5""#),
        ("algorithm", r#","algorithm":1"#),
        ("period", r#","period":301"#),
        ("period", r#","period":"0""#),
        ("period", r#","period":true"#),
        ("digits", r#","digits":4"#),
        ("digits", r#","digits":"eight""#),
        ("digits", r#","digits":[8]"#),
    ] {
        let r = parse(Format::Cxf, &cxf_login_with_totp(member));
        assert_eq!(r.entries.len(), 1, "{member}");
        assert_eq!(r.entries[0].username.as_deref(), Some("alice"), "{member}");
        assert_eq!(r.entries[0].otp, None, "{member}");
        assert_eq!(r.errors.len(), 1, "{member}: {:?}", r.errors);
        assert_eq!(r.errors[0].row, 1);
        let message = &r.errors[0].message;
        assert!(message.contains("totp"), "{message}");
        assert!(message.contains(what), "{message}");
    }
}

// Absence is a different matter: the spec gives each parameter a default, and
// a seed with none spelled out is the bare seed. A readable string number is
// as good as a number.
#[test]
fn cxf_totp_parameters_default_only_when_absent() {
    let r = parse(Format::Cxf, &cxf_login_with_totp(""));
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(r.entries[0].otp.as_deref(), Some("JBSWY3DPEHPK3PXP"));

    let r = parse(
        Format::Cxf,
        &cxf_login_with_totp(r#","digits":"8","period":"60","algorithm":"SHA512""#),
    );
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(
        r.entries[0].otp.as_deref(),
        Some("otpauth://totp/?secret=JBSWY3DPEHPK3PXP&digits=8&period=60&algorithm=SHA512")
    );
}

// --- every field, every kind, through our own CSV ----------------------------

// What a Rowel row carries whatever its kind is. Every entry below starts from
// this, so a column that belongs to no kind is proven on all seven.
fn shared() -> ImportedEntry {
    ImportedEntry {
        notes: Some("what it is for".into()),
        tags: vec!["Work".into(), "2FA".into()],
        favorite: true,
        created_at: Some("2024-01-02T03:04:05+00:00".into()),
        updated_at: Some("2025-06-07T08:09:10+00:00".into()),
        ..Default::default()
    }
}

// One entry per kind, with every column that kind owns filled in.
fn one_of_every_kind() -> Vec<ImportedEntry> {
    vec![
        ImportedEntry {
            kind: EntryKind::Login,
            title: "Acme".into(),
            username: Some("neo".into()),
            password: Some("trinity".into()),
            url: Some("https://acme.test".into()),
            otp: Some("SEED".into()),
            email: Some("neo@acme.test".into()),
            password_updated_at: Some("2025-05-04T03:02:01+00:00".into()),
            ..shared()
        },
        ImportedEntry {
            kind: EntryKind::Note,
            title: "Wifi".into(),
            ..shared()
        },
        ImportedEntry {
            kind: EntryKind::Card,
            title: "Visa".into(),
            card_number: Some("4111111111111111".into()),
            card_month: Some("01".into()),
            card_year: Some("2030".into()),
            card_cvc: Some("123".into()),
            cardholder: Some("NEO ANDERSON".into()),
            card_pin: Some("4321".into()),
            ..shared()
        },
        ImportedEntry {
            kind: EntryKind::Identity,
            title: "Passport".into(),
            doc_type: Some("passport".into()),
            doc_number: Some("X1234567".into()),
            doc_country: Some("GBR".into()),
            holder_name: Some("ADA LOVELACE".into()),
            doc_nationality: Some("British".into()),
            doc_birth_date: Some("1815-12-10".into()),
            doc_sex: Some("F".into()),
            doc_issue_date: Some("2020-03-04".into()),
            doc_expiry_date: Some("2030-03-04".into()),
            doc_authority: Some("HMPO".into()),
            doc_personal_number: Some("PN-42".into()),
            ..shared()
        },
        ImportedEntry {
            kind: EntryKind::Ssh,
            title: "Deploy key".into(),
            // No trailing newline: every column but the file body is trimmed on
            // the way in, as it has always been.
            ssh_private_key: Some("-----BEGIN OPENSSH PRIVATE KEY-----\nc2VjcmV0".into()),
            ssh_public_key: Some("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5 alice@laptop".into()),
            ssh_fingerprint: Some("SHA256:abc".into()),
            ssh_passphrase: Some("hunter2".into()),
            ..shared()
        },
        ImportedEntry {
            kind: EntryKind::Env,
            title: "api · production".into(),
            env_body: Some(ENV_BODY.into()),
            env_file_name: Some(".env.production".into()),
            ..shared()
        },
        ImportedEntry {
            kind: EntryKind::ApiKey,
            title: "Coupler.io".into(),
            url: Some("https://api.coupler.io/v1".into()),
            api_key: Some("cpl_live_4f9a0b3c".into()),
            api_environment: Some("production".into()),
            api_scopes: Some("read:data write:data".into()),
            api_expires: Some("2027-01-01".into()),
            ..shared()
        },
    ]
}

// The acceptance test for the whole export: our own sheet names every row's
// kind and has a column for every field, so every kind comes back as itself
// with nothing dropped. A card used to come back as a login with no number.
#[test]
fn round_trip_generic_csv_every_kind() {
    let entries = one_of_every_kind();
    let bytes = to_generic_csv(&entries).unwrap();
    let back = super::csv::GenericCsv.parse(&bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries, entries);
}

// A row of ours naming a kind we do not have is reported rather than quietly
// filed as a login — guessing is only for a sheet that is not ours.
#[test]
fn generic_csv_rejects_an_unknown_kind_on_a_rowel_row() {
    let bytes = format!(
        "type,title,{}\nwidget,Thing,3\n",
        super::export::CSV_VERSION_HEADER
    );
    let back = super::csv::GenericCsv.parse(bytes.as_bytes());
    assert!(back.entries.is_empty());
    assert_eq!(back.errors.len(), 1);
    assert_eq!(back.errors[0].row, 2);
}

// Version 3 only appended columns, so a version-2 sheet — any marked sheet —
// still comes back as what it was, the columns it never had reading as absent.
#[test]
fn generic_csv_version_2_still_round_trips() {
    let bytes =
        b"type,title,username,password,url,notes,otp,body,file_name,tags,_rowel_csv_version\n\
login,Acme,neo,trinity,https://acme.test,hi,SEED,,,Work,2\n\
env,api,,,,rotated,,\"A=1\n\",.env,,2\n";
    let back = super::csv::GenericCsv.parse(bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries.len(), 2);

    let login = &back.entries[0];
    assert_eq!(login.kind, EntryKind::Login);
    assert_eq!(login.username.as_deref(), Some("neo"));
    assert_eq!(login.otp.as_deref(), Some("SEED"));
    assert_eq!(login.tags, vec!["Work".to_string()]);
    // Absent, not empty: version 2 had no column for any of these.
    assert_eq!(login.email, None);
    assert_eq!(login.created_at, None);
    assert!(!login.favorite);

    let env = &back.entries[1];
    assert_eq!(env.kind, EntryKind::Env);
    assert_eq!(env.env_body.as_deref(), Some("A=1\n"));
    assert_eq!(env.env_file_name.as_deref(), Some(".env"));
}

// --- the star, the dates and the labelled fields -----------------------------

fn starred_login() -> ImportedEntry {
    ImportedEntry {
        kind: EntryKind::Login,
        title: "Acme".into(),
        username: Some("neo".into()),
        password: Some("trinity".into()),
        email: Some("neo@acme.test".into()),
        favorite: true,
        created_at: Some("2024-01-02T03:04:05+00:00".into()),
        updated_at: Some("2025-06-07T08:09:10+00:00".into()),
        ..Default::default()
    }
}

// Bitwarden has members for the star and both dates; the email has none there,
// so it rides as a labelled custom field and is taken back out of the extras.
#[test]
fn round_trip_bitwarden_favorite_dates_and_a_labelled_field() {
    let entries = vec![starred_login()];
    let bytes = to_bitwarden_json(&entries).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let item = &out["items"][0];
    assert_eq!(item["favorite"], true);
    assert_eq!(item["creationDate"], "2024-01-02T03:04:05+00:00");
    assert_eq!(item["revisionDate"], "2025-06-07T08:09:10+00:00");
    assert_eq!(item["fields"][0]["name"], "Email");
    assert_eq!(item["fields"][0]["value"], "neo@acme.test");

    let back = parse(Format::Bitwarden, &bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries, entries);
    // The label was ours, so it did not also stay on as a custom field.
    assert!(back.entries[0].extra.is_empty());
}

// A user may well have named a custom field "Email" themselves. Ours is written
// first, so the importer's first match is ours and theirs stays a custom field.
#[test]
fn a_users_own_field_sharing_our_label_survives_a_bitwarden_round_trip() {
    let mut login = starred_login();
    login.extra = vec![("Email".into(), "the user's own note".into())];
    let entries = vec![login];
    let bytes = to_bitwarden_json(&entries).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(out["items"][0]["fields"][0]["value"], "neo@acme.test");
    assert_eq!(out["items"][0]["fields"][1]["value"], "the user's own note");

    let back = parse(Format::Bitwarden, &bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries, entries);
}

// The same, when we have nothing of our own to write under the label: with the
// email unset and the star off, the user's "Email" and "Favorite" fields used to
// be the first — and so the only — match, and were moved into our slots. An
// empty field of ours now goes ahead of each, so theirs come back as theirs.
#[test]
fn a_users_own_field_sharing_a_label_we_did_not_write_survives_a_bitwarden_round_trip() {
    let mut login = starred_login();
    login.email = None;
    login.favorite = false;
    login.extra = vec![
        ("Email".into(), "the user's own note".into()),
        ("favorite".into(), "yes, very".into()),
    ];
    let entries = vec![login];
    let bytes = to_bitwarden_json(&entries).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let fields: Vec<(&str, &str)> = out["items"][0]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["name"].as_str().unwrap(), f["value"].as_str().unwrap()))
        .collect();
    assert_eq!(
        fields,
        vec![
            ("Favorite", ""),
            ("Email", ""),
            ("Email", "the user's own note"),
            ("favorite", "yes, very"),
        ]
    );

    let back = parse(Format::Bitwarden, &bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries, entries);
}

// A CXF `modifiedAt` written as a float or a string is read like `creationAt`
// is, rather than failing the document.
#[test]
fn cxf_reads_a_lenient_modified_at() {
    let json = br#"{"version":{"major":1,"minor":0},"accounts":[{"items":[
      {"id":"aQ","title":"Acme","creationAt":1.7e9,"modifiedAt":"1700000001","credentials":[
        {"type":"basic-auth","username":"neo","password":"trinity"}
      ]}
    ]}]}"#;
    let back = parse(Format::Cxf, json);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(
        back.entries[0].updated_at.as_deref(),
        Some("2023-11-14T22:13:21+00:00")
    );
}

// The rest of a document: Bitwarden's `ssn` carries the personal number and the
// dates, authority, nationality and sex travel as labelled fields.
#[test]
fn round_trip_bitwarden_whole_identity() {
    let entries = vec![ImportedEntry {
        kind: EntryKind::Identity,
        title: "Passport".into(),
        doc_type: Some("passport".into()),
        doc_number: Some("X1234567".into()),
        doc_country: Some("GBR".into()),
        holder_name: Some("ADA LOVELACE".into()),
        doc_nationality: Some("British".into()),
        doc_birth_date: Some("1815-12-10".into()),
        doc_sex: Some("F".into()),
        doc_issue_date: Some("2020-03-04".into()),
        doc_expiry_date: Some("2030-03-04".into()),
        doc_authority: Some("HMPO".into()),
        doc_personal_number: Some("PN-42".into()),
        ..Default::default()
    }];
    let bytes = to_bitwarden_json(&entries).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(out["items"][0]["identity"]["ssn"], "PN-42");

    let back = parse(Format::Bitwarden, &bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries, entries);
}

// CXF dates the item itself, in Unix seconds; the star and the email have no
// member there and travel in the same kind of custom-fields credential the SSH
// key's extra parts use.
#[test]
fn round_trip_cxf_favorite_dates_and_a_labelled_field() {
    let entries = vec![starred_login()];
    let bytes = to_cxf_json(&entries).unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let item = &out["accounts"][0]["items"][0];
    assert_eq!(item["creationAt"], 1_704_164_645_i64);
    assert_eq!(item["modifiedAt"], 1_749_283_750_i64);
    let labels: Vec<&str> = item["credentials"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["type"] == "custom-fields")
        .flat_map(|c| c["fields"].as_array().unwrap())
        .map(|f| f["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, vec!["Email", "Favorite"]);

    let back = parse(Format::Cxf, &bytes);
    assert!(back.errors.is_empty(), "{:?}", back.errors);
    assert_eq!(back.entries, entries);
}
