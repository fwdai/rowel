//! What the extension may ask for, and what it gets.
//!
//! One [`Connection`] per extension process: it owns the session keys and
//! remembers whether the extension has proved an association on it. The vault
//! is reached through [`Host`], a handful of operations with no notion of
//! sessions, stores or Tauri, so the whole exchange runs against a fake in
//! the tests and against the app (`super::AppHost`) in the binary.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::grant::{GrantReply, GrantRequest, ItemSelector, Mode, State, Target};
use super::passkeys::{self, Assertion, Registration};
use super::protocol::{self, str_of, Code, Session};

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

/// An extension the user let in: the name they gave it and its identification
/// public key. Kept inside the vault it was let into (see `super::clients`),
/// which is what makes the key worth something: the protocol has no step in
/// which the extension proves it holds the private half, so whoever can
/// present the public key is the extension as far as the host can tell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Client {
    pub name: String,
    pub key: String,
}

/// The vault as the extension host sees it.
pub trait Host {
    /// Whether the host is still switched on. Asked before every request, so
    /// turning it off in Settings ends the connections that are up, not only
    /// the ones to come.
    fn enabled(&self) -> bool;
    /// A stable digest of the open vault, or `None` while locked. The
    /// extension keys its associations by it, so each workspace is its own.
    fn database_hash(&self) -> Option<String>;
    /// The extensions let into the open vault. Empty while locked.
    fn clients(&self) -> Vec<Client>;
    /// Ask the user whether the extension holding `key` may connect, and let
    /// it into the open vault if they say so: the name they gave it. A
    /// refusal or no answer is `ActionCancelledOrDenied`; a yes that could
    /// not be kept — the write failed, or the vault that asked is no longer
    /// the one open — is `AssociationFailed`, so the extension is never told
    /// it is in when the next request will find it is not.
    fn associate(&self, key: &str) -> Result<String, Code>;
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
    /// Create a passkey in the open vault, asking the user first: the
    /// credential for the page, or the ceremony's error code. The whole of it
    /// is `passkeys::register` over this host's vault and prompt.
    fn passkey_register(&self, registration: Registration) -> Result<Value, Code>;
    /// Sign in with a passkey from the open vault, asking the user first.
    fn passkey_get(&self, assertion: Assertion) -> Result<Value, Code>;
    fn lock(&self);
    /// The extension wants the vault open: bring the app forward.
    fn unlock_requested(&self);
    /// The extension (or, later, an agent) asks to use items at targets in
    /// modes. Blocks for the user's answer in Tier 1, like `associate`.
    fn request_grant(&self, client_key: &str, req: GrantRequest) -> Result<GrantReply, Code>;
    /// Whether grant `id` is active and covers `item_id` in `mode` right now.
    fn grant_covers(&self, grant_id: &str, item_id: &str, mode: Mode) -> bool;
    /// The extension found a filled secret showing on the page: end the grant
    /// and log it.
    fn report_exposure(&self, grant_id: &str);
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
    /// The identification key this connection proved, if any. Held as the key
    /// rather than a flag: it is checked against the open vault's list on
    /// every use, so forgetting the extension in Settings, or switching to a
    /// workspace it was never let into, ends its access with the next request.
    associated: Option<String>,
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
                // again inside it; `idKey` is the one the extension keeps.
                let key = str_of(message, "key");
                let id_key = str_of(message, "idKey");
                if key.is_empty() || Some(key) != self.session.client_key() || id_key.is_empty() {
                    return Err(Code::AssociationFailed.into());
                }
                let name = self.host.associate(id_key)?;
                self.associated = Some(id_key.to_string());
                Ok(params(json!({ "hash": hash, "id": name })))
            }
            "test-associate" => {
                let id = str_of(message, "id");
                let key = str_of(message, "key");
                let known = self
                    .host
                    .clients()
                    .iter()
                    .any(|c| c.key == key && c.name == id);
                if !known {
                    return Err(Code::AssociationFailed.into());
                }
                self.associated = Some(key.to_string());
                Ok(params(json!({ "hash": hash, "id": id })))
            }
            "get-logins" => {
                self.require_association(message)?;
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
                // would raise a second dialog for a yes already given.
                let grant_id = str_of(message, "grantId");
                let (logins, grant_id) = if !grant_id.is_empty() {
                    let covered: Vec<Login> = logins
                        .into_iter()
                        .filter(|login| self.host.grant_covers(grant_id, &login.id, Mode::Fill))
                        .collect();
                    if covered.is_empty() {
                        return Err(Code::GrantDenied.into());
                    }
                    (covered, None)
                } else if flag_of(message, "agentMode") {
                    let client_key = self.associated.clone().ok_or(Code::AssociationFailed)?;
                    let request = GrantRequest {
                        client_key: client_key.clone(),
                        agent_id: None,
                        acting_for: None,
                        items: ItemSelector::Ids(logins.iter().map(|l| l.id.clone()).collect()),
                        targets: vec![Target::Origin(url.to_string())],
                        modes: vec![Mode::Fill],
                        run_id: None,
                        reason: None,
                    };
                    let reply = self.host.request_grant(&client_key, request)?;
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
                self.require_association(message)?;
                let id = str_of(message, "uuid");
                if id.is_empty() {
                    return Err(Code::NoValidUuidProvided.into());
                }
                // Under a grant, the code goes only with a login the grant
                // covers for a fill — checked before any code is generated.
                // In agent mode there is no code without a grant: the
                // extension says `agentMode` on every TOTP step, with the
                // fill's `grantId` when it holds one, and a step that has
                // none (or a grant that does not cover the login) is refused
                // rather than served the way a person's click would be.
                let grant_id = str_of(message, "grantId");
                if flag_of(message, "agentMode") && grant_id.is_empty() {
                    return Err(Code::GrantDenied.into());
                }
                if !grant_id.is_empty() && !self.host.grant_covers(grant_id, id, Mode::Fill) {
                    return Err(Code::GrantDenied.into());
                }
                let code = self.host.totp(id).ok_or(Code::NoValidUuidProvided)?;
                Ok(params(json!({ "totp": code })))
            }
            "set-login" => {
                self.require_association(message)?;
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
                self.require_association(message)?;
                let outcome =
                    passkeys::registration(message).and_then(|r| self.host.passkey_register(r));
                Ok(passkeys::reply(outcome))
            }
            "passkeys-get" => {
                self.require_association(message)?;
                let outcome = passkeys::assertion(message).and_then(|a| self.host.passkey_get(a));
                Ok(passkeys::reply(outcome))
            }
            // Locking takes no secret, but it is the user's session to end:
            // an extension that was never let in does not get to end it.
            "lock-database" => {
                self.require_association(message)?;
                self.host.lock();
                Ok(Map::new())
            }
            // The page showed a secret the extension had filled: the grant
            // ends and the log says so. Nothing comes back, but it is the
            // user's grant to end, so a stranger is refused as for a lock.
            "report-exposure" => {
                self.require_association(message)?;
                let grant_id = str_of(message, "grantId");
                if grant_id.is_empty() {
                    return Err(Code::NoValidUuidProvided.into());
                }
                self.host.report_exposure(grant_id);
                Ok(Map::new())
            }
            // The extension asks for groups before saving a new login; a vault
            // has none, and this is the code it takes for "put it at the root".
            "get-database-groups" => Err(Code::NoGroupsFound.into()),
            _ => Err(Code::IncorrectAction.into()),
        }
    }

    // An association proved on this connection, or one the request carries:
    // the extension sends every identification key it holds with each ask,
    // and one the user let in before is as good as a fresh `test-associate`.
    // Either way the key has to be in the *open* vault's list now — not when
    // it was proved — so a forget, or a switch to another workspace, takes
    // effect on the next request rather than at the next reconnect.
    fn require_association(&mut self, message: &Map<String, Value>) -> Result<(), Code> {
        let known = self.host.clients();
        let still_known = |key: &str| known.iter().any(|c| c.key == key);
        if self.associated.as_deref().is_some_and(still_known) {
            return Ok(());
        }
        let offered = message
            .get("keys")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
            .map(|offer| str_of(offer, "key"))
            .find(|key| still_known(key));
        match offered {
            Some(key) => {
                self.associated = Some(key.to_string());
                Ok(())
            }
            None => {
                self.associated = None;
                Err(Code::AssociationFailed)
            }
        }
    }
}

fn params(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
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
