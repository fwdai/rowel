//! A vault to talk to, and an extension to talk to it: the scripted [`Host`]
//! the protocol tests run a [`Connection`] against, and the extension's side
//! of the sealed channel — the production [`Channel`], with the checks a test
//! wants as assertions. Lent to the app's own tests under `test-support`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value};
use zeroize::Zeroizing;

use crate::actions::{Client, ClientKind, Connection, Hello, Host, Login, VisibleRef};
use crate::client::{Channel, ChannelError, Transport};
use crate::grant::{GrantReply, GrantRequest, Mode, ResolveContext, State, Target, VisibleItem};
use crate::passkeys::{self, Assertion, Registration};
use crate::protocol::{Code, NONCE_LEN, VERSION};
use crate::reference::Reference;
use crate::signal::Signal;
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
    /// What `resolve` serves, by (item id, field name as the placeholder
    /// writes it): the logins' secrets and an API key, in the one lockbox
    /// this host has open, [`LOCKBOX`].
    pub secrets: HashMap<(String, String), String>,
    pub clients: RefCell<Vec<Client>>,
    /// What the user answers an `associate` with.
    pub consent: Option<String>,
    /// The app's Agents switch: off, and an agent is refused at the door.
    pub agents_enabled: Cell<bool>,
    /// The yes cannot be kept: the write fails, or the vault changed hands.
    pub remember_fails: Cell<bool>,
    /// Every key `client_seen` was told about, in order.
    pub seen: RefCell<Vec<String>>,
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
    /// Which client each passkey ceremony was asked for.
    pub passkey_requests: RefCell<Vec<Client>>,
    /// What the host answers a grant request with.
    pub grant_answer: GrantAnswer,
    /// Every grant request, with the client key it came in under.
    pub grant_requests: RefCell<Vec<(String, GrantRequest)>>,
    /// The grants the host knows, by id — which is what `grant_covers` and
    /// `grant_status` answer from. An `Active` answer adds its grant here.
    pub grants: RefCell<HashMap<String, MockGrant>>,
    /// Every grant ended by its client, by id.
    pub ended: RefCell<Vec<String>>,
    /// What `visible_items` answers, whoever asks.
    pub visible: Vec<VisibleRef>,
    /// What `take_signals` hands each client, by client key.
    pub signals: RefCell<HashMap<String, Vec<Signal>>>,
    /// Every exposure reported, by grant id.
    pub exposures: RefCell<Vec<String>>,
}

pub type Save = (Option<String>, String, String, String, String);

/// A grant as the scripted host keeps it: who it was issued to, the one item
/// and mode it covers, where (none meaning anywhere), where it is in its
/// life, and who has been served under it — enough for the protocol tests to
/// see every shape the real host sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MockGrant {
    pub client_key: String,
    pub item: String,
    pub mode: Mode,
    pub targets: Vec<Target>,
    pub state: State,
    pub uses: u32,
    pub served_to: Option<String>,
}

/// How the scripted host answers `request_grant`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantAnswer {
    /// Approved for the one item named; the grant then covers it in the
    /// first mode asked for.
    Active(String),
    Denied,
    /// Another ask holds the dialog; retry after this long, when told.
    Pending(Option<u64>),
    /// Agents are off, as the app answers today.
    Disabled,
}

/// The id of the grant an `Active` answer issues.
pub const GRANT_ID: &str = "grant-1";
/// When a grant the scripted host issues ends.
pub const GRANT_EXPIRES_AT_MS: i64 = 900_000;
/// The id of the one lockbox the scripted host has open: what a placeholder
/// must name for `resolve` to look further.
pub const LOCKBOX: &str = "lb";

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
            secrets: [
                ("gh", "password", "hunter2"),
                ("gh", "otp", "123456"),
                ("stripe", "apiKey", "sk_test_mock"),
            ]
            .into_iter()
            .map(|(item, field, value)| ((item.into(), field.into()), value.into()))
            .collect(),
            clients: RefCell::new(Vec::new()),
            consent: Some("Chrome".into()),
            agents_enabled: Cell::new(true),
            remember_fails: Cell::new(false),
            seen: RefCell::new(Vec::new()),
            totp: Some("654321".into()),
            locks: Cell::new(0),
            raises: Cell::new(0),
            saves: RefCell::new(Vec::new()),
            read_only: false,
            passkeys: MemoryVault::new(),
            passkey_consent: true,
            passkey_choice: 0,
            passkey_asks: Arc::default(),
            passkey_requests: RefCell::new(Vec::new()),
            grant_answer: GrantAnswer::Disabled,
            grant_requests: RefCell::new(Vec::new()),
            grants: RefCell::new(HashMap::new()),
            ended: RefCell::new(Vec::new()),
            visible: Vec::new(),
            signals: RefCell::new(HashMap::new()),
            exposures: RefCell::new(Vec::new()),
        }
    }

    /// With a grant already in force for `item_id` in `mode`, anywhere.
    /// Issued to no client this side knows — an agent's, as far as whoever
    /// asks under it can tell — so the first client served under it claims it.
    pub fn granting(self, grant_id: &str, item_id: &str, mode: Mode) -> Self {
        self.grant(grant_id, item_id, mode, Vec::new())
    }

    /// As `granting`, at `target` only: what a sidecar's `resolve` is checked
    /// against.
    pub fn granting_at(self, grant_id: &str, item_id: &str, mode: Mode, target: Target) -> Self {
        self.grant(grant_id, item_id, mode, vec![target])
    }

    fn grant(self, grant_id: &str, item_id: &str, mode: Mode, targets: Vec<Target>) -> Self {
        self.grants.borrow_mut().insert(
            grant_id.into(),
            MockGrant {
                client_key: String::new(),
                item: item_id.into(),
                mode,
                targets,
                state: State::Active,
                uses: 0,
                served_to: None,
            },
        );
        self
    }

    fn answer(&self) -> Answer {
        Answer {
            allow: self.passkey_consent,
            choice: self.passkey_choice,
            asks: self.passkey_asks.clone(),
        }
    }

    /// With an extension already let in under `name`.
    pub fn known(self, name: &str, key: &str) -> Self {
        self.clients
            .borrow_mut()
            .push(client(name, key, &Hello::default()));
        self
    }

    /// With an extension let in under `name` and marked by the user as one
    /// in a browser an AI agent drives.
    pub fn known_driven(self, name: &str, key: &str) -> Self {
        let driven = Client {
            driven_by_agent: true,
            ..client(name, key, &Hello::default())
        };
        self.clients.borrow_mut().push(driven);
        self
    }

    /// With an agent from `platform` already let in under `name`.
    pub fn known_agent(self, name: &str, key: &str, platform: &str) -> Self {
        let hello = Hello {
            kind: ClientKind::Agent,
            platform: Some(platform.into()),
            ..Hello::default()
        };
        self.clients.borrow_mut().push(client(name, key, &hello));
        self
    }

    /// With a sidecar — `rowel run`, `rowel proxy` — already let in under
    /// `name`.
    pub fn known_sidecar(self, name: &str, key: &str) -> Self {
        let hello = Hello {
            kind: ClientKind::Sidecar,
            platform: Some("cli".into()),
            ..Hello::default()
        };
        self.clients.borrow_mut().push(client(name, key, &hello));
        self
    }

    /// The reply for the grant `id` as `client` may see it: its own grants
    /// only, since a grant id is a capability and its state is not for
    /// whoever has heard of it.
    fn status_of(&self, client: &Client, id: &str) -> Result<GrantReply, Code> {
        let grants = self.grants.borrow();
        let grant = grants
            .get(id)
            .filter(|grant| grant.client_key == client.key)
            .ok_or(Code::GrantDenied)?;
        Ok(GrantReply {
            grant_id: id.into(),
            state: grant.state,
            expires_at_ms: Some(GRANT_EXPIRES_AT_MS),
            retry_after_ms: None,
            uses: grant.uses,
            items: self.visible_item(&grant.item).into_iter().collect(),
        })
    }

    fn visible_item(&self, item_id: &str) -> Option<VisibleItem> {
        self.logins
            .iter()
            .find(|login| login.id == item_id)
            .map(|login| VisibleItem {
                id: login.id.clone(),
                title: login.title.clone(),
                username: Some(login.username.clone()),
            })
    }
}

/// A stored client as the scripted host writes one: what the `hello` said,
/// at time zero, since this host has no clock.
fn client(name: &str, key: &str, hello: &Hello) -> Client {
    Client {
        name: name.into(),
        key: key.into(),
        kind: hello.kind,
        platform: hello.platform.clone(),
        created_at_ms: 0,
        last_seen_at_ms: None,
        agent_id: None,
        driven_by_agent: false,
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
    fn associate(&self, key: &str, who: &Hello) -> Result<Client, Code> {
        if who.kind != ClientKind::Extension && !self.agents_enabled.get() {
            return Err(Code::AgentsDisabled);
        }
        let name = self.consent.clone().ok_or(Code::ActionCancelledOrDenied)?;
        if self.remember_fails.get() {
            return Err(Code::AssociationFailed);
        }
        let stored = client(&name, key, who);
        self.clients.borrow_mut().push(stored.clone());
        Ok(stored)
    }
    fn client_seen(&self, key: &str) {
        self.seen.borrow_mut().push(key.to_string());
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
    fn passkey_register(&self, client: &Client, registration: Registration) -> Result<Value, Code> {
        self.passkey_requests.borrow_mut().push(client.clone());
        passkeys::register(&self.passkeys, self.answer(), registration)
    }
    fn passkey_get(&self, client: &Client, assertion: Assertion) -> Result<Value, Code> {
        self.passkey_requests.borrow_mut().push(client.clone());
        passkeys::assert(&self.passkeys, self.answer(), assertion)
    }
    fn lock(&self) {
        self.locks.set(self.locks.get() + 1);
    }
    fn unlock_requested(&self) {
        self.raises.set(self.raises.get() + 1);
    }
    fn request_grant(&self, client: &Client, req: GrantRequest) -> Result<GrantReply, Code> {
        let mode = req.modes.first().copied().unwrap_or(Mode::Fill);
        let targets = req.targets.clone();
        self.grant_requests
            .borrow_mut()
            .push((client.key.clone(), req));
        let reply = |state, retry_after_ms, items| GrantReply {
            grant_id: GRANT_ID.into(),
            state,
            expires_at_ms: (state == State::Active).then_some(GRANT_EXPIRES_AT_MS),
            retry_after_ms,
            uses: 0,
            items,
        };
        match &self.grant_answer {
            GrantAnswer::Disabled => Err(Code::AgentsDisabled),
            GrantAnswer::Denied => Ok(reply(State::Denied, None, Vec::new())),
            GrantAnswer::Pending(retry) => Ok(reply(State::Pending, *retry, Vec::new())),
            GrantAnswer::Active(item) => {
                self.grants.borrow_mut().insert(
                    GRANT_ID.into(),
                    MockGrant {
                        client_key: client.key.clone(),
                        item: item.clone(),
                        mode,
                        targets,
                        state: State::Active,
                        uses: 0,
                        served_to: None,
                    },
                );
                let items = self.visible_item(item).into_iter().collect();
                Ok(reply(State::Active, None, items))
            }
        }
    }
    fn wait_grant(&self, client: &Client, id: &str, _timeout_ms: u64) -> Result<GrantReply, Code> {
        self.status_of(client, id)
    }
    fn grant_status(&self, client: &Client, id: &str) -> Result<GrantReply, Code> {
        self.status_of(client, id)
    }
    fn end_grant(&self, client: &Client, id: &str) -> Result<(), Code> {
        let mut grants = self.grants.borrow_mut();
        let grant = grants
            .get_mut(id)
            .filter(|grant| grant.client_key == client.key)
            .ok_or(Code::GrantDenied)?;
        grant.state = State::Revoked;
        self.ended.borrow_mut().push(id.to_string());
        Ok(())
    }
    fn visible_items(
        &self,
        _client: &Client,
        _lockbox: Option<&str>,
    ) -> Result<Vec<VisibleRef>, Code> {
        if !self.agents_enabled.get() {
            return Err(Code::AgentsDisabled);
        }
        Ok(self.visible.clone())
    }
    fn take_signals(&self, client: &Client) -> Vec<Signal> {
        self.signals
            .borrow_mut()
            .remove(&client.key)
            .unwrap_or_default()
    }
    // The claim as the real host makes it: the grant's own client is always
    // served; anyone else is served only if nobody, or they themselves, were
    // served before. A yes is a use.
    fn grant_covers(&self, client: &Client, grant_id: &str, item_id: &str, mode: Mode) -> bool {
        let mut grants = self.grants.borrow_mut();
        let Some(grant) = grants.get_mut(grant_id) else {
            return false;
        };
        if grant.state != State::Active || grant.item != item_id || grant.mode != mode {
            return false;
        }
        if grant.client_key != client.key {
            match &grant.served_to {
                None => grant.served_to = Some(client.key.clone()),
                Some(served) if *served == client.key => {}
                Some(_) => return false,
            }
        }
        grant.uses += 1;
        true
    }
    // Only the grants this client was issued or already served under: a step
    // that names no grant never claims one.
    fn client_grant_covers(&self, client: &Client, item_id: &str, mode: Mode) -> bool {
        let ids: Vec<String> = self
            .grants
            .borrow()
            .iter()
            .filter(|(_, g)| {
                g.client_key == client.key || g.served_to.as_deref() == Some(&client.key)
            })
            .map(|(id, _)| id.clone())
            .collect();
        ids.iter()
            .any(|id| self.grant_covers(client, id, item_id, mode))
    }
    fn report_exposure(&self, _client: &Client, grant_id: &str) {
        self.exposures.borrow_mut().push(grant_id.to_string());
    }
    // The check as the real host makes it, in one word for every no: the
    // grant exists — anyone's, the id being the capability — and is active,
    // in this lockbox, for this item, in the mode the target implies, at
    // this target (or anywhere), and the item has the field. A yes is a use.
    fn resolve(
        &self,
        _client: &Client,
        grant_id: &str,
        reference: &Reference,
        ctx: &ResolveContext,
    ) -> Result<Zeroizing<String>, Code> {
        let mut grants = self.grants.borrow_mut();
        let grant = grants.get_mut(grant_id).ok_or(Code::GrantDenied)?;
        let probe = ctx.target.probe();
        let covered = grant.state == State::Active
            && reference.lockbox == LOCKBOX
            && grant.item == reference.item
            && grant.mode == ctx.target.mode()
            && (grant.targets.is_empty() || grant.targets.iter().any(|t| t.matches(&probe)));
        if !covered {
            return Err(Code::GrantDenied);
        }
        let value = self
            .secrets
            .get(&(reference.item.clone(), reference.field.to_string()))
            .ok_or(Code::GrantDenied)?;
        grant.uses += 1;
        Ok(Zeroizing::new(value.clone()))
    }
}

// --- the extension's side ----------------------------------------------------

/// The one name the test extension gives itself in every envelope.
const CLIENT_ID: &str = "test";

/// The extension's side of the channel, with the checks the production
/// [`Channel`] makes turned into assertions, so a reply that is not what the
/// host should have sent fails the test where it happens.
pub struct Extension {
    channel: Channel,
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
            channel: Channel::new(),
        }
    }

    pub fn public_key(&self) -> String {
        self.channel.public_key()
    }

    /// The channel underneath, for a test that builds a request by hand.
    pub fn channel(&self) -> &Channel {
        &self.channel
    }

    /// `change-public-keys`, and the host's raw reply. A refusal comes back
    /// as sent; anything else must be a well-formed exchange.
    pub fn exchange<H: Host>(&mut self, connection: &mut Connection<H>) -> Value {
        let (request, nonce) = self.channel.exchange_request(CLIENT_ID);
        let response = raw(connection, &request);
        match self.channel.accept_exchange(&nonce, &response) {
            Ok(version) => assert_eq!(version, VERSION),
            Err(ChannelError::Refused(_)) => {}
            Err(error) => panic!("{error}: {response}"),
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

    /// As `send`, with `outer` on the envelope beside the sealed message —
    /// `triggerUnlock`, which the host reads before it unseals anything.
    pub fn send_with<H: Host>(
        &self,
        connection: &mut Connection<H>,
        action: &str,
        message: Value,
        outer: Map<String, Value>,
    ) -> Value {
        let (mut request, nonce) = self
            .channel
            .seal(CLIENT_ID, action, message)
            .expect("keys exchanged");
        for (key, value) in outer {
            request[key] = value;
        }
        let response = raw(connection, &request);
        match self.channel.open(action, &nonce, &response) {
            Ok(opened) => {
                assert_eq!(opened["version"], VERSION);
                Value::Object(opened)
            }
            Err(ChannelError::Refused(_)) => response,
            Err(error) => panic!("{error}: {response}"),
        }
    }
}

/// A `Connection` is its own transport: the frame goes in, the reply comes
/// out, no socket between. It is what lets the CLI's tests drive a
/// `Peer<Connection<Mock>>` in-process.
impl<H: Host> Transport for Connection<H> {
    fn roundtrip(&mut self, request: &[u8]) -> io::Result<Vec<u8>> {
        Ok(self.handle(request))
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
