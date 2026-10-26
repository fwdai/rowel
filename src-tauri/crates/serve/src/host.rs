//! The headless broker as a `Host`: the app's `AppHost` method by method,
//! with the broker's engine doing the deciding over a lockbox this process
//! opened itself, and the asks going to a queue a terminal answers rather
//! than to a dialog.
//!
//! What the host holds: the session (the open lockbox, or nothing), the
//! agents' state the engine runs over, the approvals queue, and the files
//! it watches — the config and the lockbox registry. Lock order, everywhere:
//! session → agents → approvals, and the settings cell innermost, as the
//! app reads its settings innermost; never agents → session. An ask blocks
//! with no lock held.
//!
//! The switches: the lockbox's `agents` flag comes from the registry, read
//! into the agents' state by the first request of a session and re-read
//! when `workspaces.json` changes, so `rowel admin agents off` cuts a
//! running broker off and revokes what it should; the app's Agents switch
//! is implied on — this process *is* the agents broker. The epoch rule is
//! the app's: a seal moves the session counter and resets the agents'
//! state, so a grant decided in one session settles in no other.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use rowel_broker::actions::{Client, ClientKind, Hello, Host, Login, VisibleRef};
use rowel_broker::agents::{
    self, candidates_in, change_under_locks, client_covered, covered, decide, deny_unasked,
    end_grant_in, expire_due, now_ms, reply_for, resolved, settle, target_text, visible_access,
    visible_in, AccessState, AgentState, Asking, GrantEnd, Locked, Prepared, Verdict,
};
use rowel_broker::audit::Event;
use rowel_broker::grant::{
    Approval, GrantReply, GrantRequest, Mode, ResolveContext, State, VisibleItem,
};
use rowel_broker::lockbox::{
    clients_in, logins_in, mark_seen, save_failed, save_login_in, totp_in, write_clients,
};
use rowel_broker::passkeys::{self, Assertion, Registration};
use rowel_broker::protocol::Code;
use rowel_broker::reference::Reference;
use rowel_broker::server;
use rowel_broker::Signal;
use rowel_core::error::Result as CoreResult;
use rowel_core::models::Passkey;
use rowel_core::passkey::store::{PasskeyVault, SessionVault, Stored};
use rowel_core::passkey::{Ceremony, UserConsent};
use rowel_core::store::SqliteStore;
use rowel_core::workspace::{self, Registry};

use crate::approvals::{self, Answer, Approvals, AskKind, PendingAsk};
use crate::config::{self, Config, Watched};
use crate::unlock::{self, Opened};

/// What the process was started with that the config file does not say, or
/// says otherwise: the command line wins.
#[derive(Clone, Debug, Default)]
pub struct Overrides {
    pub ask_timeout_ms: Option<u64>,
    pub idle_lock_ms: Option<u64>,
}

/// The open lockbox, and which session it is: the counter moves at every
/// unlock and every seal, so work prepared against one session can tell, at
/// the moment it writes, that the session is no longer the one in front of
/// it — the app's `Session::epoch`.
struct Session {
    opened: Option<Opened>,
    epoch: u64,
}

impl Session {
    fn store(&self) -> Result<&SqliteStore, Code> {
        self.opened
            .as_ref()
            .map(|o| &o.store)
            .ok_or(Code::DatabaseNotOpened)
    }

    fn store_at(&self, epoch: u64) -> Result<&SqliteStore, Code> {
        if self.epoch != epoch {
            return Err(Code::DatabaseNotOpened);
        }
        self.store()
    }
}

/// The files the host watches, and what it last read from them.
struct Settings {
    config: Config,
    config_file: Watched,
    registry_file: Watched,
    /// The lockbox's `agents` flag as the registry last said it.
    lockbox_on: bool,
    overrides: Overrides,
}

impl Settings {
    fn ask_timeout(&self) -> Duration {
        Duration::from_millis(
            self.overrides
                .ask_timeout_ms
                .unwrap_or(self.config.ask_timeout_ms),
        )
    }

    fn idle_lock(&self) -> Option<Duration> {
        self.overrides
            .idle_lock_ms
            .or(self.config.idle_lock_ms)
            .map(Duration::from_millis)
    }
}

struct Shared {
    root: PathBuf,
    lockbox: agents::Lockbox,
    lockbox_dir: PathBuf,
    writer_id: String,
    session: Mutex<Session>,
    agents: Mutex<AgentState>,
    approvals: Approvals,
    settings: Mutex<Settings>,
    /// The key file to open the lockbox from again after a seal, when the
    /// broker runs in that mode: the next request lifts an idle lock itself.
    key_file: Option<PathBuf>,
    /// When the last request came in, for the idle lock.
    last_used_ms: AtomicI64,
}

/// The headless broker as a `Host`. Cheap to clone: every connection holds
/// one over the same state.
#[derive(Clone)]
pub struct HeadlessHost(Arc<Shared>);

/// What `rowel admin` and `rowel approve` are told about the broker.
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub lockbox: String,
    pub unlocked: bool,
    pub pending: usize,
    pub active_grants: usize,
}

/// An answer to an ask, as `rowel approve` sends it over the control socket.
#[derive(Clone, Debug, Default, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerRequest {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<usize>,
    #[serde(default)]
    pub deny: bool,
    /// The master password, for a step-up ask. Wiped after use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<Zeroizing<String>>,
}

impl HeadlessHost {
    /// A host for the lockbox `lockbox_id` under `root`, sealed, reading its
    /// config from `config_path` and its install id from the root.
    /// `key_file` is the file to open the lockbox from again after a seal,
    /// when there is one.
    pub fn new(
        root: &Path,
        lockbox_id: &str,
        config_path: &Path,
        overrides: Overrides,
        key_file: Option<PathBuf>,
        notice: Option<approvals::Notice>,
    ) -> Result<Self, config::ConfigError> {
        let config = Config::load(config_path)?;
        let registry = Registry::load(root);
        let (name, lockbox_on) = registry
            .workspaces
            .iter()
            .find(|w| w.id == lockbox_id)
            .map(|w| (w.name.clone(), w.agents))
            .unwrap_or((None, false));
        Ok(Self(Arc::new(Shared {
            root: root.to_path_buf(),
            lockbox: agents::Lockbox {
                id: lockbox_id.to_string(),
                name,
            },
            lockbox_dir: workspace::dir_of(root, lockbox_id),
            writer_id: crate::install_id_in(root),
            session: Mutex::new(Session {
                opened: None,
                epoch: 0,
            }),
            agents: Mutex::new(AgentState::default()),
            approvals: notice.map(Approvals::new).unwrap_or_default(),
            settings: Mutex::new(Settings {
                config,
                config_file: Watched::new(config_path.to_path_buf()),
                registry_file: Watched::new(root.join(workspace::REGISTRY_FILE)),
                lockbox_on,
                overrides,
            }),
            key_file,
            last_used_ms: AtomicI64::new(now_ms()),
        })))
    }

    pub fn root(&self) -> &Path {
        &self.0.root
    }

    pub fn lockbox_id(&self) -> &str {
        &self.0.lockbox.id
    }

    /// The config as last read.
    pub fn config(&self) -> Config {
        self.settings().config.clone()
    }

    pub fn approvals(&self) -> &Approvals {
        &self.0.approvals
    }

    // --- the session -------------------------------------------------------------

    fn session(&self) -> MutexGuard<'_, Session> {
        self.0.session.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn agents(&self) -> MutexGuard<'_, AgentState> {
        self.0.agents.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn settings(&self) -> MutexGuard<'_, Settings> {
        self.0.settings.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The lockbox is open: serve it from now on. Every connected extension
    /// is told, as the app tells them.
    pub fn unlock(&self, opened: Opened) {
        {
            let mut session = self.session();
            session.opened = Some(opened);
            session.epoch += 1;
        }
        self.touch();
        server::notify_unlocked();
    }

    /// Seal the lockbox: nothing is served until the next unlock, every grant
    /// goes, the switches are read afresh by the next session's first
    /// request, every ask waiting is refused, and every connected extension
    /// is told. Whether anything was open to seal.
    pub fn seal(&self) -> bool {
        let was_open = {
            let mut session = self.session();
            let was_open = session.opened.take().is_some();
            session.epoch += 1;
            // Under the session lock, as the app resets under
            // `session::sealed`: no request can decide against the old
            // session and settle against the new one.
            self.agents().reset();
            was_open
        };
        self.0.approvals.refuse_all();
        if was_open {
            server::notify_locked();
        }
        was_open
    }

    pub fn is_unlocked(&self) -> bool {
        self.session().opened.is_some()
    }

    /// Open the lockbox from the key file again, when the broker runs in that
    /// mode and the lockbox is sealed: an idle lock in key-file mode is
    /// lifted by the next request, since the material is there to be read.
    fn reopen_if_sealed(&self) {
        let Some(key_file) = &self.0.key_file else {
            return;
        };
        if self.is_unlocked() {
            return;
        }
        match unlock::open(
            &self.0.root,
            &self.0.lockbox.id,
            &unlock::Unlock::KeyFile(key_file.clone()),
        ) {
            Ok(opened) => self.unlock(opened),
            Err(e) => log::warn!("rowel serve: the key file no longer opens the lockbox: {e}"),
        }
    }

    fn touch(&self) {
        self.0.last_used_ms.store(now_ms(), Ordering::Relaxed);
    }

    // --- the files watched ------------------------------------------------------------

    /// Read the config and the registry again if either changed, and act on
    /// a change of the lockbox's flag: the one path the switch moves through
    /// (`change_under_locks`), which bumps the epoch, revokes active grants
    /// when the lockbox is cut off, and — with the locks let go — refuses
    /// the grant asks waiting, as the app closes the dialog.
    pub fn refresh(&self) {
        let flag = {
            let mut settings = self.settings();
            if settings.config_file.changed() {
                match Config::load(settings.config_file.path()) {
                    Ok(config) => {
                        log::info!("rowel serve: config reloaded");
                        settings.config = config;
                    }
                    Err(e) => log::warn!("rowel serve: {e}; keeping the config as it was"),
                }
            }
            if settings.registry_file.changed() {
                let registry = Registry::load(&self.0.root);
                let on = registry
                    .workspaces
                    .iter()
                    .find(|w| w.id == self.0.lockbox.id)
                    .is_some_and(|w| w.agents);
                if on != settings.lockbox_on {
                    settings.lockbox_on = on;
                    Some(on)
                } else {
                    None
                }
            } else {
                None
            }
        };
        let Some(on) = flag else {
            return;
        };
        let changed = {
            let session = self.session();
            let mut held = self.agents();
            let outcome = change_under_locks(
                session.store().ok(),
                &mut held,
                &self.0.writer_id,
                "lockbox",
                now_ms(),
                |access| -> CoreResult<((), bool)> {
                    match access {
                        Some(access) if access.lockbox_on != on => {
                            access.lockbox_on = on;
                            Ok(((), true))
                        }
                        // Not read yet this session: the next request reads
                        // the flag as it stands now.
                        _ => Ok(((), false)),
                    }
                },
            );
            outcome.is_ok_and(|(_, changed)| changed)
        };
        if changed {
            self.0.approvals.refuse_grants();
            server::notify_signals();
        }
    }

    /// The session's first look at the switches, under the agents lock the
    /// caller holds: the lockbox's flag as the registry last said it, and
    /// the Agents switch on — this process is the broker.
    fn initialise(&self, held: &mut AgentState) {
        if held.access.is_some() {
            return;
        }
        let lockbox_on = self.settings().lockbox_on;
        held.initialise(AccessState {
            enabled: true,
            lockbox_on,
        });
    }

    fn ask_timeout(&self) -> Duration {
        self.settings().ask_timeout()
    }

    // --- housekeeping -----------------------------------------------------------------

    /// What a timer does once a second: expire what is due and tell the
    /// extensions, seal an idle lockbox, read the files again. Whether the
    /// idle lock sealed it, for the caller that has to ask for the password
    /// again.
    pub fn tick(&self) -> bool {
        let now = now_ms();
        let changed = {
            let session = self.session();
            let mut held = self.agents();
            let flipped = expire_due(session.store().ok(), &mut held, &self.0.writer_id, now);
            let pruned = held.prune(now);
            flipped || pruned
        };
        if changed {
            server::notify_signals();
        }
        self.refresh();
        let idle = self.settings().idle_lock();
        let idle_sealed = idle.is_some_and(|idle| {
            let last = self.0.last_used_ms.load(Ordering::Relaxed);
            now.saturating_sub(last) >= idle.as_millis() as i64 && self.is_unlocked()
        });
        if idle_sealed {
            log::info!("rowel serve: idle; the lockbox is sealed");
            self.seal();
        }
        idle_sealed
    }

    // --- the control socket's view -------------------------------------------------------

    pub fn pending(&self) -> Vec<PendingAsk> {
        self.0.approvals.pending()
    }

    pub fn status(&self) -> Status {
        let now = now_ms();
        Status {
            lockbox: self.0.lockbox.id.clone(),
            unlocked: self.is_unlocked(),
            pending: self.0.approvals.pending().len(),
            active_grants: self
                .agents()
                .grants
                .iter()
                .filter(|g| g.is_active(now))
                .count(),
        }
    }

    /// Answer the ask `request` names, as `rowel approve` asks to. A step-up
    /// ask's yes takes the master password: it is derived through the
    /// lockbox's own KDF sidecar and compared with the held key's SQLCipher
    /// subkey in constant time, as the app's `prove_password` compares; a
    /// mismatch refuses the ask (`denied {"reason":"user"}`) and says so.
    pub fn answer(&self, request: AnswerRequest) -> Result<(), String> {
        let ask = self
            .0
            .approvals
            .find(&request.id)
            .ok_or_else(|| "no ask is waiting under that id".to_string())?;
        let answer = match &ask.kind {
            AskKind::Associate { hello, .. } => Answer::Pairing((!request.deny).then(|| {
                request
                    .name
                    .clone()
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| approvals::label(hello.platform.as_deref()).to_string())
            })),
            AskKind::Passkey { .. } => {
                Answer::Account((!request.deny).then(|| request.account.unwrap_or(0)))
            }
            AskKind::Grant {
                candidates,
                step_up,
                ..
            } => {
                if request.deny {
                    Answer::Grant(None)
                } else {
                    let item_id = match (&request.item, candidates.as_slice()) {
                        (Some(item), _) => item.clone(),
                        (None, [only]) => only.id.clone(),
                        (None, []) => return Err("the ask names no item to allow".into()),
                        (None, _) => {
                            return Err("several items match: say which with --item <id>".into())
                        }
                    };
                    let password = request.password.as_deref().map(String::as_str);
                    if *step_up && !self.password_matches(password)? {
                        let _ = self.0.approvals.answer(&request.id, Answer::Grant(None));
                        return Err("the password did not match; the request was refused".into());
                    }
                    Answer::Grant(Some(agents::GrantAnswer {
                        item_id,
                        standing_days: request.days,
                    }))
                }
            }
        };
        self.0
            .approvals
            .answer(&request.id, answer)
            .map_err(|e| e.to_string())
    }

    /// Whether `password` is the master password of the open lockbox. The
    /// derivation (Argon2id) runs with no lock held; only the comparison is
    /// made under the session lock.
    fn password_matches(&self, password: Option<&str>) -> Result<bool, String> {
        let password = password.ok_or_else(|| {
            "this ask needs the master password: rowel approve prompts for it".to_string()
        })?;
        let kdf = unlock::kdf_params(&self.0.lockbox_dir).map_err(|e| e.to_string())?;
        let candidate = unlock::derive(password, kdf.as_ref()).map_err(|e| e.to_string())?;
        let session = self.session();
        let opened = session
            .opened
            .as_ref()
            .ok_or_else(|| "the lockbox is sealed".to_string())?;
        let held = opened.key.sqlcipher_key();
        Ok(bool::from(candidate.sqlcipher_key()[..].ct_eq(&held[..])))
    }

    // --- the grant flow's three phases -------------------------------------------------

    /// `check` under the session and agents locks, over the open store, once
    /// the session has read the switches; false while sealed.
    fn with_locked(&self, check: impl FnOnce(Locked<'_>) -> bool) -> bool {
        let now = now_ms();
        let session = self.session();
        let Ok(store) = session.store() else {
            return false;
        };
        let mut held = self.agents();
        self.initialise(&mut held);
        check(Locked {
            store,
            held: &mut held,
            writer: &self.0.writer_id,
            now,
        })
    }

    /// Everything a grant request decides before anyone is asked, under the
    /// session and agents locks, with the config's rules beside the
    /// lockbox's; the session it was decided in comes back beside it.
    fn decide_grant(&self, client: &Client, req: GrantRequest) -> Result<(Prepared, u64), Code> {
        let extra = self.settings().config.rules.clone();
        let now = now_ms();
        let session = self.session();
        let store = session.store()?;
        let epoch = session.epoch;
        let mut held = self.agents();
        self.initialise(&mut held);
        let prepared = decide(
            Locked {
                store,
                held: &mut held,
                writer: &self.0.writer_id,
                now,
            },
            &self.0.lockbox.id,
            client,
            req,
            &extra,
        )?;
        drop(held);
        drop(session);
        if matches!(prepared, Prepared::Answered(_)) && client.kind != ClientKind::Extension {
            server::notify_signals();
        }
        Ok((prepared, epoch))
    }

    /// A grant decided but never asked about: the switches moved between
    /// the decision and the queue. On record in the session that decided
    /// it, and nowhere if that session is gone.
    fn deny_unasked(
        &self,
        client: &Client,
        epoch: u64,
        asking: &Asking,
        reason: &str,
    ) -> Result<GrantReply, Code> {
        let session = self.session();
        if let Ok(store) = session.store_at(epoch) {
            deny_unasked(store, &self.0.writer_id, client, asking, reason, now_ms());
        }
        Err(Code::GrantDenied)
    }

    /// The person has answered (or not): under the locks again, and only if
    /// it is still the session that asked, the grant is settled.
    fn settle_grant(
        &self,
        client: &Client,
        epoch: u64,
        asking: Asking,
        verdict: Verdict,
    ) -> Result<GrantReply, Code> {
        let now = now_ms();
        let session = self.session();
        let store = session.store_at(epoch).map_err(|_| {
            log::warn!("rowel serve: the lockbox was sealed while the ask was waiting");
            Code::GrantDenied
        })?;
        let mut held = self.agents();
        let reply = settle(
            Locked {
                store,
                held: &mut held,
                writer: &self.0.writer_id,
                now,
            },
            client,
            asking,
            verdict,
        )?;
        drop(held);
        drop(session);
        if client.kind != ClientKind::Extension {
            server::notify_signals();
        }
        Ok(reply)
    }

    // --- passkeys ------------------------------------------------------------------

    /// What a passkey ceremony leaves on the log, as the app writes it: a
    /// `used` row for one that went through, `denied` for one refused, in
    /// the session the ceremony began in.
    fn passkey_row(
        &self,
        epoch: u64,
        client: &Client,
        origin: &str,
        outcome: &Result<Value, Code>,
    ) {
        let event = match outcome {
            Ok(_) => Event::Used,
            Err(Code::PasskeyRequestCanceled) => Event::Denied,
            Err(_) => return,
        };
        let session = self.session();
        let Ok(store) = session.store_at(epoch) else {
            return;
        };
        let mut row = agents::draft(event, &client.key, &clients_in(store));
        row.mode = Some(Mode::Passkey);
        row.target = Some(agents::site_origin(origin));
        row.approval = Some(Approval::Ask);
        if event == Event::Used {
            row.approver = Some("user".into());
        } else {
            row.detail = Some(r#"{"reason":"user"}"#.into());
        }
        agents::record(store, &self.0.writer_id, row, now_ms());
    }

    fn open_vault(&self) -> Result<OpenVault, Code> {
        let session = self.session();
        session.store()?;
        Ok(OpenVault {
            host: self.clone(),
            epoch: session.epoch,
        })
    }
}

/// The open lockbox's passkeys, reached one operation at a time under the
/// session lock and pinned to the session the ceremony began in, as the
/// app's `OpenVault` is: a seal while the person decides fails the write.
struct OpenVault {
    host: HeadlessHost,
    epoch: u64,
}

impl OpenVault {
    fn with<T>(&self, op: impl FnOnce(&SessionVault<'_>) -> CoreResult<T>) -> CoreResult<T> {
        let session = self.host.session();
        let opened = session
            .opened
            .as_ref()
            .filter(|_| session.epoch == self.epoch)
            .ok_or(rowel_core::error::Error::Locked)?;
        op(&SessionVault::new(&opened.store, &opened.cipher))
    }
}

impl PasskeyVault for OpenVault {
    fn find(&self, rp_id: &str) -> CoreResult<Vec<Stored>> {
        self.with(|vault| vault.find(rp_id))
    }
    fn insert(&self, passkey: &Passkey) -> CoreResult<()> {
        self.with(|vault| vault.insert(passkey))
    }
    fn update(&self, entry_id: &str, passkey: &Passkey) -> CoreResult<()> {
        self.with(|vault| vault.update(entry_id, passkey))
    }
}

/// The prompt a ceremony asks through: the approvals queue, told which page
/// is asking and through which client. Never auto — the answer is the
/// user-presence gesture the site requires.
struct AskUser {
    host: HeadlessHost,
    origin: String,
    client: Client,
}

impl UserConsent for AskUser {
    fn approve(&self, ceremony: Ceremony<'_>) -> Option<usize> {
        let (text, accounts) = match ceremony {
            Ceremony::Register {
                rp_id, user_name, ..
            } => (
                format!(
                    "create a passkey for {rp_id}{}",
                    user_name.map(|u| format!(" ({u})")).unwrap_or_default()
                ),
                Vec::new(),
            ),
            Ceremony::SignIn { rp_id, accounts } => (
                format!("sign in to {rp_id} with a passkey"),
                accounts.iter().map(|a| a.user_name.clone()).collect(),
            ),
        };
        let kind = AskKind::Passkey {
            origin: self.origin.clone(),
            client_name: self.client.name.clone(),
            ceremony: text,
            accounts,
        };
        let timeout = self.host.ask_timeout();
        match self.host.0.approvals.ask(kind, timeout) {
            Some(Answer::Account(account)) => account,
            _ => None,
        }
    }
}

impl Host for HeadlessHost {
    fn enabled(&self) -> bool {
        true
    }

    // The lockbox id digested, the app's scheme, so an extension's
    // association made against the app follows the lockbox here. Every
    // sealed request begins with this call, which is where an idle lock in
    // key-file mode is lifted.
    fn database_hash(&self) -> Option<String> {
        self.reopen_if_sealed();
        if !self.is_unlocked() {
            return None;
        }
        Some(hex::encode(Sha256::digest(self.0.lockbox.id.as_bytes())))
    }

    fn clients(&self) -> Vec<Client> {
        let session = self.session();
        session.store().map(clients_in).unwrap_or_default()
    }

    // The person is asked with the session lock let go, and the key is
    // written only if the session that asked is still the one open.
    fn associate(&self, key: &str, who: &Hello) -> Result<Client, Code> {
        self.touch();
        let epoch = {
            let session = self.session();
            session.store()?;
            session.epoch
        };
        let kind = AskKind::Associate {
            key: key.to_string(),
            hello: who.clone(),
        };
        let name = match self.0.approvals.ask(kind, self.ask_timeout()) {
            Some(Answer::Pairing(Some(name))) => name,
            _ => return Err(Code::ActionCancelledOrDenied),
        };
        let now = now_ms();
        let session = self.session();
        let store = session.store_at(epoch).map_err(|_| {
            log::warn!("rowel serve: the lockbox was sealed while the association waited");
            Code::AssociationFailed
        })?;
        let mut clients = clients_in(store);
        if let Some(known) = clients.iter().find(|c| c.key == key) {
            return Ok(known.clone());
        }
        let client = Client {
            name,
            key: key.to_string(),
            kind: who.kind,
            platform: who.platform.clone(),
            created_at_ms: now,
            last_seen_at_ms: Some(now),
            agent_id: None,
            driven_by_agent: false,
        };
        clients.push(client.clone());
        write_clients(store, &clients).map_err(|e| {
            log::warn!("rowel serve: could not remember the client: {e}");
            Code::AssociationFailed
        })?;
        agents::record(
            store,
            &self.0.writer_id,
            agents::draft(Event::Associated, key, &clients),
            now,
        );
        Ok(client)
    }

    fn client_seen(&self, key: &str) {
        self.touch();
        let session = self.session();
        if let Ok(store) = session.store() {
            mark_seen(store, key, now_ms());
        }
    }

    fn logins_for(&self, page_url: &str, host: &str) -> Vec<Login> {
        self.touch();
        let session = self.session();
        match &session.opened {
            Some(opened) => logins_in(&opened.store, &opened.cipher, page_url, host),
            None => Vec::new(),
        }
    }

    fn totp(&self, id: &str) -> Option<String> {
        self.touch();
        let session = self.session();
        let opened = session.opened.as_ref()?;
        totp_in(&opened.store, &opened.cipher, id)
    }

    // The app's generator settings are the app's; a headless broker has
    // none to offer, and the extension handles the refusal.
    fn generate_password(&self) -> Option<String> {
        None
    }

    fn save_login(
        &self,
        id: Option<&str>,
        url: &str,
        host: &str,
        username: &str,
        password: &str,
    ) -> Result<(), Code> {
        self.touch();
        let session = self.session();
        let opened = session
            .opened
            .as_ref()
            .ok_or_else(|| save_failed(rowel_core::error::Error::Locked))?;
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        save_login_in(
            &opened.store,
            &opened.cipher,
            id,
            url,
            host,
            username,
            password,
            &now,
        )
    }

    fn passkey_register(&self, client: &Client, registration: Registration) -> Result<Value, Code> {
        self.touch();
        let vault = self.open_vault()?;
        let epoch = vault.epoch;
        let origin = registration.origin.clone();
        let consent = AskUser {
            host: self.clone(),
            origin: origin.clone(),
            client: client.clone(),
        };
        let outcome = passkeys::register(vault, consent, registration);
        self.passkey_row(epoch, client, &origin, &outcome);
        outcome
    }

    fn passkey_get(&self, client: &Client, assertion: Assertion) -> Result<Value, Code> {
        self.touch();
        let vault = self.open_vault()?;
        let epoch = vault.epoch;
        let origin = assertion.origin.clone();
        let consent = AskUser {
            host: self.clone(),
            origin: origin.clone(),
            client: client.clone(),
        };
        let outcome = passkeys::assert(vault, consent, assertion);
        self.passkey_row(epoch, client, &origin, &outcome);
        outcome
    }

    fn lock(&self) {
        self.seal();
    }

    // There is no window to raise; a sealed broker is unlocked at its
    // terminal, or by its key file at the next request.
    fn unlock_requested(&self) {
        log::debug!("rowel serve: a client asked for the lockbox to be unlocked");
    }

    // The app's three phases: policy under the locks, the ask with them let
    // go, the settle under them again if it is still the session that asked.
    fn request_grant(&self, client: &Client, req: GrantRequest) -> Result<GrantReply, Code> {
        self.touch();
        self.refresh();
        let (prepared, epoch) = self.decide_grant(client, req)?;
        let asking = match prepared {
            Prepared::Answered(reply) => return Ok(reply),
            Prepared::Ask(asking) => *asking,
        };
        let step_up = asking.grant.approval == Approval::StepUp;
        let kind = AskKind::Grant {
            client_name: asking.client_name.clone(),
            client_kind: client.kind,
            mode: asking.grant.modes.first().copied().unwrap_or(Mode::Fill),
            targets: asking.grant.targets.iter().map(target_text).collect(),
            reason: asking.grant.reason.clone(),
            candidates: asking
                .candidates
                .iter()
                .map(|c| c.visible.clone())
                .collect::<Vec<VisibleItem>>(),
            step_up,
        };
        let timeout = self.ask_timeout();
        // Queued under the agents lock, as the app queues under its slot: a
        // switch that moved since the decision means no ask, and one that
        // moves from here on finds the ask in the queue and refuses it
        // there. The ask goes on the client's rate record here and nowhere
        // earlier, because the limit is on asks that reach a person.
        let ticket = {
            let mut held = self.agents();
            if let Err(reason) = held.access_since(asking.access_epoch) {
                drop(held);
                return self.deny_unasked(client, epoch, &asking, reason);
            }
            held.rate(&client.key).record_ask(now_ms());
            self.0.approvals.enqueue(kind, timeout)
        };
        let verdict = match ticket.wait() {
            Some(Answer::Grant(answer)) => Verdict::Answered(answer),
            _ => Verdict::Unanswered,
        };
        self.settle_grant(client, epoch, asking, verdict)
    }

    fn wait_grant(&self, client: &Client, id: &str, _timeout_ms: u64) -> Result<GrantReply, Code> {
        self.grant_status(client, id)
    }

    // What is due to expire does so first, with its row and its signal, so
    // the reply never says `active` of a grant the next use would find
    // expired.
    fn grant_status(&self, client: &Client, id: &str) -> Result<GrantReply, Code> {
        self.touch();
        let now = now_ms();
        let (reply, flipped) = {
            let session = self.session();
            let store = session.store()?;
            let mut held = self.agents();
            self.initialise(&mut held);
            let flipped = expire_due(Some(store), &mut held, &self.0.writer_id, now);
            let grant = held
                .grants
                .iter()
                .find(|g| g.id == id && g.client_key == client.key)
                .ok_or(Code::GrantDenied)?;
            let items = candidates_in(store, &grant.items)
                .into_iter()
                .map(|c| c.visible)
                .collect();
            (reply_for(grant, items), flipped)
        };
        if flipped {
            server::notify_signals();
        }
        Ok(reply)
    }

    fn end_grant(&self, client: &Client, id: &str) -> Result<(), Code> {
        self.touch();
        let ended = {
            let session = self.session();
            let store = session.store()?;
            let mut held = self.agents();
            self.initialise(&mut held);
            let ctx = Locked {
                store,
                held: &mut held,
                writer: &self.0.writer_id,
                now: now_ms(),
            };
            match end_grant_in(ctx, id, Some(&client.key), r#"{"reason":"agent"}"#) {
                GrantEnd::NotFound => return Err(Code::GrantDenied),
                GrantEnd::AlreadyOver => false,
                GrantEnd::Ended => true,
            }
        };
        if ended {
            server::notify_signals();
        }
        Ok(())
    }

    // Only the open lockbox has anything to show; a request naming another
    // is answered with nothing rather than with this one's items under the
    // wrong name.
    fn visible_items(
        &self,
        client: &Client,
        lockbox: Option<&str>,
    ) -> Result<Vec<VisibleRef>, Code> {
        self.touch();
        self.refresh();
        let session = self.session();
        let store = session.store()?;
        let mut held = self.agents();
        self.initialise(&mut held);
        visible_access(&held)?;
        if lockbox.is_some_and(|named| named != self.0.lockbox.id) {
            return Ok(Vec::new());
        }
        visible_in(store, &held, client, &self.0.lockbox.id, now_ms())
    }

    fn take_signals(&self, client: &Client) -> Vec<Signal> {
        let mut held = self.agents();
        self.initialise(&mut held);
        held.signals.remove(&client.key).unwrap_or_default()
    }

    fn grant_covers(&self, client: &Client, grant_id: &str, item_id: &str, mode: Mode) -> bool {
        self.touch();
        self.with_locked(|ctx| covered(ctx, &client.key, grant_id, item_id, mode))
    }

    fn client_grant_covers(&self, client: &Client, item_id: &str, mode: Mode) -> bool {
        self.touch();
        self.with_locked(|ctx| client_covered(ctx, &client.key, item_id, mode))
    }

    // The page showed what was filled: the grant ends now, the log says
    // why, and every extension signs out of the site.
    fn report_exposure(&self, _client: &Client, grant_id: &str) {
        self.touch();
        let now = now_ms();
        {
            let session = self.session();
            let mut held = self.agents();
            let Some(grant) = held.find_mut(grant_id) else {
                log::debug!("rowel serve: exposure reported for an unknown grant {grant_id}");
                return;
            };
            if matches!(grant.state, State::Active | State::Pending) {
                grant.state = State::Revoked;
            }
            let grant = grant.clone();
            agents::ended(
                session.store().ok(),
                &mut held,
                &self.0.writer_id,
                &grant,
                Event::Exposed,
                r#"{"reason":"exposure"}"#,
                now,
            );
        }
        server::notify_signals();
    }

    // The one reply that carries a value to something other than an
    // extension, in the same critical section every other release is made
    // in.
    fn resolve(
        &self,
        client: &Client,
        grant_id: &str,
        reference: &Reference,
        ctx: &ResolveContext,
    ) -> Result<Zeroizing<String>, Code> {
        self.touch();
        self.refresh();
        let now = now_ms();
        let session = self.session();
        let opened = session.opened.as_ref().ok_or(Code::DatabaseNotOpened)?;
        let mut held = self.agents();
        self.initialise(&mut held);
        resolved(
            Locked {
                store: &opened.store,
                held: &mut held,
                writer: &self.0.writer_id,
                now,
            },
            &opened.cipher,
            &self.0.lockbox,
            client,
            grant_id,
            reference,
            ctx,
        )
    }
}
