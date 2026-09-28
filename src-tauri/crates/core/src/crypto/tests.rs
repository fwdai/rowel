//! Compatibility tests. Golden fixtures are produced by the independent Node
//! reference implementation (`scripts/crypto-ref.mjs gen-fixtures`); these tests
//! prove the Rust cryptor reads exactly what Node wrote.

use super::*;
use crate::models::VaultData;
use serde_json::Value;

const FIXTURES: &str = include_str!("../../../../tests/fixtures/fixtures.json");

fn fixtures() -> Value {
    serde_json::from_str(FIXTURES).unwrap()
}

#[test]
fn hash_secret_matches_reference() {
    for case in fixtures()["hashSecret"].as_array().unwrap() {
        let pw = case["password"].as_str().unwrap();
        let expected = case["secret"].as_str().unwrap();
        assert_eq!(hash_secret(pw), expected);
    }
}

#[test]
fn decrypts_golden_fields() {
    let f = fixtures();
    let cryptor = Cryptor::new(f["secret"].as_str().unwrap());
    for case in f["encrypt"].as_array().unwrap() {
        let hex = case["hex"].as_str().unwrap();
        let plaintext = case["plaintext"].as_str().unwrap();
        assert_eq!(cryptor.decrypt(hex).unwrap(), plaintext, "{}", case["name"]);
    }
}

#[test]
fn decrypts_golden_data_blobs() {
    let f = fixtures();
    let cryptor = Cryptor::new(f["secret"].as_str().unwrap());
    for case in f["encryptData"].as_array().unwrap() {
        let blob = case["blob"].as_str().unwrap();
        let value: Value = cryptor.decrypt_data(blob).unwrap();
        assert_eq!(value, case["value"], "{}", case["name"]);
    }
}

#[test]
fn decrypts_and_exposes_golden_vault() {
    let f = fixtures();
    let cryptor = Cryptor::new(f["secret"].as_str().unwrap());
    let vault = &f["vault"];

    let data: VaultData = cryptor
        .decrypt_data(vault["blob"].as_str().unwrap())
        .unwrap();
    let expected = vault["exposed"].as_array().unwrap();
    assert_eq!(data.entries.len(), expected.len());

    for (entry, want) in data.entries.iter().zip(expected) {
        let exposed = cryptor.expose(entry).unwrap();
        let got = serde_json::to_value(&exposed).unwrap();
        assert_eq!(&got, want, "entry {}", entry.id);
    }
}

#[test]
fn field_round_trip() {
    let cryptor = Cryptor::new(&hash_secret("master-pw"));
    for value in ["", "simple", "unicode 密码 🔑", &"x".repeat(9000)] {
        let hex = cryptor.encrypt(value).unwrap();
        assert_eq!(cryptor.decrypt(&hex).unwrap(), value);
    }
}

#[test]
fn data_round_trip() {
    let cryptor = Cryptor::new(&hash_secret("master-pw"));
    let value = serde_json::json!({ "entries": [], "n": 42, "s": "два" });
    let blob = cryptor.encrypt_data(&value).unwrap();
    let back: Value = cryptor.decrypt_data(&blob).unwrap();
    assert_eq!(back, value);
}

// The owned path is what an import takes: it consumes the blob and unseals in
// place, so it has to read exactly what the borrowed one reads — and still
// refuse a ciphertext that was edited under it, since decrypting in place is
// the easy way to hand back unauthenticated bytes.
#[test]
fn owned_data_round_trip_rejects_a_tampered_ciphertext() {
    let cryptor = Cryptor::new(&hash_secret("master-pw"));
    let value = serde_json::json!({ "entries": [], "n": 42, "s": "два" });
    let blob = cryptor.encrypt_data(&value).unwrap();

    let back: Value = cryptor.decrypt_data_owned(blob.clone()).unwrap();
    assert_eq!(back, value);

    // Flip the last ciphertext byte, well past `salt ‖ iv ‖ tag`.
    let mut bytes = hex::decode(STANDARD.decode(&blob).unwrap()).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    let tampered = STANDARD.encode(hex::encode(&bytes));

    assert!(cryptor
        .decrypt_data_owned::<Value>(tampered.clone())
        .is_err());
    assert!(cryptor.decrypt_data::<Value>(&tampered).is_err());
}

#[test]
fn obscure_expose_round_trip() {
    let cryptor = Cryptor::new(&hash_secret("master-pw"));
    let entry: Entry = serde_json::from_value(serde_json::json!({
        "id": "1", "type": "login", "title": "T",
        "password": "hunter2", "otp": "SEED"
    }))
    .unwrap();

    let obscured = cryptor.obscure(&entry).unwrap();
    // Sensitive fields become hex, not the plaintext.
    assert_ne!(obscured.password.as_deref(), Some("hunter2"));
    assert!(obscured.otp.as_ref().unwrap().len() > 32);

    let exposed = cryptor.expose(&obscured).unwrap();
    assert_eq!(exposed.password.as_deref(), Some("hunter2"));
    assert_eq!(exposed.otp.as_deref(), Some("SEED"));
}

// An identity's document number and personal number are its secrets; the rest
// of the document (dates, authority, country) is not.
#[test]
fn identity_obscures_only_its_numbers() {
    let cryptor = Cryptor::new(&hash_secret("master-pw"));
    let entry: Entry = serde_json::from_value(serde_json::json!({
        "id": "1", "type": "identity", "title": "Passport",
        "doc_type": "passport", "name": "ADA LOVELACE", "number": "X1234567",
        "personal_number": "99-1815", "country": "GBR", "expiry_date": "2035-06-01"
    }))
    .unwrap();

    let obscured = cryptor.obscure(&entry).unwrap();
    assert_ne!(obscured.number.as_deref(), Some("X1234567"));
    assert!(obscured.personal_number.as_ref().unwrap().len() > 32);
    assert_eq!(obscured.name.as_deref(), Some("ADA LOVELACE"));
    assert_eq!(obscured.expiry_date.as_deref(), Some("2035-06-01"));

    let exposed = cryptor.expose(&obscured).unwrap();
    assert_eq!(exposed.number.as_deref(), Some("X1234567"));
    assert_eq!(exposed.personal_number.as_deref(), Some("99-1815"));
}

// Re-encrypting only what changed has to visit the identity slots in the same
// order `transform` does, or a saved edit swaps the two numbers' ciphertexts.
#[test]
fn identity_keeps_unchanged_ciphertext_on_save() {
    let cryptor = Cryptor::new(&hash_secret("master-pw"));
    let plain: Entry = serde_json::from_value(serde_json::json!({
        "id": "1", "type": "identity", "title": "Passport",
        "number": "X1234567", "personal_number": "99-1815"
    }))
    .unwrap();
    let stored = cryptor.obscure(&plain).unwrap();

    // The editor sends plaintext for what it changed and the stored ciphertext
    // for what it did not.
    let mut next = stored.clone();
    next.personal_number = Some("00-2024".into());
    let saved = cryptor.obscure_changed(&next, Some(&stored)).unwrap();

    assert_eq!(saved.number, stored.number);
    let exposed = cryptor.expose(&saved).unwrap();
    assert_eq!(exposed.number.as_deref(), Some("X1234567"));
    assert_eq!(exposed.personal_number.as_deref(), Some("00-2024"));
}

// An env entry's whole file is its secret; the file's name is not. The body
// has to come back byte-exact, or "Save as .env" hands back a different file.
#[test]
fn env_obscures_the_body_but_not_the_file_name() {
    let cryptor = Cryptor::new(&hash_secret("master-pw"));
    let body = "# api\nexport API_KEY='abc'\n\nURL=${HOST}/v1\n";
    let entry: Entry = serde_json::from_value(serde_json::json!({
        "id": "1", "type": "env", "title": "api", "fileName": ".env.production",
        "body": body
    }))
    .unwrap();

    let obscured = cryptor.obscure(&entry).unwrap();
    assert_ne!(obscured.body.as_deref(), Some(body));
    assert!(obscured.body.as_ref().unwrap().len() > 32);
    assert_eq!(obscured.file_name.as_deref(), Some(".env.production"));

    let exposed = cryptor.expose(&obscured).unwrap();
    assert_eq!(exposed.body.as_deref(), Some(body));
}

// An API key's token is its secret; the base URL, environment, scopes and
// expiry it is used with are not.
#[test]
fn apikey_obscures_the_token_but_not_where_it_is_used() {
    let cryptor = Cryptor::new(&hash_secret("master-pw"));
    let entry: Entry = serde_json::from_value(serde_json::json!({
        "id": "1", "type": "apikey", "title": "Coupler.io", "apiKey": "cpl_live_abc",
        "environment": "production", "baseUrl": "https://api.coupler.io/v1",
        "scopes": "read write", "expiry_date": "2027-01-01"
    }))
    .unwrap();

    let obscured = cryptor.obscure(&entry).unwrap();
    assert_ne!(obscured.api_key.as_deref(), Some("cpl_live_abc"));
    assert!(obscured.api_key.as_ref().unwrap().len() > 32);
    assert_eq!(obscured.environment.as_deref(), Some("production"));
    assert_eq!(
        obscured.base_url.as_deref(),
        Some("https://api.coupler.io/v1")
    );
    assert_eq!(obscured.scopes.as_deref(), Some("read write"));
    assert_eq!(obscured.expiry_date.as_deref(), Some("2027-01-01"));

    let exposed = cryptor.expose(&obscured).unwrap();
    assert_eq!(exposed.api_key.as_deref(), Some("cpl_live_abc"));
}

#[test]
fn empty_sensitive_fields_stay_empty() {
    let cryptor = Cryptor::new(&hash_secret("master-pw"));
    let entry: Entry =
        serde_json::from_value(serde_json::json!({ "id": "1", "type": "login", "title": "T" }))
            .unwrap();
    let obscured = cryptor.obscure(&entry).unwrap();
    assert_eq!(obscured.password.as_deref(), Some(""));
    assert_eq!(obscured.otp.as_deref(), Some(""));
}

#[test]
fn wrong_secret_fails() {
    let f = fixtures();
    let bad = Cryptor::new("not-the-secret");
    let case = &f["encrypt"][0];
    assert!(bad.decrypt(case["hex"].as_str().unwrap()).is_err());
}
