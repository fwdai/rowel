//! A vault to talk to, and an extension to talk to it: the scripted [`Host`]
//! the protocol tests run a [`Connection`] against, and the extension's side
//! of the sealed channel, which opens and checks every reply the way the real
//! one does. Lent to the app's own tests under `test-support`.

use std::cell::{Cell, RefCell};
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD, Engine};
use crypto_box::{aead::Aead, Nonce, PublicKey, SalsaBox, SecretKey};
use serde_json::{json, Map, Value};

use crate::actions::{Client, Connection, Host, Login};
use crate::grant::{GrantReply, GrantRequest, Mode, State, VisibleItem};
use crate::passkeys::{self, Assertion, Registration};
use crate::protocol::{increment, Code, NONCE_LEN, VERSION};
use rowel_core::crypto::{PayloadCipher, VaultKey};
use rowel_core::models::Entry;
use rowel_core::passkey::store::MemoryVault;
use rowel_core::passkey::{Ceremony, UserConsent};
use rowel_core::store::{migrate, SqliteStore, VaultStore};

// --- a vault to talk to ------------------------------------------------------

pub struct Mock {
    pub enabled: Cell<bool>,
    pub locked: Cell<bool>,
    pub logins: Vec<Login>,
    pub clients: RefCell<Vec<Client>>,
    /// What the user answers an `associate` with.
    pub consent: Option<String>,
    /// The yes cannot be kept: the write fails, or the vault changed hands.
    pub remember_fails: Cell<bool>,
    pub totp: Option<String>,
    pub locks: Cell<u32>,
    pub raises: Cell<u32>,
    /// Every `save_login`: id, url, host, username, password.
    pub saves: RefCell<Vec<Save>>,
    /// Whether a save fails as a write, not as an unknown id.
    pub read_only: bool,
    /// The vault's passkeys, and what the user answers a passkey ask with:
    /// whether at all, and which account when it is a sign-in.
    pub passkeys: MemoryVault,
    pub passkey_consent: bool,
    pub passkey_choice: usize,
    /// Every passkey ask the user was shown.
    pub passkey_asks: Arc<Mutex<Vec<String>>>,
    /// What the host answers a grant request with.
    pub grant_answer: GrantAnswer,
    /// Every grant request, with the client key it came in under.
    pub grant_requests: RefCell<Vec<(String, GrantRequest)>>,
    /// The grants in force — id, item, mode — which is what `grant_covers`
    /// answers from. An `Active` answer adds its grant here.
    pub grants: RefCell<Vec<(String, String, Mode)>>,
    /// Every exposure reported, by grant id.
    pub exposures: RefCell<Vec<String>>,
}

pub type Save = (Option<String>, String, String, String, String);

/// How the scripted host answers `request_grant`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantAnswer {
    /// Approved for the one item named; the grant then covers it for a fill.
    Active(String),
    Denied,
    /// Another ask holds the dialog; retry after this long, when told.
    Pending(Option<u64>),
    /// Agents are off, as the app answers today.
    Disabled,
}

/// The id of the grant an `Active` answer issues.
pub const GRANT_ID: &str = "grant-1";

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
    pub fn unlocked() -> Self {
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
            grant_answer: GrantAnswer::Disabled,
            grant_requests: RefCell::new(Vec::new()),
            grants: RefCell::new(Vec::new()),
            exposures: RefCell::new(Vec::new()),
        }
    }

    /// With a grant already in force for `item_id` in `mode`.
    pub fn granting(self, grant_id: &str, item_id: &str, mode: Mode) -> Self {
        self.grants
            .borrow_mut()
            .push((grant_id.into(), item_id.into(), mode));
        self
    }

    fn answer(&self) -> Answer {
        Answer {
            allow: self.passkey_consent,
            choice: self.passkey_choice,
            asks: self.passkey_asks.clone(),
        }
    }

    pub fn known(self, name: &str, key: &str) -> Self {
        self.clients.borrow_mut().push(Client {
            name: name.into(),
            key: key.into(),
        });
        self
    }
}

pub fn login(id: &str, title: &str, username: &str, password: &str, totp: Option<&str>) -> Login {
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
    fn logins_for(&self, _page_url: &str, host: &str) -> Vec<Login> {
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
    fn request_grant(&self, client_key: &str, req: GrantRequest) -> Result<GrantReply, Code> {
        self.grant_requests
            .borrow_mut()
            .push((client_key.to_string(), req));
        let reply = |state, retry_after_ms, items| GrantReply {
            grant_id: GRANT_ID.into(),
            state,
            expires_at_ms: (state == State::Active).then_some(900_000),
            retry_after_ms,
            items,
        };
        match &self.grant_answer {
            GrantAnswer::Disabled => Err(Code::AgentsDisabled),
            GrantAnswer::Denied => Ok(reply(State::Denied, None, Vec::new())),
            GrantAnswer::Pending(retry) => Ok(reply(State::Pending, *retry, Vec::new())),
            GrantAnswer::Active(item) => {
                self.grants
                    .borrow_mut()
                    .push((GRANT_ID.into(), item.clone(), Mode::Fill));
                let items = self
                    .logins
                    .iter()
                    .filter(|login| login.id == *item)
                    .map(|login| VisibleItem {
                        id: login.id.clone(),
                        title: login.title.clone(),
                        username: Some(login.username.clone()),
                    })
                    .collect();
                Ok(reply(State::Active, None, items))
            }
        }
    }
    fn grant_covers(&self, grant_id: &str, item_id: &str, mode: Mode) -> bool {
        self.grants
            .borrow()
            .iter()
            .any(|(g, i, m)| g == grant_id && i == item_id && *m == mode)
    }
    fn report_exposure(&self, grant_id: &str) {
        self.exposures.borrow_mut().push(grant_id.to_string());
    }
}

// --- the extension's side ----------------------------------------------------

pub struct Extension {
    secret: SecretKey,
    pub host: Option<SalsaBox>,
}

impl Default for Extension {
    fn default() -> Self {
        Self::new()
    }
}

pub fn nonce() -> [u8; NONCE_LEN] {
    rand::random()
}

impl Extension {
    pub fn new() -> Self {
        Self {
            secret: SecretKey::generate(&mut rand::rngs::OsRng),
            host: None,
        }
    }

    pub fn public_key(&self) -> String {
        STANDARD.encode(self.secret.public_key().as_bytes())
    }

    pub fn exchange<H: Host>(&mut self, connection: &mut Connection<H>) -> Value {
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
    pub fn send<H: Host>(
        &self,
        connection: &mut Connection<H>,
        action: &str,
        message: Value,
    ) -> Value {
        self.send_with(connection, action, message, Map::new())
    }

    pub fn send_with<H: Host>(
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

pub fn raw<H: Host>(connection: &mut Connection<H>, request: &Value) -> Value {
    let reply = connection.handle(&serde_json::to_vec(request).unwrap());
    serde_json::from_slice(&reply).unwrap()
}

pub fn error_code(response: &Value) -> u8 {
    response["errorCode"]
        .as_str()
        .expect("a refusal carries errorCode as a string")
        .parse()
        .unwrap()
}

pub fn ready() -> (Extension, Connection<Mock>) {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock::unlocked());
    extension.exchange(&mut connection);
    (extension, connection)
}

// --- a store to talk to -------------------------------------------------------

pub fn vault() -> (tempfile::TempDir, SqliteStore, PayloadCipher) {
    let dir = tempfile::tempdir().unwrap();
    let key = VaultKey::legacy_from_password("pw");
    let store =
        SqliteStore::open(&dir.path().join("vault.db"), key.sqlcipher_key().as_slice()).unwrap();
    (dir, store, key.payload_cipher())
}

pub fn stored(store: &SqliteStore, cipher: &PayloadCipher, id: &str) -> Entry {
    let record = store.get(id).unwrap().unwrap();
    cipher.unseal(&record.id, &record.payload).unwrap()
}

pub fn seed(store: &SqliteStore, cipher: &PayloadCipher, entry: &Entry) {
    let payload = cipher.seal(entry).unwrap();
    store
        .upsert(&migrate::build_record(entry, payload).unwrap())
        .unwrap();
}
