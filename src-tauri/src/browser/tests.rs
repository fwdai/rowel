use std::cell::{Cell, RefCell};
use std::io::Cursor;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD, Engine};
use crypto_box::{aead::Aead, Nonce, PublicKey, SalsaBox, SecretKey};
use serde_json::{json, Map, Value};

use super::actions::{Client, Connection, Host, Login};
use super::manifest::{self, Family, KEEPASSXC, ROWEL};
use super::passkeys::{self, Assertion, Registration};
use super::protocol::{increment, str_of, Code, NONCE_LEN, VERSION};
use super::{
    frame, group_container, group_socket_dir, proxy, root_dir, save_login_in, server, socket_name,
    Pending, SafariSocketError, IDENTIFIER, SUN_PATH_MAX,
};
use crate::crypto::{PayloadCipher, VaultKey};
use crate::models::{Entry, PasswordHistoryItem};
use crate::passkey::store::MemoryVault;
use crate::passkey::{Ceremony, UserConsent};
use crate::store::{migrate, SqliteStore, VaultStore};
use rowel_core::app::DESKTOP_APP_GROUP;

// Passkeys through the extension, end to end.
mod ceremonies;

// --- a vault to talk to ------------------------------------------------------

struct Mock {
    enabled: Cell<bool>,
    locked: Cell<bool>,
    logins: Vec<Login>,
    clients: RefCell<Vec<Client>>,
    /// What the user answers an `associate` with.
    consent: Option<String>,
    /// The yes cannot be kept: the write fails, or the vault changed hands.
    remember_fails: Cell<bool>,
    totp: Option<String>,
    locks: Cell<u32>,
    raises: Cell<u32>,
    /// Every `save_login`: id, url, host, username, password.
    saves: RefCell<Vec<Save>>,
    /// Whether a save fails as a write, not as an unknown id.
    read_only: bool,
    /// The vault's passkeys, and what the user answers a passkey ask with:
    /// whether at all, and which account when it is a sign-in.
    passkeys: MemoryVault,
    passkey_consent: bool,
    passkey_choice: usize,
    /// Every passkey ask the user was shown.
    passkey_asks: Arc<Mutex<Vec<String>>>,
}

type Save = (Option<String>, String, String, String, String);

/// The user's answer to a passkey ask, noting what they were asked.
struct Answer {
    allow: bool,
    choice: usize,
    asks: Arc<Mutex<Vec<String>>>,
}

impl UserConsent for Answer {
    fn approve(&self, ceremony: Ceremony<'_>) -> Option<usize> {
        self.asks.lock().unwrap().push(format!("{ceremony:?}"));
        self.allow.then_some(self.choice)
    }
}

impl Mock {
    fn unlocked() -> Self {
        Self {
            enabled: Cell::new(true),
            locked: Cell::new(false),
            logins: vec![login("gh", "GitHub", "octocat", "hunter2", Some("123456"))],
            clients: RefCell::new(Vec::new()),
            consent: Some("Chrome".into()),
            remember_fails: Cell::new(false),
            totp: Some("654321".into()),
            locks: Cell::new(0),
            raises: Cell::new(0),
            saves: RefCell::new(Vec::new()),
            read_only: false,
            passkeys: MemoryVault::new(),
            passkey_consent: true,
            passkey_choice: 0,
            passkey_asks: Arc::default(),
        }
    }

    fn answer(&self) -> Answer {
        Answer {
            allow: self.passkey_consent,
            choice: self.passkey_choice,
            asks: self.passkey_asks.clone(),
        }
    }

    fn known(self, name: &str, key: &str) -> Self {
        self.clients.borrow_mut().push(Client {
            name: name.into(),
            key: key.into(),
        });
        self
    }
}

// --- the consent slot ----------------------------------------------------------

#[test]
fn an_ask_that_was_answered_leaves_the_next_ask_its_slot() {
    let pending: Pending<bool> = Pending::new();
    let (first, answer) = pending.begin("a").unwrap();
    assert!(pending.begin("b").is_none(), "one ask at a time");
    assert!(
        !pending.answer("b", true),
        "an answer for another tag does nothing"
    );
    assert!(pending.answer("a", true));
    assert_eq!(answer.recv().ok(), Some(true));

    // The answer freed the slot, and the next ask took it before the first
    // ask's thread got round to tidying up. That tidy-up must be a no-op.
    let (_, next) = pending.begin("b").expect("the answer freed the slot");
    pending.end(first);
    assert!(
        pending.answer("b", false),
        "the second ask is still waiting"
    );
    assert_eq!(next.recv().ok(), Some(false));

    // An ask nobody answered does clear its own slot.
    let (third, _) = pending.begin("c").unwrap();
    pending.end(third);
    assert!(pending.begin("d").is_some());
}

// --- what the host claims to be --------------------------------------------------

// The extension turns features on by the version alone (see `VERSION`): this
// host answers the passkey actions, so it claims the release that turns them
// on — and not the one after, whose default passkey *group* the vault has no
// groups to honour.
#[test]
fn the_version_claimed_turns_on_nothing_this_host_lacks() {
    assert_eq!(VERSION, "2.7.7");
}

fn login(id: &str, title: &str, username: &str, password: &str, totp: Option<&str>) -> Login {
    Login {
        id: id.into(),
        title: title.into(),
        username: username.into(),
        password: password.into(),
        totp: totp.map(String::from),
    }
}

impl Host for Mock {
    fn enabled(&self) -> bool {
        self.enabled.get()
    }
    fn database_hash(&self) -> Option<String> {
        (!self.locked.get()).then(|| "abc123".to_string())
    }
    fn clients(&self) -> Vec<Client> {
        self.clients.borrow().clone()
    }
    fn associate(&self, key: &str) -> Result<String, Code> {
        let name = self.consent.clone().ok_or(Code::ActionCancelledOrDenied)?;
        if self.remember_fails.get() {
            return Err(Code::AssociationFailed);
        }
        self.clients.borrow_mut().push(Client {
            name: name.clone(),
            key: key.into(),
        });
        Ok(name)
    }
    fn logins_for(&self, host: &str) -> Vec<Login> {
        if host == "github.com" {
            self.logins.clone()
        } else {
            Vec::new()
        }
    }
    fn totp(&self, id: &str) -> Option<String> {
        (id == "gh").then(|| self.totp.clone()).flatten()
    }
    fn generate_password(&self) -> Option<String> {
        Some("generated".into())
    }
    fn save_login(
        &self,
        id: Option<&str>,
        url: &str,
        host: &str,
        username: &str,
        password: &str,
    ) -> Result<(), Code> {
        if id.is_some_and(|id| id != "gh") {
            return Err(Code::NoValidUuidProvided);
        }
        if self.read_only {
            return Err(Code::ActionCancelledOrDenied);
        }
        self.saves.borrow_mut().push((
            id.map(String::from),
            url.into(),
            host.into(),
            username.into(),
            password.into(),
        ));
        Ok(())
    }
    // The real ceremonies, over an in-memory vault and a scripted user.
    fn passkey_register(&self, registration: Registration) -> Result<Value, Code> {
        passkeys::register(&self.passkeys, self.answer(), registration)
    }
    fn passkey_get(&self, assertion: Assertion) -> Result<Value, Code> {
        passkeys::assert(&self.passkeys, self.answer(), assertion)
    }
    fn lock(&self) {
        self.locks.set(self.locks.get() + 1);
    }
    fn unlock_requested(&self) {
        self.raises.set(self.raises.get() + 1);
    }
}

// --- the extension's side ----------------------------------------------------

struct Extension {
    secret: SecretKey,
    host: Option<SalsaBox>,
}

fn nonce() -> [u8; NONCE_LEN] {
    rand::random()
}

impl Extension {
    fn new() -> Self {
        Self {
            secret: SecretKey::generate(&mut rand::rngs::OsRng),
            host: None,
        }
    }

    fn public_key(&self) -> String {
        STANDARD.encode(self.secret.public_key().as_bytes())
    }

    fn exchange<H: Host>(&mut self, connection: &mut Connection<H>) -> Value {
        let nonce = nonce();
        let request = json!({
            "action": "change-public-keys",
            "publicKey": self.public_key(),
            "nonce": STANDARD.encode(nonce),
            "clientID": "test",
        });
        let response = raw(connection, &request);
        if let Some(key) = response["publicKey"].as_str() {
            let bytes = STANDARD.decode(key).unwrap();
            let public = PublicKey::from_slice(&bytes).unwrap();
            self.host = Some(SalsaBox::new(&public, &self.secret));
            let mut expected = nonce;
            increment(&mut expected);
            assert_eq!(response["nonce"], STANDARD.encode(expected));
            assert_eq!(response["success"], "true");
        }
        response
    }

    /// A sealed request. The reply is opened and checked the way the extension
    /// checks it, and its message returned; a refusal comes back as sent.
    fn send<H: Host>(&self, connection: &mut Connection<H>, action: &str, message: Value) -> Value {
        self.send_with(connection, action, message, Map::new())
    }

    fn send_with<H: Host>(
        &self,
        connection: &mut Connection<H>,
        action: &str,
        mut message: Value,
        outer: Map<String, Value>,
    ) -> Value {
        let sealed = self.host.as_ref().expect("keys exchanged");
        let nonce = nonce();
        message["action"] = action.into();
        let cipher = sealed
            .encrypt(
                Nonce::from_slice(&nonce),
                serde_json::to_vec(&message).unwrap().as_slice(),
            )
            .unwrap();
        let mut request = outer;
        request.insert("action".into(), action.into());
        request.insert("message".into(), STANDARD.encode(cipher).into());
        request.insert("nonce".into(), STANDARD.encode(nonce).into());
        request.insert("clientID".into(), "test".into());
        let response = raw(connection, &Value::Object(request));
        let Some(body) = response["message"].as_str() else {
            return response;
        };
        let mut expected = nonce;
        increment(&mut expected);
        assert_eq!(
            response["nonce"],
            STANDARD.encode(expected),
            "reply nonce is the request's plus one"
        );
        let reply_nonce: [u8; NONCE_LEN] = STANDARD
            .decode(response["nonce"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let plain = sealed
            .decrypt(
                Nonce::from_slice(&reply_nonce),
                STANDARD.decode(body).unwrap().as_slice(),
            )
            .unwrap();
        let opened: Value = serde_json::from_slice(&plain).unwrap();
        assert_eq!(opened["success"], "true");
        assert_eq!(opened["nonce"], response["nonce"]);
        assert_eq!(opened["version"], VERSION);
        assert_eq!(opened["action"], action);
        opened
    }
}

fn raw<H: Host>(connection: &mut Connection<H>, request: &Value) -> Value {
    let reply = connection.handle(&serde_json::to_vec(request).unwrap());
    serde_json::from_slice(&reply).unwrap()
}

fn error_code(response: &Value) -> u8 {
    response["errorCode"]
        .as_str()
        .expect("a refusal carries errorCode as a string")
        .parse()
        .unwrap()
}

fn ready() -> (Extension, Connection<Mock>) {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock::unlocked());
    extension.exchange(&mut connection);
    (extension, connection)
}

// --- the exchange ------------------------------------------------------------

#[test]
fn increment_carries_across_bytes() {
    let mut nonce = [0xff; NONCE_LEN];
    nonce[2] = 0x01;
    increment(&mut nonce);
    assert_eq!(&nonce[..4], &[0x00, 0x00, 0x02, 0xff]);
}

#[test]
fn the_exchange_answers_with_the_hosts_key_and_the_next_nonce() {
    let (extension, _) = ready();
    assert!(extension.host.is_some());
}

#[test]
fn an_exchange_without_a_key_is_refused() {
    let mut connection = Connection::new(Mock::unlocked());
    let response = raw(
        &mut connection,
        &json!({ "action": "change-public-keys", "nonce": STANDARD.encode(nonce()) }),
    );
    assert_eq!(
        error_code(&response),
        Code::ClientPublicKeyNotReceived as u8
    );
}

#[test]
fn a_sealed_request_before_the_exchange_is_refused() {
    let mut connection = Connection::new(Mock::unlocked());
    let response = raw(
        &mut connection,
        &json!({ "action": "get-databasehash", "message": "x", "nonce": "y" }),
    );
    assert_eq!(
        error_code(&response),
        Code::ClientPublicKeyNotReceived as u8
    );
}

#[test]
fn a_tampered_message_is_refused() {
    let (_extension, mut connection) = ready();
    let mut request = Map::new();
    request.insert("action".into(), "get-databasehash".into());
    request.insert("message".into(), STANDARD.encode(b"not a box").into());
    request.insert("nonce".into(), STANDARD.encode(nonce()).into());
    let response = raw(&mut connection, &Value::Object(request));
    assert_eq!(error_code(&response), Code::CannotDecryptMessage as u8);
}

#[test]
fn a_message_whose_action_differs_from_the_envelope_is_refused() {
    let (extension, mut connection) = ready();
    let sealed = extension.host.as_ref().unwrap();
    let nonce = nonce();
    let inner = json!({ "action": "lock-database" });
    let cipher = sealed
        .encrypt(
            Nonce::from_slice(&nonce),
            serde_json::to_vec(&inner).unwrap().as_slice(),
        )
        .unwrap();
    let response = raw(
        &mut connection,
        &json!({ "action": "get-databasehash", "message": STANDARD.encode(cipher), "nonce": STANDARD.encode(nonce) }),
    );
    assert_eq!(error_code(&response), Code::IncorrectAction as u8);
}

#[test]
fn junk_is_refused_as_an_empty_message() {
    let mut connection = Connection::new(Mock::unlocked());
    let reply = connection.handle(b"[1, 2");
    let response: Value = serde_json::from_slice(&reply).unwrap();
    assert_eq!(error_code(&response), Code::EmptyMessageReceived as u8);
}

// --- the locked vault --------------------------------------------------------

#[test]
fn a_locked_vault_refuses_everything_but_the_exchange() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock::unlocked());
    connection_mock(&connection).locked.set(true);
    let exchanged = extension.exchange(&mut connection);
    assert_eq!(exchanged["success"], "true");

    let response = extension.send(&mut connection, "get-databasehash", json!({}));
    assert_eq!(error_code(&response), Code::DatabaseNotOpened as u8);
    assert_eq!(response["action"], "get-databasehash");
    assert_eq!(connection_mock(&connection).raises.get(), 0);
}

#[test]
fn trigger_unlock_on_a_locked_vault_brings_the_app_forward() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock::unlocked());
    connection_mock(&connection).locked.set(true);
    extension.exchange(&mut connection);

    let mut outer = Map::new();
    outer.insert("triggerUnlock".into(), "true".into());
    let response = extension.send_with(&mut connection, "get-databasehash", json!({}), outer);
    assert_eq!(error_code(&response), Code::DatabaseNotOpened as u8);
    assert_eq!(connection_mock(&connection).raises.get(), 1);
}

fn connection_mock(connection: &Connection<Mock>) -> &Mock {
    connection.host()
}

// --- association -------------------------------------------------------------

#[test]
fn get_databasehash_answers_with_the_hash() {
    let (extension, mut connection) = ready();
    let response = extension.send(&mut connection, "get-databasehash", json!({}));
    assert_eq!(response["hash"], "abc123");
}

#[test]
fn associate_needs_the_session_key_repeated_inside() {
    let (extension, mut connection) = ready();
    let response = extension.send(
        &mut connection,
        "associate",
        json!({ "key": "someone else's", "idKey": "id-key" }),
    );
    assert_eq!(error_code(&response), Code::AssociationFailed as u8);
    assert!(connection.host().clients.borrow().is_empty());
}

#[test]
fn associate_asks_the_user_and_remembers_the_answer() {
    let (extension, mut connection) = ready();
    let response = extension.send(
        &mut connection,
        "associate",
        json!({ "key": extension.public_key(), "idKey": "id-key" }),
    );
    assert_eq!(response["id"], "Chrome");
    assert_eq!(response["hash"], "abc123");
    assert_eq!(
        connection.host().clients.borrow().as_slice(),
        &[Client {
            name: "Chrome".into(),
            key: "id-key".into()
        }]
    );

    // And the connection is associated from here on: no keys needed.
    let logins = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com/login" }),
    );
    assert_eq!(logins["count"], 1);
}

#[test]
fn a_refused_associate_is_denied_and_forgotten() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock {
        consent: None,
        ..Mock::unlocked()
    });
    extension.exchange(&mut connection);
    let response = extension.send(
        &mut connection,
        "associate",
        json!({ "key": extension.public_key(), "idKey": "id-key" }),
    );
    assert_eq!(error_code(&response), Code::ActionCancelledOrDenied as u8);
    assert!(connection.host().clients.borrow().is_empty());
    let logins = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com" }),
    );
    assert_eq!(error_code(&logins), Code::AssociationFailed as u8);
}

#[test]
fn a_yes_that_could_not_be_kept_is_refused_not_reported() {
    // The write failed, or the vault the user was asked for is no longer the
    // one open: the extension must not hear "in" when its next request
    // will find it is not.
    let (extension, mut connection) = ready();
    connection.host().remember_fails.set(true);
    let response = extension.send(
        &mut connection,
        "associate",
        json!({ "key": extension.public_key(), "idKey": "id-key" }),
    );
    assert_eq!(error_code(&response), Code::AssociationFailed as u8);
    assert!(connection.host().clients.borrow().is_empty());
    let logins = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com" }),
    );
    assert_eq!(error_code(&logins), Code::AssociationFailed as u8);
}

#[test]
fn test_associate_proves_a_remembered_key_under_its_name() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock::unlocked().known("Chrome", "id-key"));
    extension.exchange(&mut connection);

    let wrong = extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "other" }),
    );
    assert_eq!(error_code(&wrong), Code::AssociationFailed as u8);
    let renamed = extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Firefox", "key": "id-key" }),
    );
    assert_eq!(error_code(&renamed), Code::AssociationFailed as u8);

    let right = extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "id-key" }),
    );
    assert_eq!(right["id"], "Chrome");
    assert_eq!(right["hash"], "abc123");
}

#[test]
fn a_request_carrying_a_remembered_key_counts_as_associated() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock::unlocked().known("Chrome", "id-key"));
    extension.exchange(&mut connection);
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com", "keys": [{ "id": "Chrome", "key": "id-key" }] }),
    );
    assert_eq!(response["count"], 1);
}

// --- what the extension gets -------------------------------------------------

fn associated() -> (Extension, Connection<Mock>) {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock::unlocked().known("Chrome", "id-key"));
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "id-key" }),
    );
    (extension, connection)
}

#[test]
fn get_logins_answers_with_the_logins_for_the_site() {
    let (extension, mut connection) = associated();
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com/login", "submitUrl": "https://github.com/session" }),
    );
    assert_eq!(response["count"], 1);
    assert_eq!(response["hash"], "abc123");
    assert_eq!(
        response["entries"],
        json!([{ "login": "octocat", "password": "hunter2", "name": "GitHub", "uuid": "gh", "totp": "123456" }])
    );
}

#[test]
fn get_logins_for_a_site_without_any_is_the_no_logins_refusal() {
    let (extension, mut connection) = associated();
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://example.com" }),
    );
    assert_eq!(error_code(&response), Code::NoLoginsFound as u8);
}

#[test]
fn get_logins_without_a_usable_url_is_refused() {
    let (extension, mut connection) = associated();
    let missing = extension.send(&mut connection, "get-logins", json!({}));
    assert_eq!(error_code(&missing), Code::NoUrlProvided as u8);
    let junk = extension.send(&mut connection, "get-logins", json!({ "url": "not a url" }));
    assert_eq!(error_code(&junk), Code::NoUrlProvided as u8);
}

#[test]
fn get_totp_answers_with_the_current_code() {
    let (extension, mut connection) = associated();
    let response = extension.send(&mut connection, "get-totp", json!({ "uuid": "gh" }));
    assert_eq!(response["totp"], "654321");
    let unknown = extension.send(&mut connection, "get-totp", json!({ "uuid": "nope" }));
    assert_eq!(error_code(&unknown), Code::NoValidUuidProvided as u8);
    let blank = extension.send(&mut connection, "get-totp", json!({}));
    assert_eq!(error_code(&blank), Code::NoValidUuidProvided as u8);
}

#[test]
fn set_login_without_a_uuid_saves_a_new_login_for_the_site() {
    let (extension, mut connection) = associated();
    let response = extension.send(
        &mut connection,
        "set-login",
        json!({
            "url": "https://Example.com/signup",
            "submitUrl": "https://example.com/session",
            "login": "me",
            "password": "s3cret",
            "group": "",
            "groupUuid": "",
            "downloadFavicon": "true",
        }),
    );
    assert_eq!(response["error"], "success");
    assert_eq!(response["hash"], "abc123");
    assert!(response["count"].is_null() && response["entries"].is_null());
    assert_eq!(
        connection.host().saves.borrow().as_slice(),
        &[(
            None,
            "https://Example.com/signup".into(),
            "example.com".into(),
            "me".into(),
            "s3cret".into()
        )]
    );
}

#[test]
fn set_login_with_a_uuid_updates_that_login() {
    let (extension, mut connection) = associated();
    let response = extension.send(
        &mut connection,
        "set-login",
        json!({ "url": "https://github.com/login", "uuid": "gh", "login": "octocat", "password": "new" }),
    );
    assert_eq!(response["error"], "success");
    let saves = connection.host().saves.borrow();
    assert_eq!(saves.len(), 1);
    assert_eq!(saves[0].0.as_deref(), Some("gh"));
    assert_eq!(saves[0].4, "new");
}

#[test]
fn set_login_for_an_unknown_uuid_is_refused() {
    let (extension, mut connection) = associated();
    let response = extension.send(
        &mut connection,
        "set-login",
        json!({ "url": "https://github.com", "uuid": "nope", "login": "a", "password": "b" }),
    );
    assert_eq!(error_code(&response), Code::NoValidUuidProvided as u8);
    assert!(connection.host().saves.borrow().is_empty());
}

#[test]
fn set_login_needs_an_association() {
    let (extension, mut connection) = ready();
    let response = extension.send(
        &mut connection,
        "set-login",
        json!({ "url": "https://github.com", "login": "a", "password": "b" }),
    );
    assert_eq!(error_code(&response), Code::AssociationFailed as u8);
    assert!(connection.host().saves.borrow().is_empty());
}

#[test]
fn a_set_login_that_fails_to_write_says_so_inside_the_reply() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock {
        read_only: true,
        ..Mock::unlocked().known("Chrome", "id-key")
    });
    extension.exchange(&mut connection);
    let response = extension.send(
        &mut connection,
        "set-login",
        json!({ "url": "https://github.com", "login": "a", "password": "b", "keys": [{ "id": "Chrome", "key": "id-key" }] }),
    );
    assert_eq!(response["error"], "error");
    assert_eq!(response["hash"], "abc123");
}

// --- the save itself, against a store --------------------------------------------

fn vault() -> (tempfile::TempDir, SqliteStore, PayloadCipher) {
    let dir = tempfile::tempdir().unwrap();
    let key = VaultKey::legacy_from_password("pw");
    let store =
        SqliteStore::open(&dir.path().join("vault.db"), key.sqlcipher_key().as_slice()).unwrap();
    (dir, store, key.payload_cipher())
}

fn stored(store: &SqliteStore, cipher: &PayloadCipher, id: &str) -> Entry {
    let record = store.get(id).unwrap().unwrap();
    cipher.unseal(&record.id, &record.payload).unwrap()
}

fn seed(store: &SqliteStore, cipher: &PayloadCipher, entry: &Entry) {
    let payload = cipher.seal(entry).unwrap();
    store
        .upsert(&migrate::build_record(entry, payload).unwrap())
        .unwrap();
}

const THEN: &str = "2026-01-01T00:00:00.000Z";
const NOW: &str = "2026-09-26T12:00:00.000Z";

#[test]
fn a_save_without_an_id_creates_a_login_for_the_site() {
    let (_dir, store, cipher) = vault();
    save_login_in(
        &store,
        &cipher,
        None,
        "https://github.com/signup",
        "github.com",
        "octocat",
        "hunter2",
        NOW,
    )
    .unwrap();

    let metas = store.list().unwrap();
    assert_eq!(metas.len(), 1);
    assert_eq!(metas[0].kind, "login");
    assert_eq!(metas[0].url_host, "github.com");
    let entry = stored(&store, &cipher, &metas[0].id);
    assert_eq!(entry.title, "github.com");
    assert_eq!(entry.website.as_deref(), Some("https://github.com/signup"));
    assert_eq!(entry.username.as_deref(), Some("octocat"));
    assert_eq!(entry.password.as_deref(), Some("hunter2"));
    assert_eq!(entry.created_at.as_deref(), Some(NOW));
    assert_eq!(entry.updated_at.as_deref(), Some(NOW));
    assert_eq!(entry.password_updated_at.as_deref(), Some(NOW));
}

// A login kept by its email is served to the extension under that email
// (`logins_for`), so what comes back goes to the same field: the row must not
// grow a username beside the email it already had.
#[test]
fn a_save_over_an_email_login_writes_back_to_its_email() {
    let (_dir, store, cipher) = vault();
    seed(
        &store,
        &cipher,
        &Entry {
            id: "gh".into(),
            kind: "login".into(),
            title: "GitHub".into(),
            website: Some("https://github.com".into()),
            email: Some("octocat@example.com".into()),
            password: Some("old".into()),
            ..Entry::default()
        },
    );

    save_login_in(
        &store,
        &cipher,
        Some("gh"),
        "https://github.com/login",
        "github.com",
        "octo@example.com",
        "new",
        NOW,
    )
    .unwrap();

    let entry = stored(&store, &cipher, "gh");
    assert_eq!(entry.email.as_deref(), Some("octo@example.com"));
    assert_eq!(entry.username, None, "no second name beside the email");
    assert_eq!(entry.password.as_deref(), Some("new"));

    // A name that is not an email cannot go in the email field: it becomes
    // the username, and the email is left as it was.
    save_login_in(
        &store,
        &cipher,
        Some("gh"),
        "https://github.com/login",
        "github.com",
        "octocat",
        "new",
        NOW,
    )
    .unwrap();
    let entry = stored(&store, &cipher, "gh");
    assert_eq!(entry.username.as_deref(), Some("octocat"));
    assert_eq!(entry.email.as_deref(), Some("octo@example.com"));

    // With a username of its own, that is the field — the email stays what
    // it was, whatever the page called the user.
    seed(
        &store,
        &cipher,
        &Entry {
            id: "both".into(),
            kind: "login".into(),
            title: "GitHub".into(),
            website: Some("https://github.com".into()),
            username: Some("octocat".into()),
            email: Some("octocat@example.com".into()),
            password: Some("old".into()),
            ..Entry::default()
        },
    );
    save_login_in(
        &store,
        &cipher,
        Some("both"),
        "https://github.com/login",
        "github.com",
        "octocat2",
        "new",
        NOW,
    )
    .unwrap();
    let entry = stored(&store, &cipher, "both");
    assert_eq!(entry.username.as_deref(), Some("octocat2"));
    assert_eq!(entry.email.as_deref(), Some("octocat@example.com"));
}

#[test]
fn a_save_over_a_login_changes_only_what_the_page_sent() {
    let (_dir, store, cipher) = vault();
    seed(
        &store,
        &cipher,
        &Entry {
            id: "gh".into(),
            kind: "login".into(),
            title: "GitHub (work)".into(),
            website: Some("https://github.com".into()),
            username: Some("octocat".into()),
            password: Some("old".into()),
            otp: Some("JBSWY3DPEHPK3PXP".into()),
            tags: Some(vec!["work".into()]),
            created_at: Some(THEN.into()),
            updated_at: Some(THEN.into()),
            password_updated_at: Some(THEN.into()),
            ..Entry::default()
        },
    );

    save_login_in(
        &store,
        &cipher,
        Some("gh"),
        "https://github.com/settings",
        "github.com",
        "octocat",
        "new",
        NOW,
    )
    .unwrap();

    let entry = stored(&store, &cipher, "gh");
    assert_eq!(entry.password.as_deref(), Some("new"));
    assert_eq!(entry.password_updated_at.as_deref(), Some(NOW), "rotated");
    assert_eq!(entry.updated_at.as_deref(), Some(NOW));
    // Untouched: the row is the user's, the page only knows two fields of it.
    assert_eq!(entry.title, "GitHub (work)");
    assert_eq!(entry.website.as_deref(), Some("https://github.com"));
    assert_eq!(entry.otp.as_deref(), Some("JBSWY3DPEHPK3PXP"));
    assert_eq!(entry.tags, Some(vec!["work".to_string()]));
    assert_eq!(entry.created_at.as_deref(), Some(THEN));
    assert_eq!(
        store.list().unwrap().len(),
        1,
        "an update, not a second row"
    );
}

#[test]
fn a_save_with_the_same_password_does_not_count_as_a_rotation() {
    let (_dir, store, cipher) = vault();
    seed(
        &store,
        &cipher,
        &Entry {
            id: "gh".into(),
            kind: "login".into(),
            title: "GitHub".into(),
            username: Some("old-name".into()),
            password: Some("same".into()),
            password_updated_at: Some(THEN.into()),
            ..Entry::default()
        },
    );
    save_login_in(
        &store,
        &cipher,
        Some("gh"),
        "https://github.com",
        "github.com",
        "new-name",
        "same",
        NOW,
    )
    .unwrap();
    let entry = stored(&store, &cipher, "gh");
    assert_eq!(entry.username.as_deref(), Some("new-name"));
    assert_eq!(entry.password_updated_at.as_deref(), Some(THEN));
    assert_eq!(entry.updated_at.as_deref(), Some(NOW));
    assert_eq!(entry.password_history, None, "nothing was replaced");
}

// A password the page rotated is kept in the login's history, in front of
// what was already there — the same record the editor's save makes.
#[test]
fn a_save_that_rotates_the_password_keeps_the_old_one_in_history() {
    let (_dir, store, cipher) = vault();
    seed(
        &store,
        &cipher,
        &Entry {
            id: "gh".into(),
            kind: "login".into(),
            title: "GitHub".into(),
            username: Some("octocat".into()),
            password: Some("old".into()),
            password_history: Some(vec![PasswordHistoryItem {
                password: "older".into(),
                replaced_at: THEN.into(),
            }]),
            ..Entry::default()
        },
    );
    save_login_in(
        &store,
        &cipher,
        Some("gh"),
        "https://github.com",
        "github.com",
        "octocat",
        "new",
        NOW,
    )
    .unwrap();

    let entry = stored(&store, &cipher, "gh");
    assert_eq!(entry.password.as_deref(), Some("new"));
    let history: Vec<(&str, &str)> = entry
        .password_history
        .iter()
        .flatten()
        .map(|p| (p.password.as_str(), p.replaced_at.as_str()))
        .collect();
    assert_eq!(history, [("old", NOW), ("older", THEN)]);
}

#[test]
fn a_save_over_something_that_is_not_a_login_is_refused() {
    let (_dir, store, cipher) = vault();
    seed(
        &store,
        &cipher,
        &Entry {
            id: "note".into(),
            kind: "note".into(),
            title: "Not a login".into(),
            note: Some("keep".into()),
            ..Entry::default()
        },
    );
    let missing = save_login_in(
        &store,
        &cipher,
        Some("nope"),
        "https://x.com",
        "x.com",
        "a",
        "b",
        NOW,
    );
    assert_eq!(missing, Err(Code::NoValidUuidProvided));
    let wrong_kind = save_login_in(
        &store,
        &cipher,
        Some("note"),
        "https://x.com",
        "x.com",
        "a",
        "b",
        NOW,
    );
    assert_eq!(wrong_kind, Err(Code::NoValidUuidProvided));
    assert_eq!(
        stored(&store, &cipher, "note").note.as_deref(),
        Some("keep")
    );
}

#[test]
fn generate_password_needs_no_association() {
    let (extension, mut connection) = ready();
    let response = extension.send(
        &mut connection,
        "generate-password",
        json!({ "requestID": "abcdefgh" }),
    );
    assert_eq!(response["password"], "generated");
}

#[test]
fn lock_database_locks_for_an_extension_that_was_let_in() {
    let (extension, mut connection) = associated();
    let response = extension.send(&mut connection, "lock-database", json!({}));
    assert_eq!(response["success"], "true");
    assert_eq!(connection.host().locks.get(), 1);
}

#[test]
fn lock_database_is_refused_to_a_stranger() {
    let (extension, mut connection) = ready();
    let response = extension.send(&mut connection, "lock-database", json!({}));
    assert_eq!(error_code(&response), Code::AssociationFailed as u8);
    assert_eq!(connection.host().locks.get(), 0);
}

// --- access that ends -------------------------------------------------------

#[test]
fn a_forgotten_extension_loses_access_on_its_next_request() {
    let (extension, mut connection) = associated();
    let before = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com" }),
    );
    assert_eq!(before["count"], 1);

    // Settings › forget, while the connection is still up.
    connection.host().clients.borrow_mut().clear();

    let after = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com", "keys": [{ "id": "Chrome", "key": "id-key" }] }),
    );
    assert_eq!(error_code(&after), Code::AssociationFailed as u8);
    let totp = extension.send(&mut connection, "get-totp", json!({ "uuid": "gh" }));
    assert_eq!(error_code(&totp), Code::AssociationFailed as u8);
}

#[test]
fn an_association_does_not_follow_the_connection_into_another_vault() {
    // The list is the open vault's: a switch to a workspace the extension was
    // never let into reads as a vault with no clients at all.
    let (extension, mut connection) = associated();
    *connection.host().clients.borrow_mut() = vec![Client {
        name: "Chrome".into(),
        key: "a key of the other vault's".into(),
    }];
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com", "keys": [{ "id": "Chrome", "key": "id-key" }] }),
    );
    assert_eq!(error_code(&response), Code::AssociationFailed as u8);

    // Let in there too, and the key it carries opens it again.
    connection.host().clients.borrow_mut().push(Client {
        name: "Chrome".into(),
        key: "id-key".into(),
    });
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com", "keys": [{ "id": "Chrome", "key": "id-key" }] }),
    );
    assert_eq!(response["count"], 1);
}

#[test]
fn serve_ends_the_connection_once_the_host_is_switched_off() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let served = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let host = Mock::unlocked();
        host.enabled.set(false);
        server::serve(stream, host);
    });

    let mut stream = TcpStream::connect(address).unwrap();
    let request = json!({ "action": "change-public-keys", "publicKey": "x", "nonce": "y" });
    frame::write(&mut stream, &serde_json::to_vec(&request).unwrap()).unwrap();
    // No reply: the other end hangs up instead.
    assert_eq!(frame::read(&mut stream).unwrap(), None);
    served.join().unwrap();
}

#[test]
fn groups_and_unknown_actions_are_refused_in_the_protocols_words() {
    let (extension, mut connection) = associated();
    let groups = extension.send(&mut connection, "get-database-groups", json!({}));
    assert_eq!(error_code(&groups), Code::NoGroupsFound as u8);
    let entries = extension.send(&mut connection, "get-database-entries", json!({}));
    assert_eq!(error_code(&entries), Code::IncorrectAction as u8);
    assert_eq!(entries["error"], Code::IncorrectAction.message());
}

// --- framing and the wire ----------------------------------------------------

#[test]
fn frames_round_trip_and_end_cleanly() {
    let mut wire = Vec::new();
    frame::write(&mut wire, b"{\"a\":1}").unwrap();
    frame::write(&mut wire, b"").unwrap();
    let mut reader = Cursor::new(wire);
    assert_eq!(
        frame::read(&mut reader).unwrap().as_deref(),
        Some(&b"{\"a\":1}"[..])
    );
    assert_eq!(frame::read(&mut reader).unwrap().as_deref(), Some(&b""[..]));
    assert_eq!(frame::read(&mut reader).unwrap(), None);
}

#[test]
fn a_frame_over_the_cap_is_refused_before_it_is_read() {
    let mut wire = Vec::new();
    wire.extend_from_slice(&((frame::MAX_FRAME as u32) + 1).to_ne_bytes());
    assert!(frame::read(&mut Cursor::new(wire)).is_err());
    assert!(frame::write(&mut Vec::new(), &vec![0u8; frame::MAX_FRAME + 1]).is_err());
}

#[test]
fn a_frame_cut_short_is_an_error_not_an_end() {
    let mut wire = Vec::new();
    wire.extend_from_slice(&8u32.to_ne_bytes());
    wire.extend_from_slice(b"abc");
    assert!(frame::read(&mut Cursor::new(wire)).is_err());
}

#[test]
fn serve_answers_frames_on_a_stream_until_it_closes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let served = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        server::serve(stream, Mock::unlocked());
    });

    let mut stream = TcpStream::connect(address).unwrap();
    let extension = Extension::new();
    let request = json!({
        "action": "change-public-keys",
        "publicKey": extension.public_key(),
        "nonce": STANDARD.encode(nonce()),
    });
    frame::write(&mut stream, &serde_json::to_vec(&request).unwrap()).unwrap();
    let reply: Value = serde_json::from_slice(&frame::read(&mut stream).unwrap().unwrap()).unwrap();
    assert_eq!(reply["success"], "true");
    assert!(reply["publicKey"].is_string());

    drop(stream);
    served.join().unwrap();
}

// The signal registry is one per process, so the tests that register with it
// run one at a time: with both up at once, one's broadcasts would land on the
// other's connection, ahead of the frames it is counting.
static SIGNAL_TESTS: Mutex<()> = Mutex::new(());

#[test]
fn a_lock_signal_reaches_every_connection_and_drops_the_gone_ones() {
    let _one_at_a_time = SIGNAL_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let mut extension = TcpStream::connect(address).unwrap();
    let (open, _) = listener.accept().unwrap();
    let _gone_peer = TcpStream::connect(address).unwrap();
    let (gone, _) = listener.accept().unwrap();
    gone.shutdown(std::net::Shutdown::Write).unwrap();

    let open = Arc::new(Mutex::new(open));
    let gone = Arc::new(Mutex::new(gone));
    let open_id = server::register(open.clone());
    server::register(gone.clone());
    server::notify_locked();
    server::notify_unlocked();

    // In order, on the connection that reads.
    let signal: Value =
        serde_json::from_slice(&frame::read(&mut extension).unwrap().unwrap()).unwrap();
    assert_eq!(signal, json!({ "action": "database-locked" }));
    let signal: Value =
        serde_json::from_slice(&frame::read(&mut extension).unwrap().unwrap()).unwrap();
    assert_eq!(signal, json!({ "action": "database-unlocked" }));

    // The gone one's write failed on its own thread, which let the writer go.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while Arc::strong_count(&gone) > 1 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        Arc::strong_count(&gone),
        1,
        "a failed write drops the writer"
    );
    assert_eq!(Arc::strong_count(&open), 2, "a live one stays registered");

    // The connection ending lets its writer go too.
    server::forget(open_id);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while Arc::strong_count(&open) > 1 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(Arc::strong_count(&open), 1, "a forgotten one is let go");
}

#[test]
fn a_peer_that_stops_reading_stalls_no_one_else() {
    let _one_at_a_time = SIGNAL_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    // A peer that never reads: its socket buffer fills, then writes block.
    let _stalled_peer = TcpStream::connect(address).unwrap();
    let (stalled, _) = listener.accept().unwrap();
    let mut extension = TcpStream::connect(address).unwrap();
    let (open, _) = listener.accept().unwrap();

    let stalled_id = server::register(Arc::new(Mutex::new(stalled)));
    let open_id = server::register(Arc::new(Mutex::new(open)));
    // Far more than a socket buffer holds; every one of these would block a
    // shared thread on the stalled peer.
    for _ in 0..4096 {
        server::notify_locked();
    }
    server::notify_unlocked();

    // The reading peer still gets every signal, in order.
    for _ in 0..4096 {
        let signal: Value =
            serde_json::from_slice(&frame::read(&mut extension).unwrap().unwrap()).unwrap();
        assert_eq!(signal, json!({ "action": "database-locked" }));
    }
    let signal: Value =
        serde_json::from_slice(&frame::read(&mut extension).unwrap().unwrap()).unwrap();
    assert_eq!(signal, json!({ "action": "database-unlocked" }));

    server::forget(stalled_id);
    server::forget(open_id);
}

#[test]
fn bind_listens_at_the_root_and_takes_over_a_socket_left_behind() {
    use interprocess::local_socket::{prelude::*, Stream};
    let dir = tempfile::tempdir().unwrap();
    let listener = server::bind(dir.path()).expect("a listener at a fresh root");
    let mut client = Stream::connect(socket_name(dir.path()).unwrap()).unwrap();
    let mut served = listener.accept().unwrap();
    frame::write(&mut client, b"{}").unwrap();
    assert_eq!(
        frame::read(&mut served).unwrap().as_deref(),
        Some(&b"{}"[..])
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join(super::SOCKET_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "owner-only socket file");
    }

    // A crash leaves the file behind with nobody listening: the next launch
    // binds over it rather than refusing.
    std::mem::forget(listener);
    let again = server::bind(dir.path()).expect("a listener over a stale socket file");
    drop(again);
}

#[test]
fn bind_makes_a_missing_socket_directory_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir
        .path()
        .join("Group Containers")
        .join("group")
        .join("dev");
    let listener = server::bind(&root).expect("a listener in a directory not yet made");
    assert!(root.join(super::SOCKET_FILE).exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for made in [&root, root.parent().unwrap()] {
            let mode = std::fs::metadata(made).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "{} is owner-only", made.display());
        }
    }
    drop(listener);
}

#[test]
fn the_group_container_is_the_team_prefixed_one_under_the_home() {
    assert_eq!(
        group_container(std::path::Path::new("/Users/someone")),
        std::path::Path::new(
            "/Users/someone/Library/Group Containers/UFBL3F444A.app.rowel.desktop"
        )
    );
    // The iOS group is `group.`-prefixed, which macOS gates behind a profile.
    assert_ne!(DESKTOP_APP_GROUP, rowel_core::app::APP_GROUP);
    assert!(!DESKTOP_APP_GROUP.starts_with("group."));
}

#[test]
fn chrome_and_firefox_keep_the_data_directory_socket() {
    // The debug override wins; a run that sets it has nothing to say about
    // the platform default.
    if std::env::var_os("ROWEL_DB_DIR").is_some() {
        return;
    }
    // Where it always was, on every platform, macOS included: the app-data
    // directory under the identifier, `dev` in a debug (`cargo test`) build.
    assert_eq!(
        root_dir(),
        Some(dirs::data_dir().unwrap().join(IDENTIFIER).join("dev"))
    );
}

#[test]
fn the_safari_socket_is_in_the_group_container_while_its_path_fits() {
    use std::path::Path;
    let home = |len: usize| format!("/Users/{}", "a".repeat(len));
    // `sun_path` is 104 bytes on macOS, the NUL included, and the group
    // container's path is long: a short name of up to 29 characters fits a
    // release build's socket, 25 a debug build's (`dev/`).
    for (debug, fits) in [(false, 29), (true, 25)] {
        let dir = group_socket_dir(Path::new(&home(fits)), debug).expect("the longest that fits");
        let expected = group_container(Path::new(&home(fits)));
        assert_eq!(
            dir,
            if debug {
                expected.join("dev")
            } else {
                expected
            }
        );
        assert_eq!(
            dir.join(super::SOCKET_FILE).as_os_str().len() + 1,
            SUN_PATH_MAX
        );

        let too_long = home(fits + 1);
        match group_socket_dir(Path::new(&too_long), debug) {
            Err(SafariSocketError::TooLong { path, bytes }) => {
                assert_eq!(bytes, SUN_PATH_MAX + 1);
                assert!(path.ends_with(super::SOCKET_FILE));
                assert!(path.starts_with(&too_long));
            }
            other => panic!("a short name of {} characters: {other:?}", fits + 1),
        }
    }
}

#[test]
fn the_app_group_is_the_one_the_app_is_entitled_to() {
    let entitlements = include_str!("../../Entitlements.plist");
    assert!(entitlements.contains("<key>com.apple.security.application-groups</key>"));
    assert!(entitlements.contains(&format!("<string>{DESKTOP_APP_GROUP}</string>")));
}

// The Safari extension is Swift (src-tauri/gen/safari) and has to agree with
// this side on where the socket is and how a frame is capped: held to the
// constants here, since nothing else would notice the two drifting apart.
#[test]
fn the_safari_extension_agrees_on_the_group_the_socket_and_the_cap() {
    let entitlements = include_str!("../../gen/safari/rowel_safari/rowel_safari.entitlements");
    assert!(entitlements.contains(&format!("<string>{DESKTOP_APP_GROUP}</string>")));
    let connection = include_str!("../../gen/safari/Sources/HostConnection.swift");
    assert!(connection.contains(&format!("appGroup = \"{DESKTOP_APP_GROUP}\"")));
    assert!(connection.contains(&format!("socketFile = \"{}\"", super::SOCKET_FILE)));
    let framing = include_str!("../../gen/safari/Sources/Frame.swift");
    assert_eq!(frame::MAX_FRAME, 1024 * 1024);
    assert!(framing.contains("maxLength = 1024 * 1024"));
}

#[test]
fn a_browser_launch_is_told_by_what_it_puts_on_the_command_line() {
    fn launched(list: &[&str]) -> bool {
        proxy::launched_by_browser(list.iter().map(|s| s.to_string()))
    }
    assert!(launched(&[
        "chrome-extension://oboonakemofpalcgghocfoadofidjkkk/"
    ]));
    assert!(launched(&["chrome-extension://abc/", "--parent-window=0"]));
    assert!(launched(&[
        "/path/manifest.json",
        "keepassxc-browser@keepassxc.org"
    ]));
    assert!(launched(&["/path/manifest.json", "browser@rowel.app"]));
    assert!(!launched(&["/path/manifest.json", "other@example.org"]));
    assert!(!launched(&[]));
    assert!(!launched(&["/Users/me/backup.rowel"]));
}

#[test]
fn the_socket_name_follows_the_root() {
    let a = socket_name(std::path::Path::new("/tmp/rowel-a")).unwrap();
    let again = socket_name(std::path::Path::new("/tmp/rowel-a")).unwrap();
    let b = socket_name(std::path::Path::new("/tmp/rowel-b")).unwrap();
    assert_eq!(format!("{a:?}"), format!("{again:?}"));
    assert_ne!(format!("{a:?}"), format!("{b:?}"));
}

// --- manifests ---------------------------------------------------------------

#[test]
fn the_identifier_is_the_one_tauri_builds_with() {
    let conf: Value = serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
    assert_eq!(conf["identifier"], IDENTIFIER);
}

#[test]
fn rowels_manifest_names_its_own_host_and_extension() {
    let exe = std::path::Path::new("/Applications/Rowel.app/Contents/MacOS/rowel");
    let chromium: Value =
        serde_json::from_str(&manifest::manifest(&ROWEL, Family::Chromium, exe)).unwrap();
    assert_eq!(chromium["name"], "app.rowel.browser");
    assert_eq!(chromium["type"], "stdio");
    assert_eq!(chromium["path"], exe.to_string_lossy().as_ref());
    assert_eq!(
        chromium["allowed_origins"],
        json!([
            "chrome-extension://dimghkhcdfaokfingegmgbnpnpcoeofj/",
            "chrome-extension://aajfpjaphnegnekpggjnocmgbhkabeke/"
        ])
    );
    assert!(chromium.get("allowed_extensions").is_none());

    let firefox: Value =
        serde_json::from_str(&manifest::manifest(&ROWEL, Family::Firefox, exe)).unwrap();
    assert_eq!(firefox["name"], "app.rowel.browser");
    assert_eq!(firefox["allowed_extensions"], json!(["browser@rowel.app"]));
    assert!(firefox.get("allowed_origins").is_none());
}

#[test]
fn the_compatibility_manifest_names_keepassxcs_host_and_the_stock_extension() {
    let exe = std::path::Path::new("/Applications/Rowel.app/Contents/MacOS/rowel");
    let chromium: Value =
        serde_json::from_str(&manifest::manifest(&KEEPASSXC, Family::Chromium, exe)).unwrap();
    assert_eq!(chromium["name"], "org.keepassxc.keepassxc_browser");
    assert_eq!(chromium["path"], exe.to_string_lossy().as_ref());
    assert_eq!(
        chromium["allowed_origins"],
        json!([
            "chrome-extension://oboonakemofpalcgghocfoadofidjkkk/",
            "chrome-extension://pdffhmdngciaglkoonimfcmckehcpafo/"
        ])
    );
    assert!(chromium.get("allowed_extensions").is_none());

    let firefox: Value =
        serde_json::from_str(&manifest::manifest(&KEEPASSXC, Family::Firefox, exe)).unwrap();
    assert_eq!(
        firefox["allowed_extensions"],
        json!(["keepassxc-browser@keepassxc.org"])
    );
    assert!(firefox.get("allowed_origins").is_none());
}

#[test]
fn a_manifest_in_place_is_told_apart_from_keepassxcs_own() {
    use manifest::{classify, Found};
    let exe = std::path::Path::new("/Applications/Rowel.app/Contents/MacOS/rowel");
    let ours = manifest::manifest(&KEEPASSXC, Family::Chromium, exe);
    assert_eq!(classify(&KEEPASSXC, &ours, exe), Found::Current);

    let moved = std::path::Path::new("/opt/rowel/rowel");
    assert_eq!(classify(&KEEPASSXC, &ours, moved), Found::Stale);

    assert_eq!(classify(&KEEPASSXC, &keepassxcs_own(), exe), Found::Foreign);
    assert_eq!(classify(&KEEPASSXC, "not json", exe), Found::Foreign);

    let rowel = manifest::manifest(&ROWEL, Family::Chromium, exe);
    assert_eq!(classify(&ROWEL, &rowel, exe), Found::Current);
}

// The places a browser reads manifests from, as a directory: one folder per
// browser, a `<host name>.json` in each. Browsers not `present` are not here.
struct Places {
    dir: tempfile::TempDir,
    present: Vec<&'static str>,
}

impl Places {
    fn new(present: &[&'static str]) -> Self {
        Places {
            dir: tempfile::tempdir().unwrap(),
            present: present.to_vec(),
        }
    }

    fn path(&self, host: &manifest::NativeHost, browser: &str) -> std::path::PathBuf {
        self.dir
            .path()
            .join(browser)
            .join(format!("{}.json", host.name))
    }

    fn read(&self, host: &manifest::NativeHost, browser: &str) -> Option<String> {
        std::fs::read_to_string(self.path(host, browser)).ok()
    }

    fn put(&self, host: &manifest::NativeHost, browser: &str, text: &str) {
        let path = self.path(host, browser);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn status(&self, exe: &std::path::Path, compat: bool, browser: &str) -> manifest::Status {
        manifest::status_in(self, Some(exe), compat)
            .into_iter()
            .find(|status| status.id == browser)
            .unwrap()
    }
}

impl manifest::Places for Places {
    fn detected(&self, browser: &manifest::Browser) -> bool {
        self.present.contains(&browser.id)
    }

    fn existing(&self, host: &manifest::NativeHost, browser: &manifest::Browser) -> Option<String> {
        self.read(host, browser.id)
    }

    fn install(
        &self,
        host: &manifest::NativeHost,
        browser: &manifest::Browser,
        manifest: &str,
    ) -> std::io::Result<()> {
        self.put(host, browser.id, manifest);
        Ok(())
    }

    fn remove(
        &self,
        host: &manifest::NativeHost,
        browser: &manifest::Browser,
    ) -> std::io::Result<()> {
        match std::fs::remove_file(self.path(host, browser.id)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

fn keepassxcs_own() -> String {
    json!({
        "name": KEEPASSXC.name,
        "description": "KeePassXC integration with native messaging support",
        "path": "/Applications/KeePassXC.app/Contents/MacOS/keepassxc-proxy",
        "type": "stdio",
        "allowed_origins": ["chrome-extension://oboonakemofpalcgghocfoadofidjkkk/"]
    })
    .to_string()
}

const EXE: &str = "/Applications/Rowel.app/Contents/MacOS/rowel";

#[test]
fn install_writes_both_names_with_compatibility_on() {
    let exe = std::path::Path::new(EXE);
    let places = Places::new(&["chrome", "firefox"]);

    manifest::install_in(&places, Some(exe), true);

    for (browser, family) in [("chrome", Family::Chromium), ("firefox", Family::Firefox)] {
        assert_eq!(
            places.read(&ROWEL, browser),
            Some(manifest::manifest(&ROWEL, family, exe))
        );
        assert_eq!(
            places.read(&KEEPASSXC, browser),
            Some(manifest::manifest(&KEEPASSXC, family, exe))
        );
        let status = places.status(exe, true, browser);
        assert!(status.installed);
        assert!(!status.conflict);
    }
    // A browser not on this machine is not registered with.
    assert_eq!(places.read(&ROWEL, "edge"), None);
    assert!(!places.status(exe, true, "edge").installed);
}

#[test]
fn install_writes_only_rowels_name_with_compatibility_off() {
    let exe = std::path::Path::new(EXE);
    let places = Places::new(&["chrome"]);

    manifest::install_in(&places, Some(exe), false);

    assert!(places.read(&ROWEL, "chrome").is_some());
    assert_eq!(places.read(&KEEPASSXC, "chrome"), None);
    assert!(places.status(exe, false, "chrome").installed);
}

#[test]
fn turning_compatibility_off_takes_back_only_the_keepassxc_manifest() {
    let exe = std::path::Path::new(EXE);
    let places = Places::new(&["chrome"]);
    manifest::install_in(&places, Some(exe), true);

    manifest::install_in(&places, Some(exe), false);

    assert!(places.read(&ROWEL, "chrome").is_some());
    assert_eq!(places.read(&KEEPASSXC, "chrome"), None);
}

// The layout from before Rowel had a name of its own: one manifest, under
// KeePassXC's name, for an executable since moved. It is rewritten with
// compatibility on and taken back with it off, never left pointing at nothing.
#[test]
fn a_manifest_from_before_the_rename_is_rewritten_or_taken_back() {
    let exe = std::path::Path::new(EXE);
    let old = manifest::manifest(
        &KEEPASSXC,
        Family::Chromium,
        std::path::Path::new("/opt/rowel/rowel"),
    );

    let places = Places::new(&["chrome"]);
    places.put(&KEEPASSXC, "chrome", &old);
    manifest::install_in(&places, Some(exe), true);
    assert_eq!(
        places.read(&KEEPASSXC, "chrome"),
        Some(manifest::manifest(&KEEPASSXC, Family::Chromium, exe))
    );

    let places = Places::new(&["chrome"]);
    places.put(&KEEPASSXC, "chrome", &old);
    manifest::install_in(&places, Some(exe), false);
    assert_eq!(places.read(&KEEPASSXC, "chrome"), None);
    assert!(places.read(&ROWEL, "chrome").is_some());
}

#[test]
fn keepassxcs_own_manifest_is_left_alone_and_reported_only_with_compatibility_on() {
    let exe = std::path::Path::new(EXE);
    let places = Places::new(&["chrome"]);
    places.put(&KEEPASSXC, "chrome", &keepassxcs_own());

    manifest::install_in(&places, Some(exe), true);
    assert_eq!(places.read(&KEEPASSXC, "chrome"), Some(keepassxcs_own()));
    let status = places.status(exe, true, "chrome");
    // Rowel's own extension still reaches it: the clash is KeePassXC-Browser's.
    assert!(status.installed);
    assert!(status.conflict);

    manifest::install_in(&places, Some(exe), false);
    assert_eq!(places.read(&KEEPASSXC, "chrome"), Some(keepassxcs_own()));
    assert!(!places.status(exe, false, "chrome").conflict);

    manifest::remove_in(&places, Some(exe));
    assert_eq!(places.read(&KEEPASSXC, "chrome"), Some(keepassxcs_own()));
}

#[test]
fn remove_takes_back_both_names_from_every_browser() {
    let exe = std::path::Path::new(EXE);
    let mut places = Places::new(&["chrome", "firefox"]);
    manifest::install_in(&places, Some(exe), true);
    // Written while the browser was here; it has gone since.
    places.present.clear();

    manifest::remove_in(&places, Some(exe));

    for browser in ["chrome", "firefox"] {
        assert_eq!(places.read(&ROWEL, browser), None);
        assert_eq!(places.read(&KEEPASSXC, browser), None);
        assert!(!places.status(exe, true, browser).installed);
    }
}

// Windows' flat manifest from before the rename, `<data>/browser/<id>.json`:
// kept while KeePassXC's registry value names it — however that value spells
// the path — and deleted once the value names anything else, or is gone.
#[test]
fn the_legacy_windows_manifest_goes_only_once_the_registry_stops_naming_it() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("browser").join("chrome.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, "{}").unwrap();
    let spelled = legacy.to_string_lossy().into_owned();

    for named in [
        spelled.clone(),
        spelled.to_uppercase(),
        spelled.replace('/', "\\"),
        format!("{spelled}\\"),
    ] {
        manifest::remove_unless_named(&legacy, Some(&named)).unwrap();
        assert!(
            legacy.exists(),
            "deleted while the registry named it as {named}"
        );
    }

    let rewritten = dir
        .path()
        .join("browser")
        .join("chrome")
        .join(format!("{}.json", KEEPASSXC.name));
    manifest::remove_unless_named(&legacy, Some(&rewritten.to_string_lossy())).unwrap();
    assert!(!legacy.exists());

    // Already gone, and no key left: nothing to do, and no error.
    manifest::remove_unless_named(&legacy, None).unwrap();

    std::fs::write(&legacy, "{}").unwrap();
    manifest::remove_unless_named(&legacy, None).unwrap();
    assert!(!legacy.exists());
}

#[test]
fn every_browser_has_a_place_on_every_platform() {
    for browser in manifest::BROWSERS {
        assert!(!browser.id.is_empty());
        assert!(!browser.label.is_empty());
    }
    assert_eq!(str_of(&Map::new(), "missing"), "");
}
