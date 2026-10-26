use std::cell::RefCell;
use std::io::Cursor;

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Map, Value};
use zeroize::Zeroizing;

use crate::actions::{Client, ClientKind, Connection, Hello, Host, Login, Proof, VisibleRef};
use crate::client::{Peer, PeerError};
use crate::frame;
use crate::grant::{
    GrantReply, GrantRequest, ItemSelector, Mode, ResolveContext, Target, VisibleItem,
};
use crate::lockbox::save_login_in;
use crate::passkeys::{Assertion, Registration};
use crate::protocol::{increment, Code, NONCE_LEN, VERSION};
use crate::reference::Reference;
use crate::signal::{Signal, ROWEL_SIGNAL};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::socket::{root_dir, socket_name, IDENTIFIER};
use crate::testing::{
    error_code, login, nonce, raw, ready, seed, stored, vault, Extension, GrantAnswer, Mock,
    GRANT_EXPIRES_AT_MS, GRANT_ID, LOCKBOX,
};
use rowel_core::models::{Entry, PasswordHistoryItem};
use rowel_core::store::VaultStore;

// Passkeys through the extension, end to end.
mod ceremonies;

// --- what the host claims to be --------------------------------------------------

// The extension turns features on by the version alone (see `VERSION`): this
// host answers the passkey actions, so it claims the release that turns them
// on — and not the one after, whose default passkey *group* the vault has no
// groups to honour.
#[test]
fn the_version_claimed_turns_on_nothing_this_host_lacks() {
    assert_eq!(VERSION, "2.7.7");
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
    // `exchange` checks the nonce and `success` on the way in; what is left
    // to see is the key, and that a sealed request opens afterwards.
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock::unlocked());
    let response = extension.exchange(&mut connection);
    assert_eq!(response["success"], "true");
    assert_eq!(response["version"], VERSION);
    assert!(response["publicKey"].is_string());
    let sealed = extension.send(&mut connection, "get-databasehash", json!({}));
    assert_eq!(sealed["hash"], "abc123");
}

#[test]
fn a_code_is_found_by_its_number_and_an_unknown_number_is_none() {
    assert_eq!(Code::from_number(8), Some(Code::AssociationFailed));
    assert_eq!(Code::from_number(43), Some(Code::NotPermittedForKind));
    assert_eq!(Code::from_number(46), Some(Code::AgentsDisabled));
    assert_eq!(
        Code::from_number(2),
        None,
        "a KeePassXC code this host never raises"
    );
    assert_eq!(Code::from_number(99), None);
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
    let (mut request, _) = extension
        .channel()
        .seal("test", "lock-database", json!({}))
        .unwrap();
    request["action"] = "get-databasehash".into();
    let response = raw(&mut connection, &request);
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

/// A stored extension as the scripted host writes one.
fn extension_client(name: &str, key: &str) -> Client {
    Client {
        name: name.into(),
        key: key.into(),
        kind: ClientKind::Extension,
        platform: None,
        created_at_ms: 0,
        last_seen_at_ms: None,
        agent_id: None,
        driven_by_agent: false,
    }
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
    // No `hello`: the reply KeePassXC-Browser knows, plus what this broker
    // can do beyond it — and no `clientId`, which is the agent protocol's.
    assert_eq!(
        response["brokerCapabilities"],
        json!(["grants", "signals", "resolve"])
    );
    assert_eq!(
        sorted_keys(&response),
        [
            "action",
            "brokerCapabilities",
            "hash",
            "id",
            "nonce",
            "success",
            "version"
        ]
    );
    assert_eq!(
        connection.host().clients.borrow().as_slice(),
        &[extension_client("Chrome", "id-key")]
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

// --- kinds: who a client is, and what the record says ----------------------------

// The rows in a lockbox from before there were kinds are `{"name","key"}`;
// they must read back as what they were, extensions, and the full record must
// survive a trip through the store.
#[test]
fn a_client_row_from_before_kinds_reads_as_an_extension() {
    let old: Client = serde_json::from_str(r#"{"name":"Chrome","key":"id-key"}"#).unwrap();
    assert_eq!(old, extension_client("Chrome", "id-key"));
    assert_eq!(old.kind, ClientKind::Extension);
    assert_eq!(old.created_at_ms, 0);
    assert_eq!(
        serde_json::to_value(&old).unwrap(),
        json!({ "name": "Chrome", "key": "id-key", "kind": "extension", "createdAtMs": 0 }),
        "nothing it did not have is written back as a value"
    );

    let agent = Client {
        name: "Claude".into(),
        key: "agent-key".into(),
        kind: ClientKind::Agent,
        platform: Some("claude-code".into()),
        created_at_ms: 1_700_000_000_000,
        last_seen_at_ms: Some(1_700_000_001_000),
        agent_id: Some("def-1".into()),
        driven_by_agent: false,
    };
    let wire = serde_json::to_value(&agent).unwrap();
    assert_eq!(
        wire,
        json!({
            "name": "Claude", "key": "agent-key", "kind": "agent", "platform": "claude-code",
            "createdAtMs": 1_700_000_000_000i64, "lastSeenAtMs": 1_700_000_001_000i64, "agentId": "def-1"
        })
    );
    assert_eq!(serde_json::from_value::<Client>(wire).unwrap(), agent);
}

#[test]
fn a_hello_reads_with_every_field_optional() {
    let bare: Hello = serde_json::from_value(json!({})).unwrap();
    assert_eq!(bare, Hello::default());
    assert_eq!(bare.kind, ClientKind::Extension);
    let agent: Hello =
        serde_json::from_value(json!({ "kind": "agent", "platform": "claude-code" })).unwrap();
    assert_eq!(
        agent,
        Hello {
            kind: ClientKind::Agent,
            platform: Some("claude-code".into()),
            version: None,
            capabilities: Vec::new(),
            proof: None,
        }
    );
    assert!(serde_json::from_value::<Hello>(json!({ "kind": "robot" })).is_err());
}

#[test]
fn associate_with_a_hello_stores_the_kind_and_platform_and_names_the_client() {
    let (extension, mut connection) = ready();
    let response = extension.send(
        &mut connection,
        "associate",
        json!({
            "key": extension.public_key(),
            "idKey": "agent-key",
            "hello": { "kind": "agent", "platform": "claude-code", "version": "1.2.0", "capabilities": ["grants"] },
        }),
    );
    assert_eq!(response["id"], "Chrome");
    assert_eq!(response["hash"], "abc123");
    assert_eq!(
        response["brokerCapabilities"],
        json!(["grants", "signals", "resolve"])
    );
    assert_eq!(response["clientId"], "agent-key");
    assert_eq!(
        sorted_keys(&response),
        [
            "action",
            "brokerCapabilities",
            "clientId",
            "hash",
            "id",
            "nonce",
            "success",
            "version"
        ]
    );
    let stored = connection.host().clients.borrow();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].kind, ClientKind::Agent);
    assert_eq!(stored[0].platform.as_deref(), Some("claude-code"));
    assert_eq!(stored[0].key, "agent-key");
}

/// The scripted host, remembering every `Hello` an `associate` brought it.
/// `Mock` keeps only what it stores of one, and a proof is read by a server
/// rather than stored, so this is the one way to see it arrive whole.
struct Recording {
    inner: Mock,
    hellos: RefCell<Vec<Hello>>,
}

impl Host for Recording {
    fn enabled(&self) -> bool {
        self.inner.enabled()
    }
    fn database_hash(&self) -> Option<String> {
        self.inner.database_hash()
    }
    fn clients(&self) -> Vec<Client> {
        self.inner.clients()
    }
    fn associate(&self, key: &str, who: &Hello) -> Result<Client, Code> {
        self.hellos.borrow_mut().push(who.clone());
        self.inner.associate(key, who)
    }
    fn client_seen(&self, key: &str) {
        self.inner.client_seen(key)
    }
    fn logins_for(&self, page_url: &str, host: &str) -> Vec<Login> {
        self.inner.logins_for(page_url, host)
    }
    fn totp(&self, id: &str) -> Option<String> {
        self.inner.totp(id)
    }
    fn generate_password(&self) -> Option<String> {
        self.inner.generate_password()
    }
    fn save_login(
        &self,
        id: Option<&str>,
        url: &str,
        host: &str,
        username: &str,
        password: &str,
    ) -> Result<(), Code> {
        self.inner.save_login(id, url, host, username, password)
    }
    fn passkey_register(&self, client: &Client, registration: Registration) -> Result<Value, Code> {
        self.inner.passkey_register(client, registration)
    }
    fn passkey_get(&self, client: &Client, assertion: Assertion) -> Result<Value, Code> {
        self.inner.passkey_get(client, assertion)
    }
    fn lock(&self) {
        self.inner.lock()
    }
    fn unlock_requested(&self) {
        self.inner.unlock_requested()
    }
    fn request_grant(&self, client: &Client, req: GrantRequest) -> Result<GrantReply, Code> {
        self.inner.request_grant(client, req)
    }
    fn wait_grant(&self, client: &Client, id: &str, timeout_ms: u64) -> Result<GrantReply, Code> {
        self.inner.wait_grant(client, id, timeout_ms)
    }
    fn grant_status(&self, client: &Client, id: &str) -> Result<GrantReply, Code> {
        self.inner.grant_status(client, id)
    }
    fn end_grant(&self, client: &Client, id: &str) -> Result<(), Code> {
        self.inner.end_grant(client, id)
    }
    fn visible_items(
        &self,
        client: &Client,
        lockbox: Option<&str>,
    ) -> Result<Vec<VisibleRef>, Code> {
        self.inner.visible_items(client, lockbox)
    }
    fn take_signals(&self, client: &Client) -> Vec<Signal> {
        self.inner.take_signals(client)
    }
    fn grant_covers(&self, client: &Client, grant_id: &str, item_id: &str, mode: Mode) -> bool {
        self.inner.grant_covers(client, grant_id, item_id, mode)
    }
    fn client_grant_covers(&self, client: &Client, item_id: &str, mode: Mode) -> bool {
        self.inner.client_grant_covers(client, item_id, mode)
    }
    fn report_exposure(&self, client: &Client, grant_id: &str) {
        self.inner.report_exposure(client, grant_id)
    }
    fn resolve(
        &self,
        client: &Client,
        grant_id: &str,
        reference: &Reference,
        ctx: &ResolveContext,
    ) -> Result<Zeroizing<String>, Code> {
        self.inner.resolve(client, grant_id, reference, ctx)
    }
}

// A workload identity proof (`spec/02`) rides inside the hello; the broker
// reads it strictly like the rest and carries it to the host untouched, for
// a server to verify.
#[test]
fn a_proof_inside_the_hello_reaches_the_host_whole() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Recording {
        inner: Mock::unlocked(),
        hellos: RefCell::new(Vec::new()),
    });
    extension.exchange(&mut connection);
    let response = extension.send(
        &mut connection,
        "associate",
        json!({
            "key": extension.public_key(),
            "idKey": "agent-key",
            "hello": {
                "kind": "agent",
                "platform": "cli",
                "proof": { "kind": "oidc", "token": "eyJhbGciOi.claims.signature", "nonce": "n-1" },
            },
        }),
    );
    assert_eq!(response["clientId"], "agent-key");
    let hellos = connection.host().hellos.borrow();
    assert_eq!(hellos.len(), 1);
    assert_eq!(hellos[0].kind, ClientKind::Agent);
    assert_eq!(
        hellos[0].proof,
        Some(Proof {
            kind: "oidc".into(),
            token: "eyJhbGciOi.claims.signature".into(),
            nonce: Some("n-1".into()),
        })
    );
}

// A client from before proofs sends none and reads as none, and none is
// what goes on the wire for it; a proof that is shown — a `Hello` is
// logged — never shows the token, which is the credential.
#[test]
fn a_hello_without_a_proof_has_none_and_a_proof_is_shown_without_its_token() {
    let hello: Hello =
        serde_json::from_value(json!({ "kind": "agent", "platform": "cli" })).unwrap();
    assert_eq!(hello.proof, None);
    assert_eq!(serde_json::to_value(&hello).unwrap().get("proof"), None);

    let proof = Proof {
        kind: "oidc".into(),
        token: "eyJhbGciOi.secret-claims.signature".into(),
        nonce: None,
    };
    let shown = format!("{proof:?}");
    assert!(shown.contains("oidc"));
    assert!(!shown.contains("secret-claims"));
    let hello = Hello {
        proof: Some(proof),
        ..Hello::default()
    };
    assert!(!format!("{hello:?}").contains("secret-claims"));
}

#[test]
fn a_hello_that_does_not_read_fails_the_association() {
    let (extension, mut connection) = ready();
    for hello in [json!("agent"), json!({ "kind": "robot" }), json!(42)] {
        let response = extension.send(
            &mut connection,
            "associate",
            json!({ "key": extension.public_key(), "idKey": "id-key", "hello": hello }),
        );
        assert_eq!(
            error_code(&response),
            Code::AssociationFailed as u8,
            "{hello}"
        );
    }
    assert!(connection.host().clients.borrow().is_empty());
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com" }),
    );
    assert_eq!(error_code(&response), Code::AssociationFailed as u8);
}

// What an agent says about itself goes into a stored row and a dialog line,
// so each field is cut to a length a line can hold — cut, not refused, since
// none of it decides anything.
#[test]
fn what_a_hello_says_is_bounded() {
    let (extension, mut connection) = ready();
    extension.send(
        &mut connection,
        "associate",
        json!({
            "key": extension.public_key(),
            "idKey": "agent-key",
            "hello": { "kind": "agent", "platform": "p".repeat(100) },
        }),
    );
    let stored = connection.host().clients.borrow();
    assert_eq!(stored[0].platform.as_deref(), Some("p".repeat(64).as_str()));
}

#[test]
fn an_agent_is_refused_at_the_door_while_agents_are_off() {
    let (extension, mut connection) = ready();
    connection.host().agents_enabled.set(false);
    let response = extension.send(
        &mut connection,
        "associate",
        json!({ "key": extension.public_key(), "idKey": "agent-key", "hello": { "kind": "agent" } }),
    );
    assert_eq!(error_code(&response), Code::AgentsDisabled as u8);
    assert!(connection.host().clients.borrow().is_empty());

    // An extension is let in regardless: the switch is about agents.
    let response = extension.send(
        &mut connection,
        "associate",
        json!({ "key": extension.public_key(), "idKey": "id-key" }),
    );
    assert_eq!(response["id"], "Chrome");
}

#[test]
fn test_associate_tells_the_host_the_client_was_seen() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock::unlocked().known("Chrome", "id-key").known_agent(
        "Claude",
        "agent-key",
        "claude-code",
    ));
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "other" }),
    );
    assert!(
        connection.host().seen.borrow().is_empty(),
        "a key the vault does not hold was not seen"
    );

    let right = extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Claude", "key": "agent-key", "hello": { "kind": "agent", "platform": "claude-code" } }),
    );
    assert_eq!(right["id"], "Claude");
    assert_eq!(right["clientId"], "agent-key");
    assert_eq!(
        right["brokerCapabilities"],
        json!(["grants", "signals", "resolve"])
    );
    assert_eq!(connection.host().seen.borrow().as_slice(), ["agent-key"]);

    let malformed = extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "id-key", "hello": "browser" }),
    );
    assert_eq!(error_code(&malformed), Code::AssociationFailed as u8);
    assert_eq!(connection.host().seen.borrow().len(), 1);
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

// --- fills in agent mode: every fill is a grant ----------------------------------

const GH_LOGIN: &str = "https://github.com/login";

fn two_logins() -> Vec<Login> {
    vec![
        login("gh", "GitHub", "octocat", "hunter2", Some("123456")),
        login("gh2", "GitHub (work)", "octocorp", "s3cret", None),
    ]
}

/// Associated, with two logins for GitHub and the host scripted to `answer`.
fn agent_connection(answer: GrantAnswer) -> (Extension, Connection<Mock>) {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock {
        logins: two_logins(),
        grant_answer: answer,
        ..Mock::unlocked().known("Chrome", "id-key")
    });
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "id-key" }),
    );
    (extension, connection)
}

fn sorted_keys(response: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = response
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

// The reply to a `get-logins` without the new fields has exactly the fields
// it had, and nobody is asked anything — whatever the host would answer.
#[test]
fn get_logins_without_the_agent_fields_is_what_it_always_was() {
    let (extension, mut connection) = agent_connection(GrantAnswer::Active("gh2".into()));
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "submitUrl": "https://github.com/session" }),
    );
    assert_eq!(response["count"], 2);
    assert_eq!(response["hash"], "abc123");
    assert_eq!(
        response["entries"],
        json!([
            { "login": "octocat", "password": "hunter2", "name": "GitHub", "uuid": "gh", "totp": "123456" },
            { "login": "octocorp", "password": "s3cret", "name": "GitHub (work)", "uuid": "gh2" },
        ])
    );
    assert_eq!(
        sorted_keys(&response),
        ["action", "count", "entries", "hash", "nonce", "success", "version"]
    );
    assert!(connection.host().grant_requests.borrow().is_empty());
}

#[test]
fn in_agent_mode_a_fill_is_a_grant_and_only_the_admitted_login_is_served() {
    let (extension, mut connection) = agent_connection(GrantAnswer::Active("gh2".into()));
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "agentMode": "true" }),
    );
    assert_eq!(response["count"], 1);
    assert_eq!(response["grantId"], GRANT_ID);
    assert_eq!(response["hash"], "abc123");
    assert_eq!(
        response["entries"],
        json!([{ "login": "octocorp", "password": "s3cret", "name": "GitHub (work)", "uuid": "gh2" }])
    );
    assert_eq!(
        sorted_keys(&response),
        ["action", "count", "entries", "grantId", "hash", "nonce", "success", "version"]
    );

    // What the host was asked: by the connection's key, for the page's
    // logins as candidates, at the page, to fill.
    let asked = connection.host().grant_requests.borrow();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].0, "id-key");
    assert_eq!(
        asked[0].1,
        GrantRequest {
            client_key: "id-key".into(),
            agent_id: None,
            acting_for: None,
            items: ItemSelector::Ids(vec!["gh".into(), "gh2".into()]),
            targets: vec![Target::Origin(GH_LOGIN.into())],
            modes: vec![Mode::Fill],
            run_id: None,
            reason: None,
        }
    );
}

#[test]
fn agent_mode_is_read_as_the_string_true_or_a_json_true() {
    let (extension, mut connection) = agent_connection(GrantAnswer::Active("gh".into()));
    let boolean = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "agentMode": true }),
    );
    assert_eq!(boolean["grantId"], GRANT_ID);
    assert_eq!(boolean["count"], 1);

    let off = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "agentMode": "false" }),
    );
    assert_eq!(off["count"], 2);
    assert!(off.get("grantId").is_none());
    assert_eq!(connection.host().grant_requests.borrow().len(), 1);
}

#[test]
fn a_denied_grant_refuses_the_fill_and_the_refusal_carries_nothing() {
    let (extension, mut connection) = agent_connection(GrantAnswer::Denied);
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "agentMode": "true" }),
    );
    assert_eq!(error_code(&response), Code::GrantDenied as u8);
    assert_eq!(response["error"], "Grant denied");
    assert_eq!(sorted_keys(&response), ["action", "error", "errorCode"]);
    let wire = serde_json::to_string(&response).unwrap();
    for secret in [
        "hunter2", "s3cret", "123456", "octocat", "entries", "password",
    ] {
        assert!(!wire.contains(secret), "{secret} in {wire}");
    }
    assert_eq!(connection.host().grant_requests.borrow().len(), 1);
}

#[test]
fn a_pending_grant_says_so_and_when_to_ask_again() {
    let (extension, mut connection) = agent_connection(GrantAnswer::Pending(Some(1_500)));
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "agentMode": "true" }),
    );
    assert_eq!(error_code(&response), Code::GrantPending as u8);
    assert_eq!(response["retryAfterMs"], 1_500);
    assert_eq!(
        sorted_keys(&response),
        ["action", "error", "errorCode", "retryAfterMs"]
    );

    // No time given: no field, and the refusal is the one it always was.
    let (extension, mut connection) = agent_connection(GrantAnswer::Pending(None));
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "agentMode": "true" }),
    );
    assert_eq!(error_code(&response), Code::GrantPending as u8);
    assert_eq!(sorted_keys(&response), ["action", "error", "errorCode"]);
}

#[test]
fn with_agents_off_an_agent_mode_fill_is_refused_and_a_plain_one_is_not() {
    let (extension, mut connection) = agent_connection(GrantAnswer::Disabled);
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "agentMode": "true" }),
    );
    assert_eq!(error_code(&response), Code::AgentsDisabled as u8);
    assert_eq!(
        response["error"],
        "Agents are turned off in Rowel's settings"
    );
    let plain = extension.send(&mut connection, "get-logins", json!({ "url": GH_LOGIN }));
    assert_eq!(plain["count"], 2);
}

// Before the grant comes the page's logins: a site with none is the refusal
// it always was, and nobody is asked for nothing.
#[test]
fn in_agent_mode_a_site_without_logins_asks_nobody() {
    let (extension, mut connection) = agent_connection(GrantAnswer::Active("gh".into()));
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://example.com", "agentMode": "true" }),
    );
    assert_eq!(error_code(&response), Code::NoLoginsFound as u8);
    assert!(connection.host().grant_requests.borrow().is_empty());
}

#[test]
fn a_grant_the_extension_holds_serves_only_what_it_covers() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock {
        logins: two_logins(),
        ..Mock::unlocked()
            .known("Chrome", "id-key")
            .granting("g1", "gh", Mode::Fill)
    });
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "id-key" }),
    );

    let covered = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "grantId": "g1" }),
    );
    assert_eq!(covered["count"], 1);
    assert_eq!(covered["entries"][0]["uuid"], "gh");
    assert!(covered.get("grantId").is_none());

    let unknown = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "grantId": "g9" }),
    );
    assert_eq!(error_code(&unknown), Code::GrantDenied as u8);
    assert!(
        connection.host().grant_requests.borrow().is_empty(),
        "a held grant asks nobody"
    );

    // Both fields: the grant in hand wins, and nobody is asked again.
    let both = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "grantId": "g1", "agentMode": "true" }),
    );
    assert_eq!(both["count"], 1);
    assert!(connection.host().grant_requests.borrow().is_empty());
}

// In agent mode the TOTP step never falls back to the ungranted path: a
// request saying `agentMode` without a grant is refused, one with a grant
// that covers the login is served, and the two legacy shapes are untouched.
#[test]
fn get_totp_in_agent_mode_is_refused_without_a_grant_and_served_under_one() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock::unlocked().known("Chrome", "id-key").granting(
        "g1",
        "gh",
        Mode::Fill,
    ));
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "id-key" }),
    );

    let ungranted = extension.send(
        &mut connection,
        "get-totp",
        json!({ "uuid": "gh", "agentMode": "true" }),
    );
    assert_eq!(error_code(&ungranted), Code::GrantDenied as u8);
    assert!(ungranted.get("totp").is_none());

    let covered = extension.send(
        &mut connection,
        "get-totp",
        json!({ "uuid": "gh", "agentMode": "true", "grantId": "g1" }),
    );
    assert_eq!(covered["totp"], "654321");

    let uncovered = extension.send(
        &mut connection,
        "get-totp",
        json!({ "uuid": "gh", "agentMode": true, "grantId": "g9" }),
    );
    assert_eq!(error_code(&uncovered), Code::GrantDenied as u8);

    // `agentMode` off, or absent: today's behaviour, grant or no grant.
    let off = extension.send(
        &mut connection,
        "get-totp",
        json!({ "uuid": "gh", "agentMode": "false" }),
    );
    assert_eq!(off["totp"], "654321");
    let legacy = extension.send(&mut connection, "get-totp", json!({ "uuid": "gh" }));
    assert_eq!(legacy["totp"], "654321");
    let held = extension.send(
        &mut connection,
        "get-totp",
        json!({ "uuid": "gh", "grantId": "g1" }),
    );
    assert_eq!(held["totp"], "654321");
}

// --- an extension the user marked as driven by an agent -------------------------

/// Associated as an extension the user marked as driven by an agent, with
/// two logins for GitHub and the host scripted to `answer`.
fn driven_connection(answer: GrantAnswer) -> (Extension, Connection<Mock>) {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock {
        logins: two_logins(),
        grant_answer: answer,
        ..Mock::unlocked().known_driven("Comet", "id-key")
    });
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Comet", "key": "id-key" }),
    );
    (extension, connection)
}

// The mark is the host's: a marked extension's fill is a grant whether it
// says `agentMode`, says it is off, or says nothing at all.
#[test]
fn a_marked_extension_s_fill_is_a_grant_whatever_it_says() {
    for message in [
        json!({ "url": GH_LOGIN }),
        json!({ "url": GH_LOGIN, "agentMode": "false" }),
        json!({ "url": GH_LOGIN, "agentMode": "true" }),
    ] {
        let (extension, mut connection) = driven_connection(GrantAnswer::Active("gh2".into()));
        let response = extension.send(&mut connection, "get-logins", message.clone());
        assert_eq!(response["count"], 1, "{message}");
        assert_eq!(response["entries"][0]["uuid"], "gh2", "{message}");
        assert_eq!(response["grantId"], GRANT_ID, "{message}");
        assert_eq!(connection.host().grant_requests.borrow().len(), 1);
    }

    let (extension, mut connection) = driven_connection(GrantAnswer::Disabled);
    let response = extension.send(&mut connection, "get-logins", json!({ "url": GH_LOGIN }));
    assert_eq!(error_code(&response), Code::AgentsDisabled as u8);
    assert!(response.get("entries").is_none());
}

// A marked extension's TOTP step needs a grant it holds or was served under,
// named or not: none before its fill was granted, the code after, and never
// by claiming a grant it was not served under.
#[test]
fn a_marked_extension_s_totp_step_is_served_only_under_its_own_grant() {
    let (extension, mut connection) = driven_connection(GrantAnswer::Active("gh".into()));
    let before = extension.send(&mut connection, "get-totp", json!({ "uuid": "gh" }));
    assert_eq!(error_code(&before), Code::GrantDenied as u8);
    assert!(before.get("totp").is_none());

    extension.send(&mut connection, "get-logins", json!({ "url": GH_LOGIN }));
    let after = extension.send(&mut connection, "get-totp", json!({ "uuid": "gh" }));
    assert_eq!(after["totp"], "654321");

    // An agent's grant nobody has been served under yet is not this
    // extension's to claim without naming it.
    let mut extension = Extension::new();
    let mut connection = Connection::new(
        Mock::unlocked()
            .known_driven("Comet", "id-key")
            .granting("g1", "gh", Mode::Fill),
    );
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Comet", "key": "id-key" }),
    );
    let unclaimed = extension.send(&mut connection, "get-totp", json!({ "uuid": "gh" }));
    assert_eq!(error_code(&unclaimed), Code::GrantDenied as u8);
    assert_eq!(connection.host().grants.borrow()["g1"].served_to, None);
}

// The mark changes nothing for an extension that does not carry it, and
// an old record without it reads as unmarked and is written back as it was.
#[test]
fn an_unmarked_extension_fills_as_it_always_did() {
    let (extension, mut connection) = agent_connection(GrantAnswer::Disabled);
    let logins = extension.send(&mut connection, "get-logins", json!({ "url": GH_LOGIN }));
    assert_eq!(logins["count"], 2);
    let code = extension.send(&mut connection, "get-totp", json!({ "uuid": "gh" }));
    assert_eq!(code["totp"], "654321");
    assert!(connection.host().grant_requests.borrow().is_empty());

    let old: Client = serde_json::from_str(r#"{"name":"Chrome","key":"k"}"#).unwrap();
    assert!(!old.driven_by_agent);
    assert!(!serde_json::to_string(&old)
        .unwrap()
        .contains("drivenByAgent"));
    let marked = Client {
        driven_by_agent: true,
        ..old
    };
    assert_eq!(
        serde_json::to_value(&marked).unwrap()["drivenByAgent"],
        true
    );
}

#[test]
fn get_totp_under_a_grant_needs_the_grant_to_cover_the_login_for_a_fill() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(
        Mock::unlocked()
            .known("Chrome", "id-key")
            .granting("g1", "gh", Mode::Fill)
            .granting("g2", "gh", Mode::Passkey),
    );
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "id-key" }),
    );

    let covered = extension.send(
        &mut connection,
        "get-totp",
        json!({ "uuid": "gh", "grantId": "g1" }),
    );
    assert_eq!(covered["totp"], "654321");
    let unknown = extension.send(
        &mut connection,
        "get-totp",
        json!({ "uuid": "gh", "grantId": "g9" }),
    );
    assert_eq!(error_code(&unknown), Code::GrantDenied as u8);
    let other_mode = extension.send(
        &mut connection,
        "get-totp",
        json!({ "uuid": "gh", "grantId": "g2" }),
    );
    assert_eq!(error_code(&other_mode), Code::GrantDenied as u8);
    let plain = extension.send(&mut connection, "get-totp", json!({ "uuid": "gh" }));
    assert_eq!(plain["totp"], "654321");
}

#[test]
fn report_exposure_reaches_the_host() {
    let (extension, mut connection) = associated();
    let response = extension.send(
        &mut connection,
        "report-exposure",
        json!({ "grantId": "g1" }),
    );
    assert_eq!(response["success"], "true");
    assert_eq!(
        sorted_keys(&response),
        ["action", "nonce", "success", "version"]
    );
    assert_eq!(connection.host().exposures.borrow().as_slice(), ["g1"]);

    let blank = extension.send(&mut connection, "report-exposure", json!({}));
    assert_eq!(error_code(&blank), Code::NoValidUuidProvided as u8);
    assert_eq!(connection.host().exposures.borrow().len(), 1);
}

#[test]
fn the_grant_paths_are_refused_before_association() {
    let (extension, mut connection) = ready();
    let agent = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "agentMode": "true" }),
    );
    assert_eq!(error_code(&agent), Code::AssociationFailed as u8);
    let held = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "grantId": "g1" }),
    );
    assert_eq!(error_code(&held), Code::AssociationFailed as u8);
    let totp = extension.send(
        &mut connection,
        "get-totp",
        json!({ "uuid": "gh", "grantId": "g1" }),
    );
    assert_eq!(error_code(&totp), Code::AssociationFailed as u8);
    let exposure = extension.send(
        &mut connection,
        "report-exposure",
        json!({ "grantId": "g1" }),
    );
    assert_eq!(error_code(&exposure), Code::AssociationFailed as u8);

    let host = connection.host();
    assert!(host.grant_requests.borrow().is_empty());
    assert!(host.exposures.borrow().is_empty());
}

// --- the vocabularies: what each kind may say ---------------------------------------

const AGENT_KEY: &str = "agent-key";

/// Associated as the agent "Claude" from claude-code, with two logins for
/// GitHub and the host scripted to `answer`.
fn agent_associated(answer: GrantAnswer) -> (Extension, Connection<Mock>) {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock {
        logins: two_logins(),
        grant_answer: answer,
        ..Mock::unlocked()
            .known("Chrome", "id-key")
            .known_agent("Claude", AGENT_KEY, "claude-code")
    });
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Claude", "key": AGENT_KEY }),
    );
    (extension, connection)
}

/// The simplest well-formed `request-grant`: one login, at GitHub, to fill.
fn grant_request() -> Value {
    json!({
        "runId": "run-1",
        "reason": "deploy the docs site",
        "items": { "ids": ["gh2"] },
        "targets": [{ "origin": "https://github.com" }],
        "modes": ["fill"],
    })
}

/// Every action whose reply carries a secret, with a request the host would
/// otherwise answer.
fn secret_carrying_actions() -> Vec<(&'static str, Value)> {
    vec![
        ("get-logins", json!({ "url": GH_LOGIN })),
        ("get-totp", json!({ "uuid": "gh" })),
        (
            "set-login",
            json!({ "url": GH_LOGIN, "login": "a", "password": "b" }),
        ),
        (
            "passkeys-get",
            json!({ "publicKey": {}, "origin": GH_LOGIN }),
        ),
        (
            "passkeys-register",
            json!({ "publicKey": {}, "origin": GH_LOGIN }),
        ),
        ("get-signals", json!({})),
        ("report-exposure", json!({ "grantId": "g1" })),
        (
            "resolve",
            json!({ "grantId": "g1", "ref": gh_ref("password"), "context": http_context("GET") }),
        ),
    ]
}

fn agent_actions() -> Vec<(&'static str, Value)> {
    vec![
        ("request-grant", grant_request()),
        (
            "wait-grant",
            json!({ "grantId": GRANT_ID, "timeoutMs": 10 }),
        ),
        ("grant-status", json!({ "grantId": GRANT_ID })),
        ("end-grant", json!({ "grantId": GRANT_ID })),
        ("list-visible", json!({})),
    ]
}

// An agent never receives a value, by construction: every action that
// carries one is refused in the clear before the host is asked anything,
// and the refusal says which door it was.
#[test]
fn an_agent_is_refused_every_action_that_carries_a_secret() {
    let (extension, mut connection) = agent_associated(GrantAnswer::Active("gh2".into()));
    refused_at_the_door(&extension, &mut connection, secret_carrying_actions());

    // What everyone may say, an agent may too.
    let hash = extension.send(&mut connection, "get-databasehash", json!({}));
    assert_eq!(hash["hash"], "abc123");
    let password = extension.send(&mut connection, "generate-password", json!({}));
    assert_eq!(password["password"], "generated");
    let locked = extension.send(&mut connection, "lock-database", json!({}));
    assert_eq!(locked["success"], "true");
    assert_eq!(connection.host().locks.get(), 1);
}

/// Every one of `actions` is refused as the wrong kind, in the clear, before
/// the host is asked anything, with no value and no field a value would
/// travel in anywhere in the text.
fn refused_at_the_door(
    extension: &Extension,
    connection: &mut Connection<Mock>,
    actions: Vec<(&'static str, Value)>,
) {
    for (action, message) in actions {
        let response = extension.send(connection, action, message);
        assert_eq!(
            error_code(&response),
            Code::NotPermittedForKind as u8,
            "{action}"
        );
        assert_eq!(
            sorted_keys(&response),
            ["action", "error", "errorCode"],
            "{action}"
        );
        // The key set above is the field check; what is left is that no
        // value leaked into the text.
        let wire = serde_json::to_string(&response).unwrap();
        for secret in [
            "hunter2",
            "s3cret",
            "123456",
            "654321",
            "octocat",
            "sk_test_mock",
            "value",
        ] {
            assert!(
                !wire.contains(secret),
                "{secret} in {action}'s refusal: {wire}"
            );
        }
    }
    let host = connection.host();
    assert!(host.grant_requests.borrow().is_empty());
    assert!(host.saves.borrow().is_empty());
    assert!(host.passkey_requests.borrow().is_empty());
    assert!(host.exposures.borrow().is_empty());
    assert!(host.grants.borrow().values().all(|grant| grant.uses == 0));
}

// A sidecar is an extension no more than an agent is: the fill's secrets
// and the extension's signals are refused to it the same way.
#[test]
fn a_sidecar_is_refused_every_action_of_the_extensions() {
    let (extension, mut connection) = sidecar_associated(
        Mock {
            logins: two_logins(),
            ..Mock::unlocked().granting("g1", "gh", Mode::Fill)
        },
        GrantAnswer::Active("gh2".into()),
    );
    let extensions_own: Vec<(&'static str, Value)> = secret_carrying_actions()
        .into_iter()
        .filter(|(action, _)| *action != "resolve")
        .collect();
    assert_eq!(extensions_own.len(), secret_carrying_actions().len() - 1);
    refused_at_the_door(&extension, &mut connection, extensions_own);
}

#[test]
fn an_extension_is_refused_every_agent_action() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock {
        logins: two_logins(),
        grant_answer: GrantAnswer::Active("gh2".into()),
        ..Mock::unlocked().known("Chrome", "id-key")
    });
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "id-key" }),
    );
    for (action, message) in agent_actions() {
        let response = extension.send(&mut connection, action, message);
        assert_eq!(
            error_code(&response),
            Code::NotPermittedForKind as u8,
            "{action}"
        );
        assert_eq!(
            sorted_keys(&response),
            ["action", "error", "errorCode"],
            "{action}"
        );
    }
    let host = connection.host();
    assert!(host.grant_requests.borrow().is_empty());
    assert!(host.ended.borrow().is_empty());
}

// A stranger hears the refusal it always heard: the kind is checked second,
// so an unassociated connection learns nothing about which actions exist for
// whom.
#[test]
fn the_agent_actions_are_refused_before_association() {
    let (extension, mut connection) = ready();
    for (action, message) in agent_actions() {
        let response = extension.send(&mut connection, action, message);
        assert_eq!(
            error_code(&response),
            Code::AssociationFailed as u8,
            "{action}"
        );
    }
    let signals = extension.send(&mut connection, "get-signals", json!({}));
    assert_eq!(error_code(&signals), Code::AssociationFailed as u8);
}

// --- what an agent gets: grants by name, never a value ----------------------------

#[test]
fn request_grant_answers_active_with_the_admitted_items_and_nothing_else() {
    let (extension, mut connection) = agent_associated(GrantAnswer::Active("gh2".into()));
    let response = extension.send(&mut connection, "request-grant", grant_request());
    assert_eq!(response["grantId"], GRANT_ID);
    assert_eq!(response["state"], "active");
    assert_eq!(response["expiresAtMs"], GRANT_EXPIRES_AT_MS);
    assert_eq!(response["uses"], 0);
    assert_eq!(
        response["items"],
        json!([{ "id": "gh2", "title": "GitHub (work)", "username": "octocorp" }])
    );
    assert_eq!(
        sorted_keys(&response),
        [
            "action",
            "expiresAtMs",
            "grantId",
            "items",
            "nonce",
            "retryAfterMs",
            "state",
            "success",
            "uses",
            "version"
        ]
    );
    assert_eq!(
        sorted_keys(&response["items"][0]),
        ["id", "title", "username"]
    );
    let wire = serde_json::to_string(&response).unwrap();
    for secret in ["hunter2", "s3cret", "123456", "password", "totp"] {
        assert!(!wire.contains(secret), "{secret} in {wire}");
    }

    // What the host was asked: by the connection's client, never by a key
    // in the message, with the agent's own words along.
    let asked = connection.host().grant_requests.borrow();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].0, AGENT_KEY);
    assert_eq!(
        asked[0].1,
        GrantRequest {
            client_key: AGENT_KEY.into(),
            agent_id: None,
            acting_for: None,
            items: ItemSelector::Ids(vec!["gh2".into()]),
            targets: vec![Target::Origin("https://github.com".into())],
            modes: vec![Mode::Fill],
            run_id: Some("run-1".into()),
            reason: Some("deploy the docs site".into()),
        }
    );
}

// A denial and a pending answer are sealed replies with a state, not clear
// refusals: the agent asked a question and gets its answer.
#[test]
fn request_grant_answers_denied_and_pending_with_when_to_ask_again() {
    let (extension, mut connection) = agent_associated(GrantAnswer::Denied);
    let denied = extension.send(&mut connection, "request-grant", grant_request());
    assert_eq!(denied["state"], "denied");
    assert_eq!(denied["grantId"], GRANT_ID);
    assert_eq!(denied["items"], json!([]));
    assert!(denied["expiresAtMs"].is_null());

    let (extension, mut connection) = agent_associated(GrantAnswer::Pending(Some(1_500)));
    let pending = extension.send(&mut connection, "request-grant", grant_request());
    assert_eq!(pending["state"], "pending");
    assert_eq!(pending["retryAfterMs"], 1_500);
    assert_eq!(pending["items"], json!([]));

    let (extension, mut connection) = agent_associated(GrantAnswer::Disabled);
    let off = extension.send(&mut connection, "request-grant", grant_request());
    assert_eq!(error_code(&off), Code::AgentsDisabled as u8);
}

#[test]
fn request_grant_bounds_the_run_id_and_the_reason() {
    let (extension, mut connection) = agent_associated(GrantAnswer::Active("gh2".into()));
    let mut long_run = grant_request();
    long_run["runId"] = "r".repeat(65).into();
    let refused = extension.send(&mut connection, "request-grant", long_run);
    assert_eq!(error_code(&refused), Code::GrantDenied as u8);
    assert!(connection.host().grant_requests.borrow().is_empty());

    let mut at_the_edge = grant_request();
    at_the_edge["runId"] = "r".repeat(64).into();
    at_the_edge["reason"] = "x".repeat(400).into();
    let served = extension.send(&mut connection, "request-grant", at_the_edge);
    assert_eq!(served["state"], "active");
    let asked = connection.host().grant_requests.borrow();
    assert_eq!(asked[0].1.run_id.as_deref(), Some("r".repeat(64).as_str()));
    assert_eq!(
        asked[0].1.reason.as_deref(),
        Some("x".repeat(300).as_str()),
        "cut, not refused"
    );
    drop(asked);

    // Neither is required: a request without them is one with none.
    let mut bare = grant_request();
    bare.as_object_mut().unwrap().remove("runId");
    bare.as_object_mut().unwrap().remove("reason");
    extension.send(&mut connection, "request-grant", bare);
    let asked = connection.host().grant_requests.borrow();
    assert_eq!((&asked[1].1.run_id, &asked[1].1.reason), (&None, &None));
}

// A grant is never wider than its request, so a request the host would have
// to guess at is refused rather than read generously.
#[test]
fn request_grant_refuses_a_request_it_cannot_read() {
    let (extension, mut connection) = agent_associated(GrantAnswer::Active("gh2".into()));
    let without = |field: &str| {
        let mut request = grant_request();
        request.as_object_mut().unwrap().remove(field);
        request
    };
    let with = |field: &str, value: Value| {
        let mut request = grant_request();
        request[field] = value;
        request
    };
    for (what, request) in [
        ("no items", without("items")),
        ("no targets", without("targets")),
        ("no modes", without("modes")),
        ("an unknown selector", with("items", json!("everything"))),
        (
            "an unknown target kind",
            with("targets", json!([{ "planet": "mars" }])),
        ),
        ("an unknown mode", with("modes", json!(["teleport"]))),
        ("no target at all", with("targets", json!([]))),
        ("no mode at all", with("modes", json!([]))),
    ] {
        let response = extension.send(&mut connection, "request-grant", request);
        assert_eq!(error_code(&response), Code::GrantDenied as u8, "{what}");
    }
    assert!(connection.host().grant_requests.borrow().is_empty());
}

// A request for a header or an env grant names a request pattern or a
// command line, and the host is handed exactly that, typed.
#[test]
fn request_grant_carries_http_and_command_targets_to_the_host_as_written() {
    let (extension, mut connection) = agent_associated(GrantAnswer::Active("gh2".into()));
    let header = extension.send(
        &mut connection,
        "request-grant",
        json!({
            "runId": "run-1",
            "items": { "ids": ["stripe"] },
            "targets": [{ "http": { "host": "api.stripe.com", "path": "/v1/*", "methods": ["GET"] } }],
            "modes": ["header"],
        }),
    );
    assert_eq!(header["state"], "active");
    let env = extension.send(
        &mut connection,
        "request-grant",
        json!({
            "runId": "run-1",
            "items": { "ids": ["stripe"] },
            "targets": [{ "command": "stripe * --live" }],
            "modes": ["env"],
        }),
    );
    assert_eq!(env["state"], "active");

    let asked = connection.host().grant_requests.borrow();
    assert_eq!(asked.len(), 2);
    assert_eq!(
        (&asked[0].1.targets, &asked[0].1.modes),
        (
            &vec![Target::Http {
                host: "api.stripe.com".into(),
                path: "/v1/*".into(),
                methods: vec!["GET".into()],
            }],
            &vec![Mode::Header]
        )
    );
    assert_eq!(
        (&asked[1].1.targets, &asked[1].1.modes),
        (
            &vec![Target::Command("stripe * --live".into())],
            &vec![Mode::Env]
        )
    );
}

// The grant's state is the agent's to read, uses included — and a use made by
// the extension the grant was served to is one the agent sees.
#[test]
fn wait_grant_and_grant_status_answer_the_grants_state_with_its_uses() {
    let (extension, mut connection) = agent_associated(GrantAnswer::Active("gh2".into()));
    extension.send(&mut connection, "request-grant", grant_request());
    let waited = extension.send(
        &mut connection,
        "wait-grant",
        json!({ "grantId": GRANT_ID, "timeoutMs": 5_000 }),
    );
    assert_eq!(waited["state"], "active");
    assert_eq!(waited["uses"], 0);
    assert_eq!(
        waited["items"],
        json!([{ "id": "gh2", "title": "GitHub (work)", "username": "octocorp" }])
    );

    // The extension fills under the grant, which is a use.
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "id-key" }),
    );
    let filled = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "grantId": GRANT_ID }),
    );
    assert_eq!(filled["count"], 1);
    assert_eq!(filled["entries"][0]["uuid"], "gh2");

    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Claude", "key": AGENT_KEY }),
    );
    let status = extension.send(
        &mut connection,
        "grant-status",
        json!({ "grantId": GRANT_ID }),
    );
    assert_eq!(status["state"], "active");
    assert_eq!(status["uses"], 1);
    assert_eq!(status["grantId"], GRANT_ID);

    let unknown = extension.send(&mut connection, "grant-status", json!({ "grantId": "g9" }));
    assert_eq!(error_code(&unknown), Code::GrantDenied as u8);
    let blank = extension.send(&mut connection, "wait-grant", json!({}));
    assert_eq!(error_code(&blank), Code::NoValidUuidProvided as u8);
}

#[test]
fn end_grant_answers_ok_and_the_host_records_it() {
    let (extension, mut connection) = agent_associated(GrantAnswer::Active("gh2".into()));
    extension.send(&mut connection, "request-grant", grant_request());
    let response = extension.send(&mut connection, "end-grant", json!({ "grantId": GRANT_ID }));
    assert_eq!(response["ok"], true);
    assert_eq!(
        sorted_keys(&response),
        ["action", "nonce", "ok", "success", "version"]
    );
    assert_eq!(connection.host().ended.borrow().as_slice(), [GRANT_ID]);
    let status = extension.send(
        &mut connection,
        "grant-status",
        json!({ "grantId": GRANT_ID }),
    );
    assert_eq!(status["state"], "revoked");

    let unknown = extension.send(&mut connection, "end-grant", json!({ "grantId": "g9" }));
    assert_eq!(error_code(&unknown), Code::GrantDenied as u8);
    let blank = extension.send(&mut connection, "end-grant", json!({}));
    assert_eq!(error_code(&blank), Code::NoValidUuidProvided as u8);
    assert_eq!(connection.host().ended.borrow().len(), 1);
}

#[test]
fn list_visible_answers_names_and_fields_never_values() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock {
        visible: vec![VisibleRef {
            reference: "rowel://lb/gh".into(),
            title: "GitHub".into(),
            kind: "login".into(),
            fields: vec!["password".into(), "otp".into()],
        }],
        ..Mock::unlocked().known_agent("Claude", AGENT_KEY, "claude-code")
    });
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Claude", "key": AGENT_KEY }),
    );
    let response = extension.send(&mut connection, "list-visible", json!({ "lockbox": "lb" }));
    assert_eq!(
        response["items"],
        json!([{ "ref": "rowel://lb/gh", "title": "GitHub", "kind": "login", "fields": ["password", "otp"] }])
    );
    assert_eq!(
        sorted_keys(&response),
        ["action", "items", "nonce", "success", "version"]
    );
    assert_eq!(
        sorted_keys(&response["items"][0]),
        ["fields", "kind", "ref", "title"]
    );
    let wire = serde_json::to_string(&response).unwrap();
    for secret in ["hunter2", "123456", "octocat"] {
        assert!(!wire.contains(secret), "{secret} in {wire}");
    }

    connection.host().agents_enabled.set(false);
    let off = extension.send(&mut connection, "list-visible", json!({ "lockbox": "lb" }));
    assert_eq!(error_code(&off), Code::AgentsDisabled as u8);
}

// --- resolve: the one value that leaves for something not an extension -----------

const SIDECAR_KEY: &str = "sidecar-key";

/// Associated as the sidecar "rowel", over `mock` with the host scripted to
/// `answer`.
fn sidecar_associated(mock: Mock, answer: GrantAnswer) -> (Extension, Connection<Mock>) {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock {
        grant_answer: answer,
        ..mock.known_sidecar("rowel", SIDECAR_KEY)
    });
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "rowel", "key": SIDECAR_KEY }),
    );
    (extension, connection)
}

/// The placeholder for `field` of the GitHub login, in the open lockbox.
fn gh_ref(field: &str) -> String {
    format!("rowel://{LOCKBOX}/gh/{field}")
}

/// The placeholder for the Stripe key, in the open lockbox.
fn stripe_ref() -> String {
    format!("rowel://{LOCKBOX}/stripe/apiKey")
}

/// A `resolve` context for `method` of Stripe's balance endpoint.
fn http_context(method: &str) -> Value {
    json!({ "target": { "http": { "host": "api.stripe.com", "path": "/v1/balance", "method": method } } })
}

/// The Stripe target a header grant in these tests names.
fn stripe_target() -> Target {
    Target::Http {
        host: "api.stripe.com".into(),
        path: "/v1/*".into(),
        methods: vec!["GET".into()],
    }
}

fn uses_of(connection: &Connection<Mock>, grant_id: &str) -> u32 {
    connection.host().grants.borrow()[grant_id].uses
}

#[test]
fn a_sidecar_resolves_under_an_active_header_grant_and_the_use_is_counted() {
    let (extension, mut connection) = sidecar_associated(
        Mock::unlocked().granting_at("g1", "stripe", Mode::Header, stripe_target()),
        GrantAnswer::Denied,
    );
    let response = extension.send(
        &mut connection,
        "resolve",
        json!({ "grantId": "g1", "ref": stripe_ref(), "context": http_context("GET") }),
    );
    assert_eq!(response["value"], "sk_test_mock");
    assert_eq!(
        sorted_keys(&response),
        ["action", "nonce", "success", "value", "version"]
    );
    assert_eq!(uses_of(&connection, "g1"), 1);

    extension.send(
        &mut connection,
        "resolve",
        json!({ "grantId": "g1", "ref": stripe_ref(), "context": http_context("get") }),
    );
    assert_eq!(
        uses_of(&connection, "g1"),
        2,
        "methods compare like HTTP does"
    );
}

#[test]
fn a_sidecar_resolves_an_env_grant_at_the_command_line_it_names() {
    let (extension, mut connection) = sidecar_associated(
        Mock::unlocked().granting_at(
            "g1",
            "stripe",
            Mode::Env,
            Target::Command("stripe * --live".into()),
        ),
        GrantAnswer::Denied,
    );
    let response = extension.send(
        &mut connection,
        "resolve",
        json!({
            "grantId": "g1",
            "ref": stripe_ref(),
            "context": { "target": { "command": "stripe balance retrieve --live" } },
        }),
    );
    assert_eq!(response["value"], "sk_test_mock");
    assert_eq!(uses_of(&connection, "g1"), 1);

    // A grant anywhere serves any line of its mode, and a login's secrets
    // resolve by their own names.
    let (extension, mut connection) = sidecar_associated(
        Mock::unlocked().granting("g2", "gh", Mode::Env),
        GrantAnswer::Denied,
    );
    for (field, value) in [("password", "hunter2"), ("otp", "123456")] {
        let response = extension.send(
            &mut connection,
            "resolve",
            json!({
                "grantId": "g2",
                "ref": gh_ref(field),
                "context": { "target": { "command": "gh auth login" } },
            }),
        );
        assert_eq!(response["value"], value, "{field}");
    }
    assert_eq!(uses_of(&connection, "g2"), 2);
}

// The verb is the sidecar's alone: an agent — the model-facing surface — and
// an extension hear which door it was, and no value is in the text.
#[test]
fn an_agent_and_an_extension_are_refused_resolve_at_the_door() {
    let mock = || Mock::unlocked().granting_at("g1", "stripe", Mode::Header, stripe_target());
    let resolve = json!({ "grantId": "g1", "ref": stripe_ref(), "context": http_context("GET") });

    let (extension, mut connection) = sidecar_associated(mock(), GrantAnswer::Denied);
    let served = extension.send(&mut connection, "resolve", resolve.clone());
    assert_eq!(served["value"], "sk_test_mock", "the shape a sidecar gets");

    let mut extension = Extension::new();
    let mut connection = Connection::new(mock().known_agent("Claude", AGENT_KEY, "claude-code"));
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Claude", "key": AGENT_KEY }),
    );
    refused_at_the_door(
        &extension,
        &mut connection,
        vec![("resolve", resolve.clone())],
    );

    let mut extension = Extension::new();
    let mut connection = Connection::new(mock().known("Chrome", "id-key"));
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "id-key" }),
    );
    refused_at_the_door(&extension, &mut connection, vec![("resolve", resolve)]);
}

// Every no is the same no, and none is a use: a grant id is a capability,
// and a request under one is not told which part of it was wrong.
#[test]
fn resolve_is_denied_in_one_word_for_whatever_is_wrong_and_counts_no_use() {
    let (extension, mut connection) = sidecar_associated(
        Mock::unlocked()
            .granting_at("g1", "stripe", Mode::Header, stripe_target())
            .granting("g2", "gh", Mode::Fill),
        GrantAnswer::Denied,
    );
    let request = |grant_id: &str, reference: &str, context: Value| json!({ "grantId": grant_id, "ref": reference, "context": context });
    for (what, message) in [
        (
            "an unknown grant",
            request("g9", &stripe_ref(), http_context("GET")),
        ),
        (
            "a grant in the wrong mode",
            request("g2", &gh_ref("password"), http_context("GET")),
        ),
        (
            "a target the grant does not cover",
            request("g1", &stripe_ref(), http_context("POST")),
        ),
        (
            "a host the grant does not cover",
            request(
                "g1",
                &stripe_ref(),
                json!({ "target": { "http": { "host": "api.evil.io", "path": "/v1/balance", "method": "GET" } } }),
            ),
        ),
        (
            "a command line under a header grant",
            request(
                "g1",
                &stripe_ref(),
                json!({ "target": { "command": "stripe balance retrieve" } }),
            ),
        ),
        (
            "an item the grant does not admit",
            request("g1", &gh_ref("password"), http_context("GET")),
        ),
        (
            "a field the item lacks",
            request(
                "g1",
                &format!("rowel://{LOCKBOX}/stripe/otp"),
                http_context("GET"),
            ),
        ),
        (
            "the wrong lockbox",
            request("g1", "rowel://other/stripe/apiKey", http_context("GET")),
        ),
        (
            "a malformed ref",
            request("g1", "stripe/apiKey", http_context("GET")),
        ),
        (
            "no ref",
            json!({ "grantId": "g1", "context": http_context("GET") }),
        ),
        (
            "a malformed context",
            request(
                "g1",
                &stripe_ref(),
                json!({ "target": { "planet": "mars" } }),
            ),
        ),
        (
            "no context",
            json!({ "grantId": "g1", "ref": stripe_ref() }),
        ),
    ] {
        let response = extension.send(&mut connection, "resolve", message);
        assert_eq!(error_code(&response), Code::GrantDenied as u8, "{what}");
        assert_eq!(
            sorted_keys(&response),
            ["action", "error", "errorCode"],
            "{what}"
        );
        let wire = serde_json::to_string(&response).unwrap();
        for secret in ["sk_test_mock", "hunter2", "value"] {
            assert!(!wire.contains(secret), "a value in {what}'s refusal");
        }
    }
    assert_eq!(uses_of(&connection, "g1"), 0);
    assert_eq!(uses_of(&connection, "g2"), 0);

    // No grant id at all is the protocol's word for a missing identifier,
    // as every grant action answers.
    let blank = extension.send(
        &mut connection,
        "resolve",
        json!({ "ref": stripe_ref(), "context": http_context("GET") }),
    );
    assert_eq!(error_code(&blank), Code::NoValidUuidProvided as u8);
}

#[test]
fn resolve_is_refused_before_association() {
    let (extension, mut connection) = ready();
    let response = extension.send(
        &mut connection,
        "resolve",
        json!({ "grantId": "g1", "ref": stripe_ref(), "context": http_context("GET") }),
    );
    assert_eq!(error_code(&response), Code::AssociationFailed as u8);
}

// --- signals: what the host tells an extension that did not ask ------------------

fn fill_grant_signal() -> Signal {
    Signal::FillGrant {
        grant_id: GRANT_ID.into(),
        origin: "https://github.com".into(),
        item: VisibleItem {
            id: "gh".into(),
            title: "GitHub".into(),
            username: Some("octocat".into()),
        },
        auto_submit: true,
        expires_at_ms: GRANT_EXPIRES_AT_MS,
    }
}

#[test]
fn a_signal_is_tagged_and_camel_cased_on_the_wire() {
    assert_eq!(
        serde_json::from_slice::<Value>(ROWEL_SIGNAL).unwrap(),
        json!({ "action": "rowel-signal" })
    );
    let fill = serde_json::to_value(fill_grant_signal()).unwrap();
    assert_eq!(
        fill,
        json!({
            "signal": "fill-grant", "grantId": GRANT_ID, "origin": "https://github.com",
            "item": { "id": "gh", "title": "GitHub", "username": "octocat" },
            "autoSubmit": true, "expiresAtMs": GRANT_EXPIRES_AT_MS
        })
    );
    assert_eq!(
        serde_json::from_value::<Signal>(fill).unwrap(),
        fill_grant_signal()
    );
    let ended = Signal::GrantEnded {
        grant_id: GRANT_ID.into(),
        origins: vec!["https://github.com".into()],
    };
    assert_eq!(
        serde_json::to_value(&ended).unwrap(),
        json!({ "signal": "grant-ended", "grantId": GRANT_ID, "origins": ["https://github.com"] })
    );
}

#[test]
fn get_signals_drains_the_queue_for_the_client_that_asks() {
    let (extension, mut connection) = associated();
    let ended = Signal::GrantEnded {
        grant_id: GRANT_ID.into(),
        origins: vec!["https://github.com".into()],
    };
    {
        let mut queues = connection.host().signals.borrow_mut();
        queues.insert("id-key".into(), vec![fill_grant_signal(), ended.clone()]);
        queues.insert("other-key".into(), vec![ended.clone()]);
    }
    let first = extension.send(&mut connection, "get-signals", json!({}));
    assert_eq!(
        first["signals"],
        serde_json::to_value([fill_grant_signal(), ended]).unwrap()
    );
    assert_eq!(
        sorted_keys(&first),
        ["action", "nonce", "signals", "success", "version"]
    );
    let second = extension.send(&mut connection, "get-signals", json!({}));
    assert_eq!(second["signals"], json!([]));
    assert_eq!(
        connection.host().signals.borrow().len(),
        1,
        "another client's queue is not this one's to drain"
    );
}

// --- who was served: an agent's fill grant is claimed by one extension -------------

#[test]
fn a_second_extension_asking_under_an_agents_fill_grant_is_refused() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(
        Mock::unlocked()
            .known("Chrome", "key-a")
            .known("Firefox", "key-b")
            .granting("g1", "gh", Mode::Fill),
    );
    extension.exchange(&mut connection);
    let as_client = |extension: &Extension, connection: &mut Connection<Mock>, id, key| {
        extension.send(
            connection,
            "test-associate",
            json!({ "id": id, "key": key }),
        );
    };

    // Chrome is served first and so claims the grant; it may keep using it.
    as_client(&extension, &mut connection, "Chrome", "key-a");
    let first = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "grantId": "g1" }),
    );
    assert_eq!(first["count"], 1);
    let again = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "grantId": "g1" }),
    );
    assert_eq!(again["count"], 1);
    let totp = extension.send(
        &mut connection,
        "get-totp",
        json!({ "uuid": "gh", "grantId": "g1" }),
    );
    assert_eq!(totp["totp"], "654321");

    // Firefox, under the same grant id, is not.
    as_client(&extension, &mut connection, "Firefox", "key-b");
    let other = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "grantId": "g1" }),
    );
    assert_eq!(error_code(&other), Code::GrantDenied as u8);
    let other_totp = extension.send(
        &mut connection,
        "get-totp",
        json!({ "uuid": "gh", "grantId": "g1" }),
    );
    assert_eq!(error_code(&other_totp), Code::GrantDenied as u8);

    // And Chrome still is.
    as_client(&extension, &mut connection, "Chrome", "key-a");
    let still = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "grantId": "g1" }),
    );
    assert_eq!(still["count"], 1);
    let grants = connection.host().grants.borrow();
    assert_eq!(grants["g1"].served_to.as_deref(), Some("key-a"));
    assert_eq!(grants["g1"].uses, 4, "every yes was a use; no refusal was");
}

// Which client a passkey ceremony is for reaches the host, so the dialog can
// say who asks.
#[test]
fn a_passkey_ceremony_tells_the_host_which_client_asks() {
    let (extension, mut connection) = associated();
    let challenge = passkey_types::encoding::base64url(&rand::random::<[u8; 32]>());
    extension.send(
        &mut connection,
        "passkeys-get",
        json!({ "publicKey": { "challenge": challenge, "rpId": "github.com" }, "origin": GH_LOGIN }),
    );
    let asked = connection.host().passkey_requests.borrow();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0], extension_client("Chrome", "id-key"));
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
    *connection.host().clients.borrow_mut() =
        vec![extension_client("Chrome", "a key of the other vault's")];
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com", "keys": [{ "id": "Chrome", "key": "id-key" }] }),
    );
    assert_eq!(error_code(&response), Code::AssociationFailed as u8);

    // Let in there too, and the key it carries opens it again.
    connection
        .host()
        .clients
        .borrow_mut()
        .push(extension_client("Chrome", "id-key"));
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com", "keys": [{ "id": "Chrome", "key": "id-key" }] }),
    );
    assert_eq!(response["count"], 1);
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

// --- the client half: a peer over a connection ----------------------------------

// What the CLI does, in-process: the exchange, a sealed request, and the
// refusals as typed errors — against the same `Connection` the extension
// tests drive, which is its own transport.
#[test]
fn a_peer_exchanges_keys_and_calls_through_the_sealed_channel() {
    let mut peer = Peer::connect(Connection::new(Mock::unlocked()), "cli").unwrap();
    assert_eq!(peer.version(), VERSION);
    let hash = peer.call("get-databasehash", json!({})).unwrap();
    assert_eq!(hash["hash"], "abc123");
    assert_eq!(hash["action"], "get-databasehash");
    assert_eq!(hash["success"], "true");

    // Associate as an agent through it, with the session key it repeats.
    let key = peer.public_key();
    let let_in = peer
        .call(
            "associate",
            json!({ "key": key, "idKey": "agent-key", "hello": { "kind": "agent", "platform": "cli" } }),
        )
        .unwrap();
    assert_eq!(let_in["clientId"], "agent-key");
    let visible = peer.call("list-visible", json!({})).unwrap();
    assert_eq!(visible["items"], json!([]));
}

#[test]
fn a_refusal_reaches_the_peer_with_its_code_and_when_to_ask_again() {
    let connection = Connection::new(Mock {
        logins: two_logins(),
        grant_answer: GrantAnswer::Pending(Some(1_500)),
        ..Mock::unlocked().known("Chrome", "id-key")
    });
    let mut peer = Peer::connect(connection, "cli").unwrap();

    let stranger = peer.call("lock-database", json!({})).unwrap_err();
    let PeerError::Refused(refused) = stranger else {
        panic!("{stranger:?}");
    };
    assert_eq!(refused.code, Some(Code::AssociationFailed));
    assert_eq!(refused.raw, Code::AssociationFailed as u8);
    assert_eq!(refused.message, Code::AssociationFailed.message());
    assert_eq!(refused.retry_after_ms, None);

    peer.call("test-associate", json!({ "id": "Chrome", "key": "id-key" }))
        .unwrap();
    let busy = peer
        .call(
            "get-logins",
            json!({ "url": GH_LOGIN, "agentMode": "true" }),
        )
        .unwrap_err();
    let PeerError::Refused(refused) = busy else {
        panic!("{busy:?}");
    };
    assert_eq!(refused.code, Some(Code::GrantPending));
    assert_eq!(refused.retry_after_ms, Some(1_500));
}

#[test]
fn a_peer_cannot_connect_to_a_host_that_refuses_the_exchange() {
    // A connection whose host refuses the key: `handle` answers the exchange
    // before the host is asked anything, so the one way to see a refusal
    // there is a request the host cannot read — which a `Transport` of our
    // own can make it send.
    struct Garbled;
    impl crate::client::Transport for Garbled {
        fn roundtrip(&mut self, _request: &[u8]) -> std::io::Result<Vec<u8>> {
            Ok(br#"{"action":"change-public-keys","errorCode":"3","error":"Client public key not received"}"#.to_vec())
        }
    }
    let Err(error) = Peer::connect(Garbled, "cli") else {
        panic!("connected through a refusal");
    };
    let PeerError::Refused(refused) = error else {
        panic!("{error:?}");
    };
    assert_eq!(refused.code, Some(Code::ClientPublicKeyNotReceived));

    struct Silent;
    impl crate::client::Transport for Silent {
        fn roundtrip(&mut self, _request: &[u8]) -> std::io::Result<Vec<u8>> {
            Ok(b"not json".to_vec())
        }
    }
    let Err(error) = Peer::connect(Silent, "cli") else {
        panic!("connected through junk");
    };
    assert!(
        matches!(
            error,
            PeerError::Channel(crate::client::ChannelError::BadReply)
        ),
        "{error:?}"
    );
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

#[cfg(not(any(target_os = "android", target_os = "ios")))]
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

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[test]
fn the_socket_name_follows_the_root() {
    let a = socket_name(std::path::Path::new("/tmp/rowel-a")).unwrap();
    let again = socket_name(std::path::Path::new("/tmp/rowel-a")).unwrap();
    let b = socket_name(std::path::Path::new("/tmp/rowel-b")).unwrap();
    assert_eq!(format!("{a:?}"), format!("{again:?}"));
    assert_ne!(format!("{a:?}"), format!("{b:?}"));
}

// --- the server loop and the lock signals --------------------------------------

// Over a TCP pair rather than the local socket: `serve` is a loop over any
// stream, and the registry a list of sinks, so neither needs the socket file
// to be exercised.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod server {
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde_json::{json, Value};

    use crate::frame;
    use crate::server;
    use crate::socket::{socket_name, SOCKET_FILE};
    use crate::testing::{nonce, Extension, Mock};

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
        let reply: Value =
            serde_json::from_slice(&frame::read(&mut stream).unwrap().unwrap()).unwrap();
        assert_eq!(reply["success"], "true");
        assert!(reply["publicKey"].is_string());

        drop(stream);
        served.join().unwrap();
    }

    // The signal registry is one per process, so the tests that register with it
    // run one at a time: with both up at once, one's broadcasts would land on the
    // other's connection, ahead of the frames it is counting.
    static SIGNAL_TESTS: Mutex<()> = Mutex::new(());

    /// A connection's writing end as the socket's accept loop registers it:
    /// every signal framed through the one shared handle.
    fn sink(writer: &Arc<Mutex<TcpStream>>) -> server::Sink {
        let writer = writer.clone();
        Box::new(move |message| frame::write(&mut *writer.lock().unwrap(), message))
    }

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
        let open_id = server::register(sink(&open));
        server::register(sink(&gone));
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

        let stalled_id = server::register(sink(&Arc::new(Mutex::new(stalled))));
        let open_id = server::register(sink(&Arc::new(Mutex::new(open))));
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
            let mode = std::fs::metadata(dir.path().join(SOCKET_FILE))
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
    fn the_lock_makes_one_broker_per_data_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data");
        let first = server::Lock::take(&root).expect("the lock at a fresh root, made for it");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(root.join(server::LOCK_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "owner-only lock file");
        }
        assert!(
            matches!(server::Lock::take(&root), Err(server::LockError::Held)),
            "a second broker on the same directory is refused while the first holds the lock"
        );
        drop(first);
        server::Lock::take(&root).expect("the lock is free once the holder is gone");
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
        assert!(root.join(SOCKET_FILE).exists());
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
}
