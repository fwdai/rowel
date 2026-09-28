use std::cell::{Cell, RefCell};
use std::io::Cursor;
use std::net::{TcpListener, TcpStream};

use base64::{engine::general_purpose::STANDARD, Engine};
use crypto_box::{aead::Aead, Nonce, PublicKey, SalsaBox, SecretKey};
use serde_json::{json, Map, Value};

use super::actions::{host_matches, site_host, Client, Connection, Host, Login};
use super::manifest::{self, Family, HOST_NAME};
use super::protocol::{increment, str_of, Code, NONCE_LEN, VERSION};
use super::{frame, proxy, server, socket_name, Pending, IDENTIFIER};

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
// host answers every action through the 2.7.0 row of that table and none of
// the passkey actions, so it claims the release just before those.
#[test]
fn the_version_claimed_turns_on_nothing_this_host_lacks() {
    assert_eq!(VERSION, "2.7.6");
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

// --- matching ----------------------------------------------------------------

#[test]
fn site_host_is_the_lowercase_host_of_a_url() {
    assert_eq!(
        site_host("https://Accounts.Google.com/signin?x=1").as_deref(),
        Some("accounts.google.com")
    );
    assert_eq!(
        site_host("http://192.168.1.1/").as_deref(),
        Some("192.168.1.1")
    );
    assert_eq!(site_host("github.com"), None);
    assert_eq!(site_host(""), None);
}

#[test]
fn a_login_matches_its_host_and_subdomains_but_not_its_lookalikes() {
    assert!(host_matches("github.com", "github.com"));
    assert!(host_matches("github.com", "GitHub.com"));
    assert!(host_matches("www.github.com", "github.com"));
    assert!(host_matches("github.com", "www.github.com"));
    assert!(host_matches("accounts.google.com", "google.com"));
    assert!(!host_matches("google.com", "accounts.google.com"));
    assert!(!host_matches("notgithub.com", "github.com"));
    assert!(!host_matches("github.com", ""));
    assert!(!host_matches("github.com", "  "));
    // A public suffix is not a parent: a login stored under one — a malformed
    // website, an import — is served to no site beneath it.
    assert!(!host_matches("github.com", "com"));
    assert!(!host_matches("bank.co.uk", "co.uk"));
    assert!(
        host_matches("co.uk", "co.uk"),
        "the suffix itself, as a site, still is"
    );
    assert!(host_matches("online.bank.co.uk", "bank.co.uk"));
}

#[test]
fn a_login_matches_whatever_else_its_website_field_carried() {
    // The column is the website cut before its first `/`, so all of these
    // are what a typed URL leaves in it.
    assert!(host_matches("example.com", "example.com:8443"));
    assert!(host_matches("example.com", "user@example.com"));
    assert!(host_matches("example.com", "user:pw@example.com:8443"));
    assert!(host_matches("example.com", "example.com?next=1"));
    assert!(host_matches("example.com", "example.com#top"));
    assert!(!host_matches("example.com", "example.com:notaport"));
    assert!(host_matches("192.168.1.1", "192.168.1.1:8080"));
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
fn a_manifest_names_the_host_and_the_extension_for_its_family() {
    let exe = std::path::Path::new("/Applications/Rowel.app/Contents/MacOS/rowel");
    let chromium: Value = serde_json::from_str(&manifest::manifest(Family::Chromium, exe)).unwrap();
    assert_eq!(chromium["name"], HOST_NAME);
    assert_eq!(chromium["type"], "stdio");
    assert_eq!(chromium["path"], exe.to_string_lossy().as_ref());
    assert_eq!(
        chromium["allowed_origins"][0],
        "chrome-extension://oboonakemofpalcgghocfoadofidjkkk/"
    );
    assert!(chromium.get("allowed_extensions").is_none());

    let firefox: Value = serde_json::from_str(&manifest::manifest(Family::Firefox, exe)).unwrap();
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
    let ours = manifest::manifest(Family::Chromium, exe);
    assert_eq!(classify(&ours, exe), Found::Current);

    let moved = std::path::Path::new("/opt/rowel/rowel");
    assert_eq!(classify(&ours, moved), Found::Stale);

    let keepassxc = json!({
        "name": HOST_NAME,
        "description": "KeePassXC integration with native messaging support",
        "path": "/Applications/KeePassXC.app/Contents/MacOS/keepassxc-proxy",
        "type": "stdio",
        "allowed_origins": ["chrome-extension://oboonakemofpalcgghocfoadofidjkkk/"]
    });
    assert_eq!(classify(&keepassxc.to_string(), exe), Found::Foreign);
    assert_eq!(classify("not json", exe), Found::Foreign);
}

#[test]
fn every_browser_has_a_place_on_every_platform() {
    for browser in manifest::BROWSERS {
        assert!(!browser.id.is_empty());
        assert!(!browser.label.is_empty());
    }
    assert_eq!(str_of(&Map::new(), "missing"), "");
}
