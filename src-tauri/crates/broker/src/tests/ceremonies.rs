//! Both ceremonies through `Connection`, sealed the way the extension seals
//! them, over the real `passkeys` code and an in-memory vault — and checked
//! the way a relying party checks what it gets back.

use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use p256::pkcs8::{DecodePrivateKey, DecodePublicKey};
use passkey_types::ctap2::{AuthenticatorData, Flags};
use passkey_types::encoding::{base64url, try_from_base64url};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use url::Url;

use super::{error_code, Code, Connection, Extension, Mock};
use crate::actions::Client;
use crate::passkeys::{self, rp_id, CeremonyLog};
use crate::testing::{seed, stored, vault};
use rowel_core::crypto::PayloadCipher;
use rowel_core::error::Result as CoreResult;
use rowel_core::models::{Entry, Passkey};
use rowel_core::passkey::store::{PasskeyVault, Stored};
use rowel_core::passkey::{Ceremony, UserConsent};
use rowel_core::store::{SqliteStore, VaultStore};

const ORIGIN: &str = "https://login.example.com";
const RP_ID: &str = "example.com";
const USER_ID: &[u8] = b"user-alice";

fn connected(mock: Mock) -> (Extension, Connection<Mock>) {
    let mut extension = Extension::new();
    let mut connection = Connection::new(mock.known("Chrome", "id-key"));
    extension.exchange(&mut connection);
    (extension, connection)
}

fn challenge() -> String {
    base64url(&rand::random::<[u8; 32]>())
}

// `PublicKeyCredentialCreationOptions` as `buildCredentialCreationOptions`
// hands them over: binary fields in base64url, `alg` a number.
fn creation(challenge: &str, exclude: &[&str]) -> Value {
    json!({
        "attestation": "none",
        "authenticatorSelection": { "residentKey": "preferred", "userVerification": "preferred" },
        "challenge": challenge,
        "extensions": { "credProps": true },
        "pubKeyCredParams": [
            { "type": "public-key", "alg": -8 },
            { "type": "public-key", "alg": -7 },
            { "type": "public-key", "alg": -257 },
        ],
        "rp": { "id": RP_ID, "name": "Example Inc" },
        "timeout": 30000,
        "excludeCredentials": exclude
            .iter()
            .map(|id| json!({ "id": id, "type": "public-key", "transports": ["internal"] }))
            .collect::<Vec<_>>(),
        "user": { "displayName": "Alice Example", "id": base64url(USER_ID), "name": "alice" },
    })
}

// And `PublicKeyCredentialRequestOptions`, `'internal'` added to each
// transport list as `buildCredentialRequestOptions` adds it.
fn request(challenge: &str, allow: &[&str]) -> Value {
    json!({
        "challenge": challenge,
        "enterpriseAttestationPossible": false,
        "extensions": {},
        "rpId": RP_ID,
        "timeout": 30000,
        "userVerification": "preferred",
        "allowCredentials": allow
            .iter()
            .map(|id| json!({ "id": id, "transports": ["usb", "internal"], "type": "public-key" }))
            .collect::<Vec<_>>(),
    })
}

/// The `response` of a sealed `passkeys-*` reply.
fn ceremony(
    extension: &Extension,
    connection: &mut Connection<Mock>,
    action: &str,
    public_key: Value,
    origin: &str,
) -> Value {
    let reply = extension.send(
        connection,
        action,
        json!({
            "publicKey": public_key,
            "origin": origin,
            "relatedOrigins": [],
            "groupName": "",
            "keys": [{ "id": "Chrome", "key": "id-key" }],
        }),
    );
    reply["response"].clone()
}

fn register(extension: &Extension, connection: &mut Connection<Mock>, options: Value) -> Value {
    ceremony(extension, connection, "passkeys-register", options, ORIGIN)
}

fn get(extension: &Extension, connection: &mut Connection<Mock>, options: Value) -> Value {
    ceremony(extension, connection, "passkeys-get", options, ORIGIN)
}

/// A ceremony's error code, which travels as a JSON number inside `response`.
fn failed(response: &Value) -> u8 {
    let code = response["errorCode"]
        .as_u64()
        .unwrap_or_else(|| panic!("expected an errorCode, got {response}"));
    u8::try_from(code).unwrap()
}

fn bytes(value: &Value) -> Vec<u8> {
    try_from_base64url(value.as_str().expect("a base64url string")).expect("base64url")
}

fn asks(connection: &Connection<Mock>) -> Vec<String> {
    connection.host().passkey_asks.lock().unwrap().clone()
}

// The bytes the relying party hashes, exactly as the spec and KeePassXC
// serialize them.
fn assert_client_data(credential: &Value, kind: &str, challenge: &str) {
    let json = String::from_utf8(bytes(&credential["response"]["clientDataJSON"])).unwrap();
    assert_eq!(
        json,
        format!(
            r#"{{"type":"{kind}","challenge":"{challenge}","origin":"{ORIGIN}","crossOrigin":false}}"#
        )
    );
}

/// What a relying party checks of an attestation; the public key it keeps.
fn verify_registration(credential: &Value, challenge: &str) -> VerifyingKey {
    assert_eq!(credential["type"], "public-key");
    assert_eq!(credential["authenticatorAttachment"], "platform");
    assert_client_data(credential, "webauthn.create", challenge);
    let response = &credential["response"];

    let attestation: coset::cbor::value::Value =
        coset::cbor::de::from_reader(bytes(&response["attestationObject"]).as_slice()).unwrap();
    let fields = attestation
        .as_map()
        .expect("the attestation object is a map");
    let field = |name: &str| {
        fields
            .iter()
            .find(|(key, _)| key.as_text() == Some(name))
            .map(|(_, value)| value)
            .unwrap_or_else(|| panic!("no {name}"))
    };
    assert_eq!(field("fmt").as_text(), Some("none"));
    let auth_data = field("authData").as_bytes().unwrap().clone();
    assert_eq!(auth_data, bytes(&response["authenticatorData"]));

    let parsed = AuthenticatorData::from_slice(&auth_data).unwrap();
    assert_eq!(parsed.rp_id_hash(), Sha256::digest(RP_ID).as_slice());
    assert!(parsed.flags.contains(Flags::UP | Flags::UV | Flags::AT));
    let attested = parsed.attested_credential_data.unwrap();
    assert_eq!(
        attested.credential_id(),
        bytes(&credential["id"]).as_slice()
    );

    // The SPKI the page hands to `getPublicKey()` is the attested COSE key.
    let spki = bytes(&response["publicKey"]);
    assert_eq!(
        passkey_authenticator::public_key_der_from_cose_key(&attested.key)
            .unwrap()
            .to_vec(),
        spki
    );
    assert_eq!(response["publicKeyAlgorithm"], -7);
    assert_eq!(
        response["clientExtensionResults"],
        json!({ "credProps": { "rk": true } })
    );
    VerifyingKey::from(p256::PublicKey::from_public_key_der(&spki).unwrap())
}

// --- the round trip ----------------------------------------------------------

#[test]
fn a_passkey_registered_through_the_extension_signs_in_through_it() {
    let (extension, mut connection) = connected(Mock::unlocked());

    let created_for = challenge();
    let created = register(&extension, &mut connection, creation(&created_for, &[]));
    let key = verify_registration(&created, &created_for);

    // Kept in the vault under the id the site now knows it by, with the key
    // whose public half the site was given.
    let stored = connection.host().passkeys.all();
    assert_eq!(stored.len(), 1);
    let passkey = &stored[0].passkey;
    assert_eq!(passkey.rp_id, RP_ID);
    assert_eq!(passkey.rp_name.as_deref(), Some("Example Inc"));
    assert_eq!(passkey.user_name, "alice");
    assert_eq!(passkey.user_display_name, "Alice Example");
    assert_eq!(
        bytes(&created["id"]),
        bytes(&Value::from(passkey.credential_id.clone()))
    );
    let private =
        p256::SecretKey::from_pkcs8_der(&bytes(&Value::from(passkey.private_key.clone())));
    assert_eq!(
        VerifyingKey::from(SigningKey::from(private.unwrap())),
        key,
        "the site holds the public half of the key we kept"
    );

    // Named in the allow list, and discovered without one.
    let id = created["id"].as_str().unwrap().to_string();
    for allow in [vec![id.as_str()], vec![]] {
        let signed_for = challenge();
        let signed = get(&extension, &mut connection, request(&signed_for, &allow));
        assert_eq!(signed["id"], created["id"]);
        assert_eq!(signed["type"], "public-key");
        assert!(
            signed["response"].get("attestationObject").is_none(),
            "the page builds an assertion only when there is no attestationObject"
        );
        assert_client_data(&signed, "webauthn.get", &signed_for);

        let response = &signed["response"];
        let auth_data = bytes(&response["authenticatorData"]);
        let parsed = AuthenticatorData::from_slice(&auth_data).unwrap();
        assert_eq!(parsed.rp_id_hash(), Sha256::digest(RP_ID).as_slice());
        assert!(parsed.flags.contains(Flags::UP | Flags::UV));
        assert_eq!(
            parsed.counter,
            Some(0),
            "synced passkeys keep a zero counter"
        );

        let mut signed_bytes = auth_data.clone();
        signed_bytes.extend(Sha256::digest(bytes(&response["clientDataJSON"])));
        let signature = Signature::from_der(&bytes(&response["signature"])).unwrap();
        key.verify(&signed_bytes, &signature)
            .expect("the assertion verifies under the key the site was given at registration");
        assert_eq!(bytes(&response["userHandle"]), USER_ID);
    }

    let made = passkey.created_at.clone();
    let sign_in = format!(
        "SignIn {{ rp_id: \"example.com\", accounts: [Account {{ credential_id: {id:?}, user_name: \"alice\", user_display_name: \"Alice Example\", created_at: {made:?} }}] }}"
    );
    assert_eq!(
        asks(&connection),
        [
            "Register { rp_id: \"example.com\", user_name: Some(\"alice\"), user_display_name: Some(\"Alice Example\") }".to_string(),
            sign_in.clone(),
            sign_in,
        ],
        "every ceremony is asked for, a sign-in with the account it would be as"
    );
}

/// A passkey for `RP_ID` as an import would leave it, for `user_name`, made
/// on `created_at`.
fn passkey(user_name: &str, created_at: &str) -> rowel_core::models::Passkey {
    rowel_core::models::Passkey {
        credential_id: base64url(format!("cred-{user_name}").as_bytes()),
        rp_id: RP_ID.into(),
        rp_name: None,
        user_handle: base64url(user_name.as_bytes()),
        user_name: user_name.into(),
        user_display_name: String::new(),
        private_key: fresh_key(),
        counter: 0,
        created_at: Some(created_at.into()),
    }
}

// A site that names no credential expects the user to pick the account. The
// library would take the newest; the user is asked with every one at stake,
// newest first, and signs in as the one they picked.
#[test]
fn a_sign_in_with_several_accounts_is_the_one_the_user_picks() {
    let (extension, mut connection) = connected(Mock {
        passkey_choice: 1,
        ..Mock::unlocked()
    });
    let vault = &connection.host().passkeys;
    for (name, made) in [
        ("alice", "2024-01-01T00:00:00Z"),
        ("bob", "2025-01-01T00:00:00Z"),
    ] {
        rowel_core::passkey::store::PasskeyVault::insert(vault, &passkey(name, made)).unwrap();
    }

    let signed = get(&extension, &mut connection, request(&challenge(), &[]));
    assert_eq!(
        bytes(&signed["id"]),
        b"cred-alice",
        "the second account offered — the older one — is the one picked"
    );
    assert_eq!(bytes(&signed["response"]["userHandle"]), b"alice");
    let asked = asks(&connection);
    assert_eq!(asked.len(), 1);
    assert!(
        asked[0].contains("user_name: \"bob\"") && asked[0].contains("user_name: \"alice\""),
        "both accounts were offered: {}",
        asked[0]
    );
    assert!(
        asked[0].find("bob") < asked[0].find("alice"),
        "newest first: {}",
        asked[0]
    );

    // An allow list naming one account leaves nothing to pick: the user is
    // asked about that one, and a pick past the end of the list is a no.
    let bob = base64url(b"cred-bob");
    let refused = get(&extension, &mut connection, request(&challenge(), &[&bob]));
    assert_eq!(failed(&refused), Code::PasskeyRequestCanceled as u8);
    let asked = asks(&connection);
    assert_eq!(asked.len(), 2);
    assert!(
        asked[1].contains("bob") && !asked[1].contains("alice"),
        "only the account the site allows is offered: {}",
        asked[1]
    );
}

// --- refusals ----------------------------------------------------------------

#[test]
fn a_ceremony_the_user_denies_is_cancelled() {
    let (extension, mut connection) = connected(Mock {
        passkey_consent: false,
        ..Mock::unlocked()
    });
    let refused = register(&extension, &mut connection, creation(&challenge(), &[]));
    assert_eq!(failed(&refused), Code::PasskeyRequestCanceled as u8);
    assert!(connection.host().passkeys.all().is_empty(), "no key minted");

    // One the user has, and still says no to.
    rowel_core::passkey::store::PasskeyVault::insert(
        &connection.host().passkeys,
        &rowel_core::models::Passkey {
            credential_id: base64url(b"cred-1"),
            rp_id: RP_ID.into(),
            rp_name: None,
            user_handle: base64url(USER_ID),
            user_name: "alice".into(),
            user_display_name: String::new(),
            private_key: fresh_key(),
            counter: 0,
            created_at: None,
        },
    )
    .unwrap();
    let refused = get(&extension, &mut connection, request(&challenge(), &[]));
    assert_eq!(failed(&refused), Code::PasskeyRequestCanceled as u8);
    assert_eq!(asks(&connection).len(), 2);
}

fn fresh_key() -> String {
    use p256::pkcs8::EncodePrivateKey;
    let secret = p256::SecretKey::random(&mut rand::thread_rng());
    base64url(secret.to_pkcs8_der().unwrap().as_bytes())
}

#[test]
fn an_excluded_credential_is_refused_before_the_user_is_asked() {
    let (extension, mut connection) = connected(Mock::unlocked());
    let created = register(&extension, &mut connection, creation(&challenge(), &[]));
    let id = created["id"].as_str().unwrap();

    let again = register(&extension, &mut connection, creation(&challenge(), &[id]));
    assert_eq!(failed(&again), Code::PasskeyCredentialExcluded as u8);
    assert_eq!(
        connection.host().passkeys.all().len(),
        1,
        "nothing new stored"
    );
    assert_eq!(
        asks(&connection).len(),
        1,
        "only the first registration was asked"
    );

    // An exclude list naming someone else's credential is no bar.
    let other = base64url(b"not ours");
    let fine = register(
        &extension,
        &mut connection,
        creation(&challenge(), &[&other]),
    );
    assert!(fine["id"].is_string());
}

#[test]
fn an_allow_list_naming_nothing_we_hold_finds_no_logins() {
    let (extension, mut connection) = connected(Mock::unlocked());
    register(&extension, &mut connection, creation(&challenge(), &[]));

    let unknown = base64url(b"someone else's");
    let response = get(
        &extension,
        &mut connection,
        request(&challenge(), &[&unknown]),
    );
    assert_eq!(failed(&response), Code::NoLoginsFound as u8);
    // An allow list of ids that do not even decode is not "any credential".
    let junk = get(&extension, &mut connection, request(&challenge(), &["!!"]));
    assert_eq!(failed(&junk), Code::NoLoginsFound as u8);
    assert_eq!(asks(&connection).len(), 1, "no sign-in was asked for");
}

#[test]
fn a_sign_in_with_no_passkey_for_the_site_finds_no_logins() {
    let (extension, mut connection) = connected(Mock::unlocked());
    let response = get(&extension, &mut connection, request(&challenge(), &[]));
    assert_eq!(failed(&response), Code::NoLoginsFound as u8);
    assert!(asks(&connection).is_empty());
}

#[test]
fn the_origin_and_the_rp_id_are_checked_before_anything_else() {
    let (extension, mut connection) = connected(Mock::unlocked());
    let at = |connection: &mut Connection<Mock>, origin: &str, rp: Value| {
        let mut options = creation(&challenge(), &[]);
        options["rp"]["id"] = rp;
        failed(&ceremony(
            &extension,
            connection,
            "passkeys-register",
            options,
            origin,
        ))
    };
    // Not a registrable suffix of the page's host.
    assert_eq!(at(&mut connection, "https://example.org", RP_ID.into()), 28);
    assert_eq!(
        at(&mut connection, "https://notexample.com", RP_ID.into()),
        28
    );
    assert_eq!(at(&mut connection, ORIGIN, "other.example.com".into()), 28);
    assert_eq!(at(&mut connection, ORIGIN, "com".into()), 28);
    // Not a page a passkey may be used from.
    assert_eq!(at(&mut connection, "http://example.com", RP_ID.into()), 25);
    assert_eq!(at(&mut connection, "", RP_ID.into()), 25);
    assert_eq!(at(&mut connection, "https://192.168.1.1", Value::Null), 27);

    let signed = |connection: &mut Connection<Mock>, rp: &str| {
        let mut options = request(&challenge(), &[]);
        options["rpId"] = rp.into();
        failed(&ceremony(
            &extension,
            connection,
            "passkeys-get",
            options,
            ORIGIN,
        ))
    };
    assert_eq!(signed(&mut connection, "example.org"), 28);

    assert!(
        asks(&connection).is_empty(),
        "nobody was asked about any of it"
    );
    assert!(connection.host().passkeys.all().is_empty());
}

#[test]
fn a_malformed_request_says_what_is_wrong_with_it() {
    let (extension, mut connection) = connected(Mock::unlocked());
    let mut with = |edit: &dyn Fn(&mut Value)| {
        let mut options = creation(&challenge(), &[]);
        edit(&mut options);
        failed(&register(&extension, &mut connection, options))
    };
    assert_eq!(with(&|o| o["challenge"] = "c2hvcnQ".into()), 32);
    assert_eq!(with(&|o| o["user"]["id"] = "".into()), 33);
    assert_eq!(with(&|o| o["user"]["id"] = base64url(&[7; 65]).into()), 33);
    assert_eq!(
        with(&|o| o["pubKeyCredParams"] = json!([{ "type": "public-key", "alg": -257 }])),
        29
    );
    assert_eq!(
        failed(&ceremony(
            &extension,
            &mut connection,
            "passkeys-register",
            json!({}),
            ORIGIN
        )),
        24
    );
}

#[test]
fn a_ceremony_needs_an_association() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock::unlocked());
    extension.exchange(&mut connection);
    let refused = extension.send(
        &mut connection,
        "passkeys-register",
        json!({ "publicKey": creation(&challenge(), &[]), "origin": ORIGIN }),
    );
    assert_eq!(error_code(&refused), Code::AssociationFailed as u8);
    assert!(connection.host().passkeys.all().is_empty());
}

// --- the rpId rule -----------------------------------------------------------

#[test]
fn an_rp_id_is_the_host_or_a_registrable_suffix_of_it() {
    let check = |origin: &str, claimed: Option<&str>| {
        rp_id(
            &Url::parse(origin).unwrap(),
            claimed.map(Value::from).as_ref(),
        )
    };
    assert_eq!(check(ORIGIN, None).as_deref(), Ok("login.example.com"));
    assert_eq!(
        check(ORIGIN, Some("Example.COM")).as_deref(),
        Ok("example.com")
    );
    assert_eq!(
        check("https://a.b.example.co.uk", Some("example.co.uk")).as_deref(),
        Ok("example.co.uk")
    );
    assert_eq!(
        check("https://example.co.uk", Some("co.uk")),
        Err(Code::PasskeyRpIdMismatch)
    );
    assert_eq!(
        check("http://localhost:3000", None).as_deref(),
        Ok("localhost")
    );
    assert_eq!(
        check("https://[::1]", None),
        Err(Code::PasskeyDomainNotValid)
    );
}

// --- what a ceremony leaves in the vault and on the log ---------------------------

/// The open vault in `store` as a host lends it to a ceremony: every
/// operation through the ceremony's log, as the app's and `rowel serve`'s
/// `OpenVault` make them.
struct Logged<'a> {
    store: &'a SqliteStore,
    cipher: &'a PayloadCipher,
    log: CeremonyLog,
}

impl PasskeyVault for Logged<'_> {
    fn find(&self, rp_id: &str) -> CoreResult<Vec<Stored>> {
        self.log
            .vault(self.store, self.cipher, |vault| vault.find(rp_id))
    }
    fn insert(&self, passkey: &Passkey) -> CoreResult<()> {
        self.log
            .vault(self.store, self.cipher, |vault| vault.insert(passkey))
    }
    fn update(&self, entry_id: &str, passkey: &Passkey) -> CoreResult<()> {
        self.log.vault(self.store, self.cipher, |vault| {
            vault.update(entry_id, passkey)
        })
    }
}

struct Yes;

impl UserConsent for Yes {
    fn approve(&self, _: Ceremony<'_>) -> Option<usize> {
        Some(0)
    }
}

/// One ceremony, `register` or not, run and settled as a host runs it.
fn logged(store: &SqliteStore, cipher: &PayloadCipher, register: bool) -> Result<Value, Code> {
    let client: Client =
        serde_json::from_value(json!({ "name": "Chrome", "key": "id-key" })).unwrap();
    let vault = Logged {
        store,
        cipher,
        log: CeremonyLog::new("dev", &client, ORIGIN),
    };
    let message = |public_key: Value| {
        json!({ "publicKey": public_key, "origin": ORIGIN })
            .as_object()
            .unwrap()
            .clone()
    };
    let outcome = if register {
        let registration = passkeys::registration(&message(creation(&challenge(), &[]))).unwrap();
        passkeys::register(&vault, Yes, registration)
    } else {
        let assertion = passkeys::assertion(&message(request(&challenge(), &[]))).unwrap();
        passkeys::assert(&vault, Yes, assertion)
    };
    vault.log.settle(Some(store), outcome)
}

fn used_rows(store: &SqliteStore) -> usize {
    store
        .audit_rows("dev")
        .unwrap()
        .iter()
        .filter(|row| row.event == "used")
        .count()
}

// A ceremony's vault write commits with its `used` row or not at all. With
// the log refusing rows, a registration is `AuditUnavailable` and the login
// it would have gone on is as it was — no credential the site never got —
// and a sign-in's counter does not move. Once the log takes rows, each goes
// through with exactly one row, the one its write committed with; a sign-in
// that writes nothing has its row written before the assertion goes out.
#[test]
fn a_ceremony_whose_row_will_not_write_leaves_the_vault_as_it_was() {
    let (_dir, store, cipher) = vault();
    let login = Entry {
        id: "site".into(),
        kind: "login".into(),
        title: "Example".into(),
        website: Some("https://example.com".into()),
        ..Default::default()
    };
    seed(&store, &cipher, &login);
    let payload = |store: &SqliteStore| store.get("site").unwrap().unwrap().payload;
    let before = payload(&store);

    store.refuse_audit_writes().unwrap();
    assert_eq!(logged(&store, &cipher, true), Err(Code::AuditUnavailable));
    assert_eq!(payload(&store), before, "no passkey was kept");
    assert_eq!(
        store.list().unwrap().len(),
        1,
        "nor put on a login of its own"
    );

    store.allow_audit_writes().unwrap();
    logged(&store, &cipher, true).unwrap();
    assert_eq!(stored(&store, &cipher, "site").passkeys.unwrap().len(), 1);
    assert_eq!(
        used_rows(&store),
        1,
        "the row it committed with, and no other"
    );

    // An imported credential keeps a counter, and a sign-in moves it.
    let mut counted = stored(&store, &cipher, "site");
    counted.passkeys.as_mut().unwrap()[0].counter = 5;
    seed(&store, &cipher, &counted);
    let counter = |store: &SqliteStore| stored(store, &cipher, "site").passkeys.unwrap()[0].counter;
    store.refuse_audit_writes().unwrap();
    assert_eq!(logged(&store, &cipher, false), Err(Code::AuditUnavailable));
    assert_eq!(counter(&store), 5, "the counter did not move");
    store.allow_audit_writes().unwrap();
    logged(&store, &cipher, false).unwrap();
    assert_eq!(counter(&store), 6);
    assert_eq!(used_rows(&store), 2);

    // A synced credential keeps none: the sign-in writes nothing to the
    // vault, and its row is what lets the assertion out.
    let mut synced = stored(&store, &cipher, "site");
    synced.passkeys.as_mut().unwrap()[0].counter = 0;
    seed(&store, &cipher, &synced);
    store.refuse_audit_writes().unwrap();
    assert_eq!(logged(&store, &cipher, false), Err(Code::AuditUnavailable));
    store.allow_audit_writes().unwrap();
    logged(&store, &cipher, false).unwrap();
    assert_eq!(used_rows(&store), 3);
}
