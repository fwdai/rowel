//! What a client may ask for, and what it gets.
//!
//! One [`Connection`] per client process: it owns the session keys and
//! remembers which client has proved an association on it. The vault is
//! reached through [`Host`], a handful of operations with no notion of
//! sessions, stores or Tauri, so the whole exchange runs against a fake in
//! the tests and against the app (`super::AppHost`) in the binary.
//!
//! Three kinds of client speak the one protocol ([`ClientKind`]). What each
//! may say is a fixed vocabulary, checked right after the association: an
//! extension fills and so receives secrets; an agent asks for grants and is
//! told *granted*, *denied* or *pending*, never a password; a sidecar asks
//! for grants too and, alone, resolves a placeholder under one — the one
//! reply besides a fill that carries a value. The kind is the stored
//! record's, never what the connection claims.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use zeroize::Zeroizing;

use super::grant::{GrantReply, GrantRequest, ItemSelector, Mode, ResolveContext, State, Target};
use super::passkeys::{self, Assertion, Registration};
use super::protocol::{self, str_of, Code, Session};
use super::reference::Reference;
use super::signal::Signal;

/// A login the extension may fill: the one secret it needs and the identity
/// it shows in its picker. `totp` is the current code, when the entry has a
/// seed, so the extension can fill the second step without another ask.
#[derive(Clone, Debug)]
pub struct Login {
    pub id: String,
    pub title: String,
    pub username: String,
    pub password: String,
    pub totp: Option<String>,
}

/// What a client is, which decides what it may ask for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    /// The browser extension: the one client a secret is served to for a fill.
    #[default]
    Extension,
    /// A coding agent, an MCP server, a hook: asks for grants, never a value.
    Agent,
    /// The CLI or proxy that performs `header` and `env` modes under a grant.
    Sidecar,
}

impl ClientKind {
    /// Whether a client of this kind may say `action` at all, past the
    /// association. The vocabulary is the point of the kinds: a fill's
    /// secrets go only to an extension, a grant is asked for only by an
    /// agent or a sidecar, a placeholder is resolved only by a sidecar, and
    /// the few actions that carry nothing personal are everyone's. An action
    /// not named here is not permitted to anyone, which is the safe answer
    /// for one a later host adds.
    pub fn permits(self, action: &str) -> bool {
        match action {
            "get-databasehash" | "generate-password" | "lock-database" => true,
            "get-logins" | "get-totp" | "set-login" | "passkeys-register" | "passkeys-get"
            | "get-signals" | "report-exposure" => self == ClientKind::Extension,
            "request-grant" | "wait-grant" | "end-grant" | "grant-status" | "list-visible" => {
                self != ClientKind::Extension
            }
            "resolve" => self == ClientKind::Sidecar,
            _ => false,
        }
    }
}

/// A client the user let in: the name they gave it and its identification
/// public key. Kept inside the vault it was let into (see `super::clients`),
/// which is what makes the key worth something: the protocol has no step in
/// which the client proves it holds the private half, so whoever can present
/// the public key is the client as far as the host can tell. A row stored
/// before there were kinds is `{"name","key"}` and reads back as an extension
/// created at time zero, which is what it was.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Client {
    pub name: String,
    /// The identification public key, base64: the whole credential in Tier 1.
    pub key: String,
    #[serde(default)]
    pub kind: ClientKind,
    /// What the client said it runs on when it was let in: `claude-code`,
    /// `codex`, `cursor`, `browser-use`, `cli`, …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default)]
    pub created_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at_ms: Option<i64>,
    /// Tier 2 only: the agent definition this client resolved to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// An extension in a browser an AI agent drives, as the user marked it
    /// in Rowel: every fill it asks for is a grant, whatever the extension
    /// says of its own agent mode, so the agent cannot switch the extension
    /// out of it. Not written while off.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub driven_by_agent: bool,
}

/// What a client says it is, inside the sealed `associate` / `test-associate`.
/// An unmodified KeePassXC-Browser sends none, and none is an extension.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    #[serde(default)]
    pub kind: ClientKind,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// What this broker can do beyond KeePassXC's protocol, told to every client
/// at association. The version string stays `2.7.7` for the extension's sake,
/// so this is the one place a client learns the agent actions exist; an
/// action added later adds a word here, so a client can tell a host that
/// serves it from one that will refuse it.
pub const BROKER_CAPABILITIES: [&str; 3] = ["grants", "signals", "resolve"];

/// An item as an agent may see it: a name to ask for it by, enough to tell the
/// user which account it is, and which fields a grant could cover. Never a
/// value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibleRef {
    /// `rowel://<lockbox id>/<item id>`.
    #[serde(rename = "ref")]
    pub reference: String,
    pub title: String,
    pub kind: String,
    pub fields: Vec<String>,
}

/// The vault as the client host sees it.
pub trait Host {
    /// Whether the host is still switched on. Asked before every request, so
    /// turning it off in Settings ends the connections that are up, not only
    /// the ones to come.
    fn enabled(&self) -> bool;
    /// A stable digest of the open vault, or `None` while locked. The
    /// extension keys its associations by it, so each workspace is its own.
    fn database_hash(&self) -> Option<String>;
    /// The clients let into the open vault. Empty while locked.
    fn clients(&self) -> Vec<Client>;
    /// Ask the user whether the client holding `key`, which says it is `who`,
    /// may be let in, and let it into the open vault if they say so: the
    /// record as stored. A refusal or no answer is `ActionCancelledOrDenied`;
    /// an agent is refused with `AgentsDisabled` while the app's Agents switch
    /// is off, without a dialog; a yes that could not be kept — the write
    /// failed, or the vault that asked is no longer the one open — is
    /// `AssociationFailed`, so the client is never told it is in when the next
    /// request will find it is not.
    fn associate(&self, key: &str, who: &Hello) -> Result<Client, Code>;
    /// A known client proved itself again (`test-associate`): the host notes
    /// `last_seen_at_ms`. A host with nothing to note need not answer.
    fn client_seen(&self, _key: &str) {}
    /// Every login for the page at `page_url`, whose host is `host` (a bare
    /// hostname, lowercase), secrets unsealed. A login saved for `https://`
    /// is not one an `http://` page gets, nor one saved with a port another
    /// port gets ([`site_matches`]).
    fn logins_for(&self, page_url: &str, host: &str) -> Vec<Login>;
    /// The current code for the login `id`, if it has a seed.
    fn totp(&self, id: &str) -> Option<String>;
    fn generate_password(&self) -> Option<String>;
    /// Save what the user typed into a page: over the login `id`'s username
    /// and password, or as a new login for `url` titled `host`. An `id` the
    /// vault holds no login under is [`Code::NoValidUuidProvided`]; any other
    /// error is a write that failed.
    fn save_login(
        &self,
        id: Option<&str>,
        url: &str,
        host: &str,
        username: &str,
        password: &str,
    ) -> Result<(), Code>;
    /// Create a passkey in the open vault, asking the user first and telling
    /// them which client asks: the credential for the page, or the ceremony's
    /// error code. The whole of it is `passkeys::register` over this host's
    /// vault and prompt.
    fn passkey_register(&self, client: &Client, registration: Registration) -> Result<Value, Code>;
    /// Sign in with a passkey from the open vault, asking the user first.
    fn passkey_get(&self, client: &Client, assertion: Assertion) -> Result<Value, Code>;
    fn lock(&self);
    /// The extension wants the vault open: bring the app forward.
    fn unlock_requested(&self);
    /// A client asks to use items at targets in modes. Blocks for the user's
    /// answer in Tier 1, like `associate`; `pending` with `retry_after_ms`
    /// when another ask holds the dialog.
    fn request_grant(&self, client: &Client, req: GrantRequest) -> Result<GrantReply, Code>;
    /// The state of grant `id` once it is settled, or after `timeout_ms`. In
    /// Tier 1 a grant is settled when `request_grant` answers, so this is the
    /// status at once; the long poll is Tier 2's.
    fn wait_grant(&self, client: &Client, id: &str, timeout_ms: u64) -> Result<GrantReply, Code>;
    /// The state of grant `id`, `uses` filled. Only for the grant's own client.
    fn grant_status(&self, client: &Client, id: &str) -> Result<GrantReply, Code>;
    /// The client is done with grant `id`: revoke it and sign out of its sites.
    fn end_grant(&self, client: &Client, id: &str) -> Result<(), Code>;
    /// The items `client` may ask for by name: those its own rules and active
    /// grants admit, in `lockbox` or the open one. Never the whole vault.
    /// A closed vault or either disabled agent-access switch is an explicit
    /// refusal, not a successful empty list.
    fn visible_items(
        &self,
        client: &Client,
        lockbox: Option<&str>,
    ) -> Result<Vec<VisibleRef>, Code>;
    /// The signals queued for `client` since it last asked, which this call
    /// clears.
    fn take_signals(&self, client: &Client) -> Vec<Signal>;
    /// Whether grant `grant_id` is active and covers `item_id` in `mode`
    /// right now, for `client`: a fill grant an agent requested is claimed by
    /// the first extension served under it and refused to any other
    /// (`Grant::served_to`). A yes is a use.
    fn grant_covers(&self, client: &Client, grant_id: &str, item_id: &str, mode: Mode) -> bool;
    /// [`Host::grant_covers`] over the grants `client` holds or was served
    /// under, for a step that names none: the TOTP step of an extension in
    /// agent mode that sent no `grantId`. Never claims a grant it was not
    /// served under.
    fn client_grant_covers(&self, client: &Client, item_id: &str, mode: Mode) -> bool;
    /// The extension found a filled secret showing on the page: end the grant
    /// and log it.
    fn report_exposure(&self, client: &Client, grant_id: &str);
    /// The value a placeholder names, for a sidecar acting under grant
    /// `grant_id` at `ctx.target` — the only reply in the protocol that
    /// carries a secret to anything but an extension. The host checks that
    /// the grant is active, was issued in the lockbox `reference` names and
    /// the open one, admits the item, covers the target in the mode the
    /// target implies ([`ResolveTarget::mode`](super::grant::ResolveTarget::mode)),
    /// and that the field exists on the item; a yes is a use, counted and
    /// written as a `used` row. Everything that is not a yes is `GrantDenied`
    /// (or `GrantExpired` from `Grant::use_once`), whatever the reason: a
    /// grant id is a capability, and why it did not work is not for whoever
    /// has heard of it. The default refuses, so a host that has not
    /// implemented the sidecar's verb serves nothing by omission.
    fn resolve(
        &self,
        _client: &Client,
        _grant_id: &str,
        _reference: &Reference,
        _ctx: &ResolveContext,
    ) -> Result<Zeroizing<String>, Code> {
        Err(Code::GrantDenied)
    }
}

/// A refusal as `dispatch` raises one: the code, and — for a grant that is
/// still pending because another ask holds the dialog — when to ask again.
/// Rides in the clear beside `errorCode`, where the extension reads only the
/// fields it knows.
struct Refusal {
    code: Code,
    retry_after_ms: Option<u64>,
}

impl From<Code> for Refusal {
    fn from(code: Code) -> Self {
        Self {
            code,
            retry_after_ms: None,
        }
    }
}

pub struct Connection<H> {
    host: H,
    session: Session,
    /// The stored record of the client this connection proved, if any. Held
    /// as the record rather than a flag: its key is checked against the open
    /// vault's list on every use, so forgetting the client in Settings, or
    /// switching to a workspace it was never let into, ends its access with
    /// the next request. Its kind is the lockbox's word, not the connection's.
    associated: Option<Client>,
}

impl<H: Host> Connection<H> {
    pub fn new(host: H) -> Self {
        Self {
            host,
            session: Session::new(),
            associated: None,
        }
    }

    pub fn host(&self) -> &H {
        &self.host
    }

    /// One request in, one reply out, both JSON. Never fails: whatever cannot
    /// be answered is refused in the protocol's own words.
    pub fn handle(&mut self, raw: &[u8]) -> Vec<u8> {
        serde_json::to_vec(&self.reply(raw)).unwrap_or_default()
    }

    fn reply(&mut self, raw: &[u8]) -> Value {
        let Ok(Value::Object(request)) = serde_json::from_slice::<Value>(raw) else {
            return protocol::error_reply("", Code::EmptyMessageReceived);
        };
        let action = str_of(&request, "action").to_string();
        if action == "change-public-keys" {
            return self.session.exchange(&request);
        }
        // Everything else needs the vault open, and is refused before it is
        // even unsealed — as KeePassXC does. `triggerUnlock` rides on the
        // outside of the request for exactly this case.
        let Some(hash) = self.host.database_hash() else {
            if action == "get-databasehash" && str_of(&request, "triggerUnlock") == "true" {
                self.host.unlock_requested();
            }
            return protocol::error_reply(&action, Code::DatabaseNotOpened);
        };
        let (message, nonce) = match self.session.open(&request) {
            Ok(opened) => opened,
            Err(code) => return protocol::error_reply(&action, code),
        };
        if str_of(&message, "action") != action {
            return protocol::error_reply(&action, Code::IncorrectAction);
        }
        match self.dispatch(&action, &message, &hash) {
            Ok(params) => self.session.seal(&action, &nonce, params),
            Err(refusal) => {
                let mut reply = protocol::error_reply(&action, refusal.code);
                if let Some(ms) = refusal.retry_after_ms {
                    reply["retryAfterMs"] = ms.into();
                }
                reply
            }
        }
    }

    fn dispatch(
        &mut self,
        action: &str,
        message: &Map<String, Value>,
        hash: &str,
    ) -> Result<Map<String, Value>, Refusal> {
        match action {
            "get-databasehash" => Ok(params(json!({ "hash": hash }))),
            "associate" => {
                // `key` is the session key the message is sealed under, sent
                // again inside it; `idKey` is the one the client keeps.
                let key = str_of(message, "key");
                let id_key = str_of(message, "idKey");
                if key.is_empty() || Some(key) != self.session.client_key() || id_key.is_empty() {
                    return Err(Code::AssociationFailed.into());
                }
                let hello = hello_of(message)?;
                let client = self.host.associate(id_key, &hello)?;
                let reply = association_reply(hash, &client);
                self.associated = Some(client);
                Ok(reply)
            }
            "test-associate" => {
                let id = str_of(message, "id");
                let key = str_of(message, "key");
                // Read for its shape only: what the client is was settled when
                // it was let in, and a malformed one is refused like any other
                // malformed association.
                hello_of(message)?;
                let known = self
                    .host
                    .clients()
                    .into_iter()
                    .find(|c| c.key == key && c.name == id)
                    .ok_or(Code::AssociationFailed)?;
                self.host.client_seen(key);
                let reply = association_reply(hash, &known);
                self.associated = Some(known);
                Ok(reply)
            }
            "get-logins" => {
                let client = self.admit(action, message)?;
                let url = str_of(message, "url");
                if url.is_empty() {
                    return Err(Code::NoUrlProvided.into());
                }
                let host = site_host(url).ok_or(Code::NoUrlProvided)?;
                let logins = self.host.logins_for(url, &host);
                if logins.is_empty() {
                    return Err(Code::NoLoginsFound.into());
                }
                // In agent mode a fill is a grant: the page's logins are the
                // candidates, the host asks (or finds a standing rule), and
                // only what was admitted is served, under the grant's id. A
                // grant the extension already holds is served without asking
                // again, and wins over `agentMode` when both are sent — the
                // grant is the more specific instruction, and asking again
                // would raise a second dialog for a yes already given. An
                // extension the user marked as driven by an agent is in agent
                // mode whatever its message says.
                let grant_id = str_of(message, "grantId");
                let (logins, grant_id) = if !grant_id.is_empty() {
                    let covered: Vec<Login> = logins
                        .into_iter()
                        .filter(|login| {
                            self.host
                                .grant_covers(&client, grant_id, &login.id, Mode::Fill)
                        })
                        .collect();
                    if covered.is_empty() {
                        return Err(Code::GrantDenied.into());
                    }
                    (covered, None)
                } else if agent_mode(&client, message) {
                    let request = GrantRequest {
                        client_key: client.key.clone(),
                        agent_id: None,
                        acting_for: None,
                        items: ItemSelector::Ids(logins.iter().map(|l| l.id.clone()).collect()),
                        targets: vec![Target::Origin(url.to_string())],
                        modes: vec![Mode::Fill],
                        run_id: None,
                        reason: None,
                    };
                    let reply = self.host.request_grant(&client, request)?;
                    match reply.state {
                        State::Active => {}
                        State::Pending => {
                            return Err(Refusal {
                                code: Code::GrantPending,
                                retry_after_ms: reply.retry_after_ms,
                            })
                        }
                        _ => return Err(Code::GrantDenied.into()),
                    }
                    let admitted: Vec<Login> = logins
                        .into_iter()
                        .filter(|login| reply.items.iter().any(|item| item.id == login.id))
                        .collect();
                    if admitted.is_empty() {
                        return Err(Code::GrantDenied.into());
                    }
                    (admitted, Some(reply.grant_id))
                } else {
                    (logins, None)
                };
                let entries: Vec<Value> = logins
                    .iter()
                    .map(|login| {
                        let mut entry = json!({
                            "login": login.username,
                            "password": login.password,
                            "name": login.title,
                            "uuid": login.id,
                        });
                        if let Some(code) = &login.totp {
                            entry["totp"] = code.as_str().into();
                        }
                        entry
                    })
                    .collect();
                let mut reply = json!({ "count": entries.len(), "entries": entries, "hash": hash });
                if let Some(grant_id) = grant_id {
                    reply["grantId"] = grant_id.into();
                }
                Ok(params(reply))
            }
            "get-totp" => {
                let client = self.admit(action, message)?;
                let id = str_of(message, "uuid");
                if id.is_empty() {
                    return Err(Code::NoValidUuidProvided.into());
                }
                // Under a grant, the code goes only with a login the grant
                // covers for a fill — checked before any code is generated.
                // In agent mode there is no code without a grant: the one the
                // step names by `grantId`, or else one the client holds for
                // the login (a marked extension may send neither flag nor
                // id). Refused otherwise, rather than served the way a
                // person's click would be.
                let grant_id = str_of(message, "grantId");
                let covered = if !grant_id.is_empty() {
                    self.host.grant_covers(&client, grant_id, id, Mode::Fill)
                } else if agent_mode(&client, message) {
                    self.host.client_grant_covers(&client, id, Mode::Fill)
                } else {
                    true
                };
                if !covered {
                    return Err(Code::GrantDenied.into());
                }
                let code = self.host.totp(id).ok_or(Code::NoValidUuidProvided)?;
                Ok(params(json!({ "totp": code })))
            }
            "set-login" => {
                self.admit(action, message)?;
                let url = str_of(message, "url");
                let host = site_host(url).ok_or(Code::NoUrlProvided)?;
                let id = Some(str_of(message, "uuid")).filter(|id| !id.is_empty());
                let saved = self.host.save_login(
                    id,
                    url,
                    &host,
                    str_of(message, "login"),
                    str_of(message, "password"),
                );
                // KeePassXC answers a save with its outcome inside a sealed
                // reply, not with a refusal; the extension reads `error`.
                let error = match saved {
                    Ok(()) => "success",
                    Err(Code::NoValidUuidProvided) => return Err(Code::NoValidUuidProvided.into()),
                    Err(_) => "error",
                };
                Ok(params(
                    json!({ "count": null, "entries": null, "error": error, "hash": hash }),
                ))
            }
            "generate-password" => {
                let password = self
                    .host
                    .generate_password()
                    .ok_or(Code::ActionCancelledOrDenied)?;
                Ok(params(json!({ "password": password })))
            }
            // Past the association check, a passkey ceremony answers inside a
            // sealed reply whatever happens — the page reads its outcome from
            // `response.errorCode`, and a refusal in the clear would reach it
            // only as "cancelled".
            "passkeys-register" => {
                let client = self.admit(action, message)?;
                let outcome = passkeys::registration(message)
                    .and_then(|r| self.host.passkey_register(&client, r));
                Ok(passkeys::reply(outcome))
            }
            "passkeys-get" => {
                let client = self.admit(action, message)?;
                let outcome =
                    passkeys::assertion(message).and_then(|a| self.host.passkey_get(&client, a));
                Ok(passkeys::reply(outcome))
            }
            // Locking takes no secret, but it is the user's session to end:
            // a client that was never let in does not get to end it.
            "lock-database" => {
                self.admit(action, message)?;
                self.host.lock();
                Ok(Map::new())
            }
            // The page showed a secret the extension had filled: the grant
            // ends and the log says so. Nothing comes back, but it is the
            // user's grant to end, so a stranger is refused as for a lock.
            "report-exposure" => {
                let client = self.admit(action, message)?;
                let grant_id = grant_id_of(message)?;
                self.host.report_exposure(&client, grant_id);
                Ok(Map::new())
            }
            // The extension was woken by `rowel-signal`: what the host has
            // queued for it, which the host forgets as it hands over.
            "get-signals" => {
                let client = self.admit(action, message)?;
                let signals = self.host.take_signals(&client);
                Ok(params(json!({ "signals": signals })))
            }
            // An agent asks. The request is read strictly — a selector, at
            // least one target, at least one mode — because a grant is never
            // wider than its request, and a request the host had to guess at
            // would be. `runId` and `reason` are the agent's own words,
            // recorded and bounded, never trusted for anything.
            "request-grant" => {
                let client = self.admit(action, message)?;
                let run_id = str_of(message, "runId");
                if run_id.chars().count() > MAX_RUN_ID_CHARS {
                    return Err(Code::GrantDenied.into());
                }
                let mut reason = str_of(message, "reason").to_string();
                clamp(&mut reason, MAX_REASON_CHARS);
                let items: ItemSelector = field_of(message, "items").ok_or(Code::GrantDenied)?;
                let targets: Vec<Target> = field_of(message, "targets").ok_or(Code::GrantDenied)?;
                let modes: Vec<Mode> = field_of(message, "modes").ok_or(Code::GrantDenied)?;
                if targets.is_empty() || modes.is_empty() {
                    return Err(Code::GrantDenied.into());
                }
                let request = GrantRequest {
                    client_key: client.key.clone(),
                    agent_id: client.agent_id.clone(),
                    acting_for: None,
                    items,
                    targets,
                    modes,
                    run_id: (!run_id.is_empty()).then(|| run_id.to_string()),
                    reason: (!reason.is_empty()).then_some(reason),
                };
                let reply = self.host.request_grant(&client, request)?;
                Ok(grant_reply(&reply))
            }
            "wait-grant" => {
                let client = self.admit(action, message)?;
                let grant_id = grant_id_of(message)?;
                let timeout_ms = message
                    .get("timeoutMs")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .min(MAX_WAIT_MS);
                let reply = self.host.wait_grant(&client, grant_id, timeout_ms)?;
                Ok(grant_reply(&reply))
            }
            "grant-status" => {
                let client = self.admit(action, message)?;
                let grant_id = grant_id_of(message)?;
                let reply = self.host.grant_status(&client, grant_id)?;
                Ok(grant_reply(&reply))
            }
            "end-grant" => {
                let client = self.admit(action, message)?;
                let grant_id = grant_id_of(message)?;
                self.host.end_grant(&client, grant_id)?;
                Ok(params(json!({ "ok": true })))
            }
            "list-visible" => {
                let client = self.admit(action, message)?;
                let lockbox = Some(str_of(message, "lockbox")).filter(|l| !l.is_empty());
                let items = self.host.visible_items(&client, lockbox)?;
                Ok(params(json!({ "items": items })))
            }
            // A sidecar names a placeholder and where the value is going, and
            // the host answers under the grant. A `ref` or `context` that
            // does not read is `GrantDenied`, the same word as a grant that
            // does not cover it: the grant id is a capability, and a request
            // under one is not told which part of it was wrong.
            "resolve" => {
                let client = self.admit(action, message)?;
                let grant_id = grant_id_of(message)?;
                let reference =
                    Reference::parse(str_of(message, "ref")).ok_or(Code::GrantDenied)?;
                let ctx: ResolveContext = field_of(message, "context").ok_or(Code::GrantDenied)?;
                let value = self.host.resolve(&client, grant_id, &reference, &ctx)?;
                // From here the value lives in a `serde_json::Value`: the
                // sealed reply is built in memory and leaves through the
                // connection's buffers, the same ones that carry a fill's
                // password. The `Zeroizing` wrapper covers the host's copy;
                // the reply's copy lives exactly as long as a fill's does.
                Ok(params(json!({ "value": value.as_str() })))
            }
            // The extension asks for groups before saving a new login; a vault
            // has none, and this is the code it takes for "put it at the root".
            "get-database-groups" => Err(Code::NoGroupsFound.into()),
            _ => Err(Code::IncorrectAction.into()),
        }
    }

    /// The client behind an action that needs one: associated, and of a kind
    /// that may say `action`. The kind is checked second, so a stranger hears
    /// the refusal it always heard and a known client hears which door it
    /// knocked on.
    fn admit(&mut self, action: &str, message: &Map<String, Value>) -> Result<Client, Code> {
        let client = self.require_association(message)?;
        if !client.kind.permits(action) {
            return Err(Code::NotPermittedForKind);
        }
        Ok(client)
    }

    // An association proved on this connection, or one the request carries:
    // the extension sends every identification key it holds with each ask,
    // and one the user let in before is as good as a fresh `test-associate`.
    // Either way the key has to be in the *open* vault's list now — not when
    // it was proved — so a forget, or a switch to another workspace, takes
    // effect on the next request rather than at the next reconnect. What
    // comes back is the stored record as it is now, kind included.
    fn require_association(&mut self, message: &Map<String, Value>) -> Result<Client, Code> {
        let known = self.host.clients();
        let stored = |key: &str| known.iter().find(|c| c.key == key).cloned();
        if let Some(client) = self
            .associated
            .as_ref()
            .and_then(|client| stored(&client.key))
        {
            self.associated = Some(client.clone());
            return Ok(client);
        }
        let offered = message
            .get("keys")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
            .map(|offer| str_of(offer, "key"))
            .find_map(stored);
        match offered {
            Some(client) => {
                self.associated = Some(client.clone());
                Ok(client)
            }
            None => {
                self.associated = None;
                Err(Code::AssociationFailed)
            }
        }
    }
}

/// The most an agent may say about itself, per field, so a stored record and
/// a dialog line stay a line.
const MAX_PLATFORM_CHARS: usize = 64;
const MAX_VERSION_CHARS: usize = 32;
const MAX_CAPABILITY_CHARS: usize = 32;
/// Spec `02`: a run id is opaque and at most 64 characters; a reason is at
/// most 300, and is cut rather than refused because it is for the dialog and
/// the log, not for the decision.
const MAX_RUN_ID_CHARS: usize = 64;
const MAX_REASON_CHARS: usize = 300;
/// Spec `04`: the longest a `wait-grant` may ask to wait.
const MAX_WAIT_MS: u64 = 120_000;

/// The `hello` inside an association: absent is an extension that predates
/// kinds, present is read strictly and bounded. One that does not read is
/// refused as the association it is part of.
fn hello_of(message: &Map<String, Value>) -> Result<Hello, Code> {
    let mut hello = match message.get("hello") {
        None | Some(Value::Null) => Hello::default(),
        Some(hello) => {
            serde_json::from_value(hello.clone()).map_err(|_| Code::AssociationFailed)?
        }
    };
    if let Some(platform) = hello.platform.as_mut() {
        clamp(platform, MAX_PLATFORM_CHARS);
    }
    if let Some(version) = hello.version.as_mut() {
        clamp(version, MAX_VERSION_CHARS);
    }
    for capability in hello.capabilities.iter_mut() {
        clamp(capability, MAX_CAPABILITY_CHARS);
    }
    Ok(hello)
}

/// What every client hears when it is in: the hash and the name, as KeePassXC
/// answers, plus what this broker can do beyond that, and — for a client that
/// is not an extension — its id key under the name the agent protocol gives it.
fn association_reply(hash: &str, client: &Client) -> Map<String, Value> {
    let mut reply = json!({
        "hash": hash,
        "id": client.name,
        "brokerCapabilities": BROKER_CAPABILITIES,
    });
    if client.kind != ClientKind::Extension {
        reply["clientId"] = client.key.as_str().into();
    }
    params(reply)
}

/// A `grantId` the message must carry; the code for a missing one is the
/// protocol's for a missing identifier, as `report-exposure` answers.
fn grant_id_of(message: &Map<String, Value>) -> Result<&str, Code> {
    Some(str_of(message, "grantId"))
        .filter(|id| !id.is_empty())
        .ok_or(Code::NoValidUuidProvided)
}

/// A typed field of the message, or `None` when it is absent or not of that
/// shape.
fn field_of<T: for<'de> Deserialize<'de>>(message: &Map<String, Value>, key: &str) -> Option<T> {
    serde_json::from_value(message.get(key)?.clone()).ok()
}

/// A `GrantReply` as the agent reads it: its own camelCase serialisation, the
/// one shape every grant action answers with.
fn grant_reply(reply: &GrantReply) -> Map<String, Value> {
    params(serde_json::to_value(reply).unwrap_or_default())
}

/// Cut `text` to at most `max_chars` characters, on a character boundary, so
/// a bound is never a panic inside a multi-byte character.
pub fn clamp(text: &mut String, max_chars: usize) {
    if let Some((end, _)) = text.char_indices().nth(max_chars) {
        text.truncate(end);
    }
}

fn params(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// Agent mode, as the extension says it or as the user marked the client in
/// Rowel. Either turns it on; the extension cannot turn the mark off.
fn agent_mode(client: &Client, message: &Map<String, Value>) -> bool {
    client.driven_by_agent || flag_of(message, "agentMode")
}

/// A flag as the extension sends one — the string `"true"`, which is how
/// KeePassXC-Browser writes every boolean — or as a JSON `true`.
fn flag_of(map: &Map<String, Value>, key: &str) -> bool {
    match map.get(key) {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::String(text)) => text == "true",
        _ => false,
    }
}

// Which logins belong to a site is the core's rule, so the iOS AutoFill
// extension serves a site exactly what this host does.
pub use rowel_core::host::{host_matches, is_public_suffix, site_host, site_matches};
