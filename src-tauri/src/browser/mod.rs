//! The browser extension host.
//!
//! Rowel speaks the KeePassXC-Browser protocol, so the stock KeePassXC-Browser
//! extension fills logins from a Rowel vault with no extension code of our
//! own. The browser launches this binary as the extension's native messaging
//! host (`proxy`), which relays stdio to the running app over a local socket
//! (`server`); the app answers each request from the open vault (`actions`)
//! inside the protocol's sealed envelope (`protocol`). Browsers find the host
//! through the manifests `manifest` writes when the user turns this on.
//!
//! What the extension can reach is what the user could copy: the logins for
//! the site it is on, a TOTP code, a generated password, and the lock — and
//! it can save the login the user just typed into a page. It can also ask for
//! a passkey to be created or used (`passkeys`), which the user answers one
//! ceremony at a time; the private key never leaves the app. An extension gets
//! in once per vault, by the user naming it in a dialog; after that its
//! identification key, kept inside that vault, is what it proves itself with
//! — to that vault, and to no other workspace on the device.

pub mod manifest;
pub mod proxy;
pub mod server;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

// The protocol's half is the broker crate's, so the app, a headless broker and
// a server link the same code; it is reached here under the paths it had.
pub use rowel_broker::socket::{root_dir, socket_name, IDENTIFIER, SOCKET_FILE, SUN_PATH_MAX};
pub use rowel_broker::{actions, frame, passkeys, protocol};

pub use self::actions::Client;
use self::actions::{ClientKind, Hello, Host, Login, VisibleRef};
use self::passkeys::{Assertion, Registration};
use self::protocol::Code;
use crate::agents::AgentState;
use crate::crypto::PayloadCipher;
use crate::error::Result;
use crate::models::{EntryMetaDto, GeneratorOptions, Passkey};
use crate::passkey::store::{PasskeyVault, SessionVault, Stored};
use crate::passkey::{Ceremony, UserConsent};
use crate::session::Epoch;
use crate::state::AppState;
use crate::store::{SqliteStore, VaultStore};
use crate::{agents, commands, events, session, settings, storage, window, workspace};
use rowel_broker::audit::Event;
use rowel_broker::grant::{
    Approval, CandidateItem, Grant, GrantReply, GrantRequest, ItemSelector, Mode, Probe, State,
    VisibleItem,
};
use rowel_broker::lockbox::{save_failed, save_login_in};
use rowel_broker::policy::{self, defaults, AgentSelector, Decision, Floors, Limits};
use rowel_broker::Signal;

/// How long an `associate` or a passkey ceremony waits for the user's answer
/// before it is refused.
pub const CONSENT_TIMEOUT: Duration = Duration::from_secs(60);

/// Why the Safari extension's socket cannot be had. None of it touches the
/// data-directory socket Chrome and Firefox use.
#[derive(Debug, PartialEq, Eq)]
pub enum SafariSocketError {
    /// A debug build pointed at another data directory (`ROWEL_DB_DIR`, for
    /// tests and throwaway vaults): the group container is the real user's,
    /// and a run like that has no business listening in it.
    Overridden,
    /// No home directory to find the group container under.
    NoHome,
    /// The socket's path does not fit a Unix socket address.
    TooLong { path: PathBuf, bytes: usize },
}

impl std::fmt::Display for SafariSocketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Overridden => write!(f, "ROWEL_DB_DIR is set"),
            Self::NoHome => write!(f, "no home directory"),
            Self::TooLong { path, bytes } => write!(
                f,
                "the socket path {} is {bytes} bytes, over the {SUN_PATH_MAX} a Unix socket allows",
                path.display()
            ),
        }
    }
}

/// The directory of the Safari extension's socket, macOS only: the App Group
/// container ([`group_socket_dir`]), the one place the sandboxed extension can
/// connect to. A second listener, beside the data-directory one
/// (`server::start`), so that nothing about it can keep Chrome and Firefox
/// from connecting.
pub fn safari_socket_dir() -> std::result::Result<PathBuf, SafariSocketError> {
    if cfg!(debug_assertions) && std::env::var_os("ROWEL_DB_DIR").is_some() {
        return Err(SafariSocketError::Overridden);
    }
    let home = dirs::home_dir().ok_or(SafariSocketError::NoHome)?;
    group_socket_dir(&home, cfg!(debug_assertions))
}

/// The Safari socket's directory for `home`'s user, `debug` or not: the group
/// container, or its `dev` subdirectory for a debug build — where the
/// extension looks for it (`HostConnection.socketPath`). Refused when the
/// socket's path in it would not fit a Unix socket address: with the group
/// id this long, a short name over 29 characters (25 for debug) is enough.
pub fn group_socket_dir(
    home: &Path,
    debug: bool,
) -> std::result::Result<PathBuf, SafariSocketError> {
    let container = group_container(home);
    let dir = if debug {
        container.join("dev")
    } else {
        container
    };
    let path = dir.join(SOCKET_FILE);
    let bytes = path.as_os_str().len() + 1;
    if bytes > SUN_PATH_MAX {
        return Err(SafariSocketError::TooLong { path, bytes });
    }
    Ok(dir)
}

/// The macOS App Group container for `home`'s user,
/// `~/Library/Group Containers/<DESKTOP_APP_GROUP>`: where
/// `containerURL(forSecurityApplicationGroupIdentifier:)` points the Safari
/// extension. Spelled out rather than asked of Foundation because the app is
/// not sandboxed and may run unsigned (`tauri dev`): it does not hold the
/// entitlement the API would look up, and does not need it to make and use
/// the directory. The app creates it when it binds (`server::bind`).
pub fn group_container(home: &Path) -> PathBuf {
    home.join("Library")
        .join("Group Containers")
        .join(rowel_core::app::DESKTOP_APP_GROUP)
}

// --- consent -------------------------------------------------------------------
//
// One security decision at a time: an `associate` or a passkey ceremony
// waits on the user in the one slot, and a second ask of either kind while
// one is up is refused, not queued, so two connections cannot put two dialogs
// on screen. The connection thread parks on the receiving end; the frontend's
// answer arrives through `respond` / `respond_passkey`, from the command its
// dialog invokes. An ask is held under a tag — an id of its own, whichever
// kind — and an answer names the tag it is for and the kind it answers: a
// dialog left up past the ask it was drawn for (Rust gives up after a
// minute, the webview does the same on its own clock) cannot answer the next
// ask with a yes the user gave while looking at another, not even the same
// extension's retry. The slot is the one record of what is up — an ask says
// what it is for, so a change of the switches can refuse the grant ask up
// through the slot and leave another kind alone (`refuse_grant_ask`).

/// What an ask is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AskKind {
    Associate,
    Passkey,
    /// A grant ask, and whether its yes takes a verified person first — the
    /// biometric prompt or the master password (`agent_grant_respond`).
    Grant {
        step_up: bool,
    },
}

/// An ask waiting on the user: its tag, its number — by which the asker tells
/// its own slot from a later ask's — what it is for, and the way back to it.
struct Ask<T> {
    serial: u64,
    tag: String,
    kind: AskKind,
    sender: SyncSender<T>,
}

struct Pending<T> {
    slot: Mutex<Option<Ask<T>>>,
    serial: AtomicU64,
}

/// Why an ask did not go up: another holds the slot, or its own `prepare`
/// refused it (a grant ask whose access moved since it was decided).
#[derive(Debug)]
enum NotAsked<E> {
    Busy,
    Refused(E),
}

impl<T> Pending<T> {
    const fn new() -> Self {
        Self {
            slot: Mutex::new(None),
            serial: AtomicU64::new(0),
        }
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Ask<T>>> {
        self.slot.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Put the question (`emit`) to the user under `tag` and block for the
    /// answer, up to [`CONSENT_TIMEOUT`]. A second question while one is up is
    /// refused rather than queued: `None`, as no answer is.
    fn ask(&self, tag: &str, kind: AskKind, emit: impl FnOnce()) -> Option<T> {
        self.try_ask_with(tag, kind, || {
            emit();
            Ok::<(), std::convert::Infallible>(())
        })
        .ok()
        .flatten()
    }

    /// [`Pending::ask`] with the question put up by `prepare`, under the slot
    /// lock, and refusable there: an `Err` from it gives the slot back with
    /// nothing on screen. What that buys a grant ask is atomicity with the
    /// switches: `prepare` checks the access epoch, and a change of the
    /// switches refuses the ask through this same lock (`refuse_grant`), so
    /// the change lands either before the check — no dialog — or after the
    /// ask is up — refused, and its dialog told to close after it was told
    /// to open. A slot another ask holds is `Busy`, told apart from an ask
    /// nobody answered: a grant request answers the first with "ask again
    /// shortly" and the second with a refusal. The wait is outside the lock,
    /// as ever.
    fn try_ask_with<E>(
        &self,
        tag: &str,
        kind: AskKind,
        prepare: impl FnOnce() -> std::result::Result<(), E>,
    ) -> std::result::Result<Option<T>, NotAsked<E>> {
        let (serial, receiver) = self.begin(tag, kind, prepare)?;
        let answer = receiver.recv_timeout(CONSENT_TIMEOUT).ok();
        self.end(serial);
        Ok(answer)
    }

    /// Take the slot for an ask under `tag`, if `prepare` lets it go up: its
    /// number, and the end its answer arrives on. `prepare` runs under the
    /// slot lock (see [`Pending::try_ask_with`]).
    fn begin<E>(
        &self,
        tag: &str,
        kind: AskKind,
        prepare: impl FnOnce() -> std::result::Result<(), E>,
    ) -> std::result::Result<(u64, Receiver<T>), NotAsked<E>> {
        let mut slot = self.slot();
        if slot.is_some() {
            return Err(NotAsked::Busy);
        }
        prepare().map_err(NotAsked::Refused)?;
        let (sender, receiver) = sync_channel(1);
        let serial = self.serial.fetch_add(1, Ordering::Relaxed);
        *slot = Some(Ask {
            serial,
            tag: tag.to_string(),
            kind,
            sender,
        });
        Ok((serial, receiver))
    }

    /// Whether the ask up under `tag` is a grant ask whose yes takes a
    /// verified person first.
    fn needs_step_up(&self, tag: &str) -> bool {
        self.slot()
            .as_ref()
            .is_some_and(|ask| ask.tag == tag && ask.kind == AskKind::Grant { step_up: true })
    }

    /// Give the slot back after ask `serial` — if it is still that ask's. An
    /// answer empties the slot itself, and the next ask may have taken it in
    /// the meantime; that one is waiting on its own answer and is not ours to
    /// clear.
    fn end(&self, serial: u64) {
        let mut slot = self.slot();
        if slot.as_ref().is_some_and(|ask| ask.serial == serial) {
            slot.take();
        }
    }

    /// The answer to the question up under `tag`. Returns whether that one was
    /// waiting for it; an answer for another tag, or for no ask, does nothing.
    fn answer(&self, tag: &str, value: T) -> bool {
        let mut slot = self.slot();
        if slot.as_ref().is_none_or(|ask| ask.tag != tag) {
            return false;
        }
        let ask = slot.take().expect("checked above");
        ask.sender.send(value).is_ok()
    }
}

impl Pending<Answer> {
    /// Refuse the grant ask up, if the ask up is one, as a "no" would: its
    /// tag, for its dialog to be told. An `associate` or a passkey ask is
    /// left alone — a change of the switches is nothing to them.
    fn refuse_grant(&self) -> Option<String> {
        let mut slot = self.slot();
        if !matches!(slot.as_ref()?.kind, AskKind::Grant { .. }) {
            return None;
        }
        let ask = slot.take().expect("checked above");
        let _ = ask.sender.send(Answer::Grant(None));
        Some(ask.tag)
    }
}

/// What the user answered, in the terms of the ask it is for. An answer of
/// the other kind — a passkey dialog somehow answering an `associate` — is no
/// answer at all.
enum Answer {
    /// The name given to an extension, or `None` for a refusal.
    Name(Option<String>),
    /// The account picked for a passkey ceremony, or `None` for a refusal.
    Account(Option<usize>),
    /// The item an agent may use, and for how long, or `None` for a refusal.
    Grant(Option<GrantAnswer>),
}

static CONSENT: Pending<Answer> = Pending::new();

/// What the user answered a grant ask with: which of the candidates the
/// dialog listed — the agent may be given that one and no other — and, for
/// "allow for N days", how many; `None` is this once.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GrantAnswer {
    pub item_id: String,
    pub standing_days: Option<u32>,
}

/// How a step-up ask proves the person before the answer counts: the OS
/// biometric prompt where the device gates a secure store with one, the
/// master password everywhere else (Linux, a Mac without Touch ID).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepUp {
    Biometric,
    Password,
}

impl StepUp {
    pub fn current() -> Self {
        if crate::secure_store::is_supported() && crate::biometrics::is_available() {
            Self::Biometric
        } else {
            Self::Password
        }
    }

    /// The word the ask carries to the dialog.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Biometric => "biometric",
            Self::Password => "password",
        }
    }
}

/// Whether the grant ask up under `id` takes a verified person before its
/// yes counts, so the answering command (`agent_grant_respond`) runs the
/// biometric prompt or checks the password first: a dialog cannot say yes
/// to a step-up ask with a bare click. Read off the slot, the one record of
/// what is up.
pub fn grant_ask_needs_step_up(id: &str) -> bool {
    CONSENT.needs_step_up(id)
}

/// The user's answer to the grant ask up under `id`, or `None` for a
/// refusal. Returns whether that ask was waiting for it.
pub fn respond_grant(id: &str, answer: Option<GrantAnswer>) -> bool {
    CONSENT.answer(id, Answer::Grant(answer))
}

/// Refuse the grant ask up, if there is one — the switches changed
/// (`agents::change_access`), or the session ended (`agents::clear`) — and
/// name the id its dialog is up under, for the dialog to be told it is over.
pub fn refuse_grant_ask() -> Option<String> {
    CONSENT.refuse_grant()
}

/// Ask the user whether the client holding `key`, which says it is `who`, may
/// connect: the name they gave it, or `None`. Blocks the calling thread (see
/// [`Pending::ask`]). The ask is tagged with an id of its own rather than
/// with the key: the same client asks again after a timeout, under the same
/// key, and a dialog left up from the first ask must not be the yes to the
/// second.
pub fn ask(app: &AppHandle, key: &str, who: &Hello) -> Option<String> {
    let id = crate::crypto::random_hex_id();
    let answer = CONSENT.ask(&id, AskKind::Associate, || {
        events::browser_associate(app, &id, key, who.kind, who.platform.as_deref());
        window::raise(app);
    });
    match answer {
        Some(Answer::Name(name)) => name,
        _ => None,
    }
}

/// The user's answer to the ask up under `id`: the name they gave the
/// extension, or `None` for a refusal. Returns whether that ask was waiting.
pub fn respond(id: &str, name: Option<String>) -> bool {
    CONSENT.answer(id, Answer::Name(name))
}

/// Ask the user whether the page at `origin`, through the client
/// `requested_by`, may have `ceremony`: the account they picked for a sign-in
/// (any `Some` for a registration), or `None` for a no — which no answer is.
/// Blocks the calling thread (see [`Pending::ask`]). The ask is tagged with
/// an id of its own, which the dialog's answer names.
pub fn ask_passkey(
    app: &AppHandle,
    origin: &str,
    ceremony: Ceremony<'_>,
    requested_by: &Client,
) -> Option<usize> {
    let id = crate::crypto::random_hex_id();
    let answer = CONSENT.ask(&id, AskKind::Passkey, || {
        events::browser_passkey(app, &id, origin, ceremony, requested_by);
        window::raise(app);
    });
    match answer {
        Some(Answer::Account(account)) => account,
        _ => None,
    }
}

/// The user's answer to the passkey ask up under `id`: which account, or
/// `None` for a refusal. Returns whether that ask was waiting for it.
pub fn respond_passkey(id: &str, account: Option<usize>) -> bool {
    CONSENT.answer(id, Answer::Account(account))
}

/// The prompt a ceremony from the extension asks through: the app's dialog,
/// told which page is asking and through which client.
struct AskUser {
    app: AppHandle,
    origin: String,
    client: Client,
}

impl UserConsent for AskUser {
    fn approve(&self, ceremony: Ceremony<'_>) -> Option<usize> {
        ask_passkey(&self.app, &self.origin, ceremony, &self.client)
    }
}

// --- the extensions let in ------------------------------------------------------
//
// A `meta` row of the vault they were let into, sealed with the rest of it.
// In the vault rather than in `settings.json` for two reasons. The key is the
// whole credential — the protocol never asks the extension to sign with the
// private half — so on disk it must be as unreadable as what it opens. And an
// association is to one vault: the extension keys its own copy by the vault
// hash, and a workspace the user never let it into must not answer to it.
// Every read and write happens under the session lock, so two approvals
// cannot each write a list that misses the other's.

const CLIENTS_META: &str = "browser_clients";

pub(crate) fn clients_in(store: &SqliteStore) -> Vec<Client> {
    store
        .meta_get(CLIENTS_META)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn write_clients(store: &SqliteStore, clients: &[Client]) -> Result<()> {
    store
        .meta_set(CLIENTS_META, &serde_json::to_string(clients)?)
        .map_err(session::store_err)
}

/// The clients let into the open vault; none while it is locked.
pub fn clients(app: &AppHandle) -> Vec<Client> {
    let state = app.state::<AppState>();
    let session = state.session.lock().unwrap();
    session.store().map(clients_in).unwrap_or_default()
}

/// Take a client's access to the open vault back, on record: the row names
/// the client as it was, since after the write nothing does.
pub fn forget(app: &AppHandle, key: &str) -> Result<()> {
    let state = app.state::<AppState>();
    let writer = storage::install_id(app);
    let now = agents::now_ms();
    let session = state.session.lock().unwrap();
    let store = session.store()?;
    let before = clients_in(store);
    let clients: Vec<Client> = before.iter().filter(|c| c.key != key).cloned().collect();
    write_clients(store, &clients)?;
    if clients.len() != before.len() {
        agents::record(
            store,
            &writer,
            agents::draft(Event::RevokedClient, key, &before),
            now,
        );
    }
    Ok(())
}

/// A known client proved itself again: its `last_seen_at_ms`, one write per
/// connection. A write that fails loses nothing but the timestamp.
fn mark_seen(store: &SqliteStore, key: &str, now: i64) {
    let mut clients = clients_in(store);
    let Some(client) = clients.iter_mut().find(|c| c.key == key) else {
        return;
    };
    client.last_seen_at_ms = Some(now);
    if let Err(e) = write_clients(store, &clients) {
        log::warn!("browser host: could not note the client was seen: {e}");
    }
}

// --- passkeys ------------------------------------------------------------------

/// The open vault's passkeys, reached one operation at a time. The session
/// lock is taken for each read or write and let go in between, so the user is
/// asked — in the middle of the ceremony — with it let go, as `associate`
/// asks; and every operation is pinned to the session the ceremony began in,
/// so a lock or a workspace switch while the user decides fails the write
/// rather than landing it in another vault.
struct OpenVault {
    app: AppHandle,
    epoch: Epoch,
}

impl OpenVault {
    fn with<T>(&self, op: impl FnOnce(&SessionVault<'_>) -> Result<T>) -> Result<T> {
        let state = self.app.state::<AppState>();
        let session = state.session.lock().unwrap();
        let store = session.store_at(self.epoch)?;
        let cipher = session.payload_cipher()?;
        op(&SessionVault::new(store, &cipher))
    }
}

impl PasskeyVault for OpenVault {
    fn find(&self, rp_id: &str) -> Result<Vec<Stored>> {
        self.with(|vault| vault.find(rp_id))
    }
    fn insert(&self, passkey: &Passkey) -> Result<()> {
        self.with(|vault| vault.insert(passkey))
    }
    fn update(&self, entry_id: &str, passkey: &Passkey) -> Result<()> {
        self.with(|vault| vault.update(entry_id, passkey))
    }
}

// --- the app as a host ------------------------------------------------------

/// The open vault, as [`Host`] sees it.
pub struct AppHost(pub AppHandle);

impl Host for AppHost {
    fn enabled(&self) -> bool {
        settings::current(&self.0).browser.enabled
    }

    fn database_hash(&self) -> Option<String> {
        let state = self.0.state::<AppState>();
        let unlocked = state.session.lock().unwrap().is_unlocked();
        if !unlocked {
            return None;
        }
        // The workspace, not the vault's sync id: every vault has one from
        // its first open, and the extension's association follows it.
        let id = crate::workspace::active_id(&self.0);
        Some(hex::encode(Sha256::digest(id.as_bytes())))
    }

    fn clients(&self) -> Vec<Client> {
        clients(&self.0)
    }

    // The user is asked with the session lock let go — they may take a
    // minute — so the vault that asked is remembered by its epoch, and the
    // key is written only if that is still the vault open when they answer.
    // A workspace switch in between makes the approval nobody's: the vault
    // it was given for is closed, and the one now open was never asked for.
    // An agent is refused at the door while the Agents switch is off, with
    // no dialog: the switch is where the user says whether agents exist for
    // this app at all, and a dialog would be asking them again.
    fn associate(&self, key: &str, who: &Hello) -> std::result::Result<Client, Code> {
        let app = &self.0;
        if who.kind != ClientKind::Extension && !settings::current(app).browser.agents.enabled {
            return Err(Code::AgentsDisabled);
        }
        let state = app.state::<AppState>();
        let epoch = commands::unlocked_epoch(&state).map_err(|_| Code::DatabaseNotOpened)?;
        let name = ask(app, key, who).ok_or(Code::ActionCancelledOrDenied)?;
        let writer = storage::install_id(app);
        let now = agents::now_ms();
        let session = state.session.lock().unwrap();
        let store = session.store_at(epoch).map_err(|_| {
            log::warn!("browser host: the vault changed hands while the user was asked");
            Code::AssociationFailed
        })?;
        let mut clients = clients_in(store);
        // A key already in — the client lost its own record of the answer
        // and asked again — is the record it has, not a second one.
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
        };
        clients.push(client.clone());
        write_clients(store, &clients).map_err(|e| {
            log::warn!("browser host: could not remember the client: {e}");
            Code::AssociationFailed
        })?;
        agents::record(
            store,
            &writer,
            agents::draft(Event::Associated, key, &clients),
            now,
        );
        // Settings › Browser & agents, if it is open, lists it now.
        events::browser_clients(app);
        Ok(client)
    }

    fn client_seen(&self, key: &str) {
        let state = self.0.state::<AppState>();
        let now = agents::now_ms();
        let session = state.session.lock().unwrap();
        if let Ok(store) = session.store() {
            mark_seen(store, key, now);
        }
    }

    fn logins_for(&self, page_url: &str, host: &str) -> Vec<Login> {
        let state = self.0.state::<AppState>();
        let session = state.session.lock().unwrap();
        let (Ok(cipher), Ok(store)) = (session.payload_cipher(), session.store()) else {
            return Vec::new();
        };
        logins_in(store, &cipher, page_url, host)
    }

    fn totp(&self, id: &str) -> Option<String> {
        let state = self.0.state::<AppState>();
        let session = state.session.lock().unwrap();
        let cipher = session.payload_cipher().ok()?;
        let record = session.store().ok()?.get(id).ok()??;
        let entry = cipher.unseal(&record.id, &record.payload).ok()?;
        let otp = entry.otp.as_deref().filter(|otp| !otp.is_empty())?;
        commands::generator::generate_otp(otp.to_string())
            .ok()
            .map(|otp| otp.code)
    }

    fn generate_password(&self) -> Option<String> {
        let defaults = settings::current(&self.0).generator;
        commands::generator::generate_password(GeneratorOptions {
            length: defaults.length,
            numbers: defaults.numbers,
            symbols: defaults.symbols,
            uppercase: defaults.uppercase,
            lowercase: Some(defaults.lowercase),
            exclude: Some(defaults.exclude),
            exclude_similar_characters: Some(defaults.exclude_similar_characters),
            strict: Some(true),
        })
        .ok()
    }

    fn save_login(
        &self,
        id: Option<&str>,
        url: &str,
        host: &str,
        username: &str,
        password: &str,
    ) -> std::result::Result<(), Code> {
        let state = self.0.state::<AppState>();
        let (epoch, entries) = {
            let session = state.session.lock().unwrap();
            let cipher = session.payload_cipher().map_err(save_failed)?;
            let store = session.store().map_err(save_failed)?;
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            save_login_in(store, &cipher, id, url, host, username, password, &now)?;
            // The row is in. A list that will not read back is not a save
            // that failed: the extension is told the truth about the write,
            // and the frontend goes without this one refresh.
            let entries = session::list_metas(store)
                .map_err(|e| log::warn!("browser host: {e}"))
                .ok();
            (session.epoch(), entries)
        };
        self.written(epoch, entries);
        Ok(())
    }

    fn passkey_register(
        &self,
        client: &Client,
        registration: Registration,
    ) -> std::result::Result<Value, Code> {
        let vault = self.open_vault()?;
        let epoch = vault.epoch;
        let origin = registration.origin.clone();
        let consent = self.ask_user(&origin, client);
        let outcome = passkeys::register(vault, consent, registration);
        self.passkey_row(epoch, client, &origin, &outcome);
        let credential = outcome?;
        // The passkey is on a login — a new one, or the one for the site.
        let entries = {
            let state = self.0.state::<AppState>();
            let session = state.session.lock().unwrap();
            let listed = session.store_at(epoch).and_then(session::list_metas);
            listed.map_err(|e| log::warn!("browser host: {e}")).ok()
        };
        self.written(epoch, entries);
        Ok(credential)
    }

    fn passkey_get(
        &self,
        client: &Client,
        assertion: Assertion,
    ) -> std::result::Result<Value, Code> {
        let vault = self.open_vault()?;
        let epoch = vault.epoch;
        let origin = assertion.origin.clone();
        let consent = self.ask_user(&origin, client);
        let outcome = passkeys::assert(vault, consent, assertion);
        self.passkey_row(epoch, client, &origin, &outcome);
        outcome
    }

    fn lock(&self) {
        session::lock(&self.0);
    }

    fn unlock_requested(&self) {
        window::raise(&self.0);
    }

    // A fill in agent mode, as `associate` is asked: policy and the store
    // under the session lock, the lock let go for the minute the user may
    // take, and the lockbox's epoch checked again before anything the answer
    // decides is written — a lock or a lockbox switch in between makes the
    // yes nobody's.
    fn request_grant(
        &self,
        client: &Client,
        req: GrantRequest,
    ) -> std::result::Result<GrantReply, Code> {
        let app = &self.0;
        // The switches are the session's to say (`AgentState::access`): read
        // from the files by the first request of the session, consulted
        // under the locks by every release below.
        agents::initialise(app);
        let lockbox = workspace::active_id(app);
        let writer = storage::install_id(app);
        let asking = match self.decide_grant(&lockbox, &writer, client, req)? {
            Prepared::Answered(reply) => return Ok(reply),
            Prepared::Ask(asking) => *asking,
        };

        let ask_id = crate::crypto::random_hex_id();
        let step_up = asking.grant.approval == Approval::StepUp;
        let verify = step_up.then(StepUp::current);
        // The dialog names the site, not the page's path and query string.
        let site = agents::site_origin(&asking.origin);
        let mode = asking.grant.modes.first().copied().unwrap_or(Mode::Fill);
        let default_standing_days = settings::current(app).browser.agents.default_standing_days;
        let state = app.state::<AppState>();
        let asked = CONSENT.try_ask_with(&ask_id, AskKind::Grant { step_up }, || {
            // Under the slot, before the dialog goes up: a switch that moved
            // since the decision means no dialog, and one that moves from
            // here on finds the ask in the slot and refuses it there.
            state
                .agents
                .lock()
                .unwrap()
                .access_since(asking.access_epoch)?;
            events::agent_grant(
                app,
                events::AgentGrantAsk {
                    id: &ask_id,
                    client_name: &asking.client_name,
                    client_kind: client.kind,
                    origin: &site,
                    mode: mode.as_str(),
                    reason: asking.grant.reason.as_deref(),
                    // Session end is the grant's, whoever asked: a fill
                    // through the extension in agent mode ends the login it
                    // enabled too, so that dialog warns the same.
                    ends_session: asking.grant.modes.contains(&Mode::Fill),
                    step_up,
                    verify: verify.map(StepUp::as_str),
                    candidates: asking
                        .candidates
                        .iter()
                        .map(|c| c.visible.clone())
                        .collect(),
                    default_standing_days,
                },
            );
            window::raise(app);
            Ok(())
        });
        let answer = match asked {
            // Another decision is on screen. Not a refusal: the client asks
            // again once that dialog has closed.
            Err(NotAsked::Busy) => {
                return Ok(GrantReply {
                    grant_id: asking.grant.id.clone(),
                    state: State::Pending,
                    expires_at_ms: None,
                    retry_after_ms: Some(1500),
                    uses: 0,
                    items: Vec::new(),
                })
            }
            Err(NotAsked::Refused(reason)) => {
                return self.deny_unasked(&writer, client, &asking, reason)
            }
            Ok(Some(Answer::Grant(answer))) => Verdict::Answered(answer),
            Ok(_) => Verdict::Unanswered,
        };
        self.settle_grant(&writer, client, asking, answer)
    }

    fn wait_grant(
        &self,
        client: &Client,
        id: &str,
        _timeout_ms: u64,
    ) -> std::result::Result<GrantReply, Code> {
        self.grant_status(client, id)
    }

    // The state as the log says it: what is due to expire does so first, with
    // its row and its signal, so the reply never says `active` of a grant the
    // next use would find expired.
    fn grant_status(&self, client: &Client, id: &str) -> std::result::Result<GrantReply, Code> {
        let app = &self.0;
        let writer = storage::install_id(app);
        let state = app.state::<AppState>();
        let now = agents::now_ms();
        let (reply, flipped) = {
            let session = state.session.lock().unwrap();
            let store = session.store().map_err(|_| Code::DatabaseNotOpened)?;
            let mut held = state.agents.lock().unwrap();
            let flipped = agents::expire_due(Some(store), &mut held, &writer, now);
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
            events::agents_changed(app);
            agents::rearm(app);
            server::notify_signals();
        }
        Ok(reply)
    }

    fn end_grant(&self, client: &Client, id: &str) -> std::result::Result<(), Code> {
        match agents::end_grant_by(&self.0, id, Some(&client.key), r#"{"reason":"agent"}"#) {
            Ok(true) => Ok(()),
            Ok(false) => Err(Code::GrantDenied),
            Err(_) => Err(Code::DatabaseNotOpened),
        }
    }

    // Only the open lockbox has anything to show: one is open at a time in
    // Tier 1, and the others are sealed. A request naming another lockbox
    // is answered with nothing rather than with the open one's items under
    // the wrong name.
    fn visible_items(&self, client: &Client, lockbox: Option<&str>) -> Vec<VisibleRef> {
        let app = &self.0;
        let state = app.state::<AppState>();
        let now = agents::now_ms();
        let open = workspace::active_id(app);
        if lockbox.is_some_and(|named| named != open) {
            return Vec::new();
        }
        let lockbox = open;
        let session = state.session.lock().unwrap();
        let Ok(store) = session.store() else {
            return Vec::new();
        };
        let held = state.agents.lock().unwrap();
        visible_in(store, &held, client, &lockbox, now)
    }

    fn take_signals(&self, client: &Client) -> Vec<Signal> {
        let state = self.0.state::<AppState>();
        let mut held = state.agents.lock().unwrap();
        held.signals.remove(&client.key).unwrap_or_default()
    }

    // A predicate with a side effect, on purpose: a yes here is the login or
    // its code about to be served, so it is a use of the grant — counted on
    // it and written as a `used` row — and this is the one place the grant is
    // in hand to say so. The targets are not checked again: the TOTP step
    // comes with no URL, and the grant was narrowed to its page when it was
    // issued.
    fn grant_covers(&self, client: &Client, grant_id: &str, item_id: &str, mode: Mode) -> bool {
        let app = &self.0;
        let writer = storage::install_id(app);
        let state = app.state::<AppState>();
        let now = agents::now_ms();
        let session = state.session.lock().unwrap();
        let Ok(store) = session.store() else {
            return false;
        };
        let mut held = state.agents.lock().unwrap();
        covered(
            Locked {
                store,
                held: &mut held,
                writer: &writer,
                now,
            },
            &client.key,
            grant_id,
            item_id,
            mode,
        )
    }

    // The page showed what was filled: the grant ends now, whatever was left
    // of its window, the log says why, and every extension signs out of the
    // site. Whoever reports it: the exposure is a fact about the page, not a
    // claim on the grant.
    fn report_exposure(&self, _client: &Client, grant_id: &str) {
        let app = &self.0;
        let writer = storage::install_id(app);
        let state = app.state::<AppState>();
        let now = agents::now_ms();
        {
            let session = state.session.lock().unwrap();
            let mut held = state.agents.lock().unwrap();
            let Some(grant) = held.find_mut(grant_id) else {
                log::debug!("browser host: exposure reported for an unknown grant {grant_id}");
                return;
            };
            if matches!(grant.state, State::Active | State::Pending) {
                grant.state = State::Revoked;
            }
            let grant = grant.clone();
            agents::ended(
                session.store().ok(),
                &mut held,
                &writer,
                &grant,
                Event::Exposed,
                r#"{"reason":"exposure"}"#,
                now,
            );
        }
        events::agents_changed(app);
        agents::rearm(app);
        server::notify_signals();
    }
}

impl AppHost {
    fn open_vault(&self) -> std::result::Result<OpenVault, Code> {
        let state = self.0.state::<AppState>();
        let epoch = commands::unlocked_epoch(&state).map_err(|_| Code::DatabaseNotOpened)?;
        Ok(OpenVault {
            app: self.0.clone(),
            epoch,
        })
    }

    fn ask_user(&self, origin: &str, client: &Client) -> AskUser {
        AskUser {
            app: self.0.clone(),
            origin: origin.to_string(),
            client: client.clone(),
        }
    }

    // What a passkey ceremony leaves on the log: a `used` row for one that
    // went through — a sign-in or a registration is a use of the site's
    // passkey, in the passkey mode, approved by the user at the dialog — and
    // a `denied` row for one the user refused. No grant object is involved:
    // the dialog is the grant. Written in the session the ceremony began in,
    // and nowhere if that session is gone; any other failure of the ceremony
    // is the page's business, not the log's.
    fn passkey_row(
        &self,
        epoch: Epoch,
        client: &Client,
        origin: &str,
        outcome: &std::result::Result<Value, Code>,
    ) {
        let event = match outcome {
            Ok(_) => Event::Used,
            Err(Code::PasskeyRequestCanceled) => Event::Denied,
            Err(_) => return,
        };
        let writer = storage::install_id(&self.0);
        let now = agents::now_ms();
        let state = self.0.state::<AppState>();
        let session = state.session.lock().unwrap();
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
        agents::record(store, &writer, row, now);
    }

    // Everything a grant request decides before anyone is asked, under the
    // session and agents locks (`decide`): a grant already held, the rules,
    // the rate, policy — and the grant itself when policy allows without
    // asking. Whatever was served, the list and the timer follow, and the
    // extensions are woken for a fill grant an agent now holds.
    fn decide_grant(
        &self,
        lockbox: &str,
        writer: &str,
        client: &Client,
        req: GrantRequest,
    ) -> std::result::Result<Prepared, Code> {
        let app = &self.0;
        let state = app.state::<AppState>();
        let now = agents::now_ms();
        let session = state.session.lock().unwrap();
        let store = session.store().map_err(|_| Code::DatabaseNotOpened)?;
        let epoch = session.epoch();
        let mut held = state.agents.lock().unwrap();
        let prepared = decide(
            Locked {
                store,
                held: &mut held,
                writer,
                now,
            },
            epoch,
            lockbox,
            client,
            req,
        )?;
        drop(held);
        drop(session);
        if matches!(prepared, Prepared::Answered(_)) {
            agents::rearm(app);
            events::agents_changed(app);
            if client.kind != ClientKind::Extension {
                server::notify_signals();
            }
        }
        Ok(prepared)
    }

    // A grant decided but never asked about: the switches moved between the
    // decision and the dialog. No `requested` row — nothing reached the
    // screen — just the refusal, in the session that decided it.
    fn deny_unasked(
        &self,
        writer: &str,
        client: &Client,
        asking: &Asking,
        reason: &str,
    ) -> std::result::Result<GrantReply, Code> {
        let state = self.0.state::<AppState>();
        let now = agents::now_ms();
        let session = state.session.lock().unwrap();
        if let Ok(store) = session.store_at(asking.epoch) {
            let clients = clients_in(store);
            let mut grant = asking.grant.clone();
            grant.state = State::Denied;
            agents::record(
                store,
                writer,
                agents::about(
                    agents::draft(Event::Denied, &client.key, &clients),
                    &grant,
                    Some(&format!(r#"{{"reason":"{reason}"}}"#)),
                ),
                now,
            );
        }
        Err(Code::GrantDenied)
    }

    // The user has answered (or not): under the session lock again, and only
    // if it is still the session that asked, the grant is settled (`settle`)
    // — active for the one item picked, or denied.
    fn settle_grant(
        &self,
        writer: &str,
        client: &Client,
        asking: Asking,
        verdict: Verdict,
    ) -> std::result::Result<GrantReply, Code> {
        let app = &self.0;
        let state = app.state::<AppState>();
        let now = agents::now_ms();
        let session = state.session.lock().unwrap();
        let store = session.store_at(asking.epoch).map_err(|_| {
            log::warn!("browser host: the vault changed hands while the user decided on a grant");
            Code::GrantDenied
        })?;
        let mut held = state.agents.lock().unwrap();
        let reply = settle(
            Locked {
                store,
                held: &mut held,
                writer,
                now,
            },
            client,
            asking,
            verdict,
        )?;
        drop(held);
        drop(session);
        agents::rearm(app);
        events::agents_changed(app);
        if client.kind != ClientKind::Extension {
            server::notify_signals();
        }
        Ok(reply)
    }

    /// A write from the extension is in, in the session `epoch` names.
    fn written(&self, epoch: Epoch, entries: Option<Vec<EntryMetaDto>>) {
        let state = self.0.state::<AppState>();
        // The list the frontend shows, refreshed the way a sync merge
        // refreshes it — for the session that wrote it. A lock or a workspace
        // switch that landed since would otherwise be shown this vault's rows
        // over its own. A list that did not read back just goes without it.
        if let Some(entries) = entries {
            if commands::same_session(&state, epoch).is_ok() {
                events::vault_merged(&self.0, entries);
            }
        }
        // And the change on its way to the user's other devices, as a save
        // from the editor schedules it.
        commands::sync::request_run_if_ready(&self.0, &state);
    }
}

// --- grant requests, between the lock and the ask ---------------------------------

/// What a grant is decided, settled or checked against, under the session
/// and agents locks: the open store, what the agents lock guards — the
/// grants held and the switches as the session holds them — and whose log
/// the rows go to. The caller holds both locks for the whole of it, which is
/// what makes the check of the switches and the release one critical
/// section; nothing of the app is in it, so the tests run the same code over
/// a store of their own.
struct Locked<'a> {
    store: &'a SqliteStore,
    held: &'a mut AgentState,
    writer: &'a str,
    now: i64,
}

/// Everything a grant request decides before anyone is asked: a grant this
/// client already holds for the page (served), the rules, the rate, policy,
/// and the grant itself when policy allows without asking (served). Both
/// releases come after one look at the switches under the same locks.
fn decide(
    ctx: Locked<'_>,
    epoch: Epoch,
    lockbox: &str,
    client: &Client,
    req: GrantRequest,
) -> std::result::Result<Prepared, Code> {
    let Locked {
        store,
        held,
        writer,
        now,
    } = ctx;
    let client_key = client.key.as_str();
    let clients = clients_in(store);
    let client_name = agents::client_label(&clients, client_key);
    let row = |event: Event| agents::draft(event, client_key, &clients);
    // A refusal before there is a grant to be about: the page and the
    // mode are still worth the row.
    let refusal = |event: Event, detail: String| {
        let mut draft = row(event);
        draft.target = agents::origin_of(&req.targets).map(agents::audit_target);
        draft.mode = req.modes.first().copied();
        draft.detail = Some(detail);
        draft
    };
    let Some(origin) = agents::origin_of(&req.targets).map(str::to_string) else {
        agents::record(
            store,
            writer,
            refusal(Event::Denied, r#"{"reason":"no_target"}"#.into()),
            now,
        );
        return Err(Code::GrantDenied);
    };
    // The switches as the session holds them, under the locks and before
    // either release below. The app's off is the extension's `AgentsDisabled`
    // and no row; the lockbox's off is a refusal on record.
    match held.access_on() {
        Ok(()) => {}
        Err("lockbox") => {
            agents::record(
                store,
                writer,
                refusal(Event::Denied, r#"{"reason":"lockbox"}"#.into()),
                now,
            );
            return Err(Code::GrantDenied);
        }
        Err(_) => return Err(Code::AgentsDisabled),
    }
    let access_epoch = held.access_epoch;

    // What the request could be about. The extension names the page's logins
    // itself — it has them in hand, since it asked for them. An agent names a
    // site and, at most, a selector: the candidates are the lockbox's logins
    // for that site, as the plaintext `url_host` column has them, narrowed by
    // the selector; no payload is opened to decide a grant.
    let candidates = if client.kind == ClientKind::Extension {
        candidates_in(store, &req.items)
    } else {
        site_candidates_in(store, &origin, &req.items)
    };
    let probe = Probe::Origin(&origin);

    // A grant this client already holds for the page is served, not asked
    // for again: a username-first sign-in asks for the page's logins
    // twice, and the second time is the same yes. An agent's is signalled
    // again: the page it is for may be in a tab opened since.
    let reusable = held.grants.iter_mut().find(|g| {
        g.client_key == client_key
            && g.is_active(now)
            && req.modes.iter().all(|m| g.modes.contains(m))
            && g.targets.iter().any(|t| t.matches(&probe))
            && candidates.iter().any(|c| g.items.admits(&c.item))
    });
    if let Some(grant) = reusable {
        grant.use_once(now)?;
        let items = admitted(&candidates, &grant.items);
        agents::record(
            store,
            writer,
            agents::about(row(Event::Used), grant, Some(r#"{"reuse":true}"#)),
            now,
        );
        let grant = grant.clone();
        queue_fill(held, &clients, client, &grant, &origin, &items);
        return Ok(Prepared::Answered(reply_for(&grant, items)));
    }

    // The baseline every lockbox with the switch on starts with, then its
    // own rules — the ones the user wrote by answering "for N days".
    let mut rules = defaults::personal();
    rules.extend(agents::live_rules(store, now));

    if held
        .rate(client_key)
        .check(&Limits::default(), now)
        .is_err()
    {
        agents::record(
            store,
            writer,
            refusal(Event::RateLimited, r#"{"reason":"rate"}"#.into()),
            now,
        );
        return Err(Code::RateLimited);
    }

    let items: Vec<CandidateItem> = candidates.iter().map(|c| c.item.clone()).collect();
    let decision = policy::evaluate(&rules, &req, &items, &[probe], &Floors::default());
    let (terms, rule_id) = match &decision {
        Decision::Denied(reason) => {
            let reason = serde_json::to_string(reason).unwrap_or_default();
            agents::record(
                store,
                writer,
                refusal(Event::Denied, format!(r#"{{"reason":{reason}}}"#)),
                now,
            );
            held.rate(client_key).record_denial(now);
            return Err(Code::GrantDenied);
        }
        Decision::Allow { rule_id, .. } => {
            (decision.terms().ok_or(Code::GrantDenied)?, rule_id.clone())
        }
    };
    let mut grant = Grant::from_request(lockbox, req, terms, now);
    // A grant is never wider than the rule that allowed it: what the
    // winning rule admits among the page's logins is what may be served,
    // and what the dialog offers.
    if let Some(rule) = rule_id.and_then(|id| rules.into_iter().find(|r| r.id == id)) {
        grant.items = ItemSelector::Ids(
            candidates
                .iter()
                .filter(|c| rule.items.admits(&c.item))
                .map(|c| c.item.id.clone())
                .collect(),
        );
    }
    let candidates: Vec<Candidate> = candidates
        .into_iter()
        .filter(|c| grant.items.admits(&c.item))
        .collect();

    match grant.approval {
        // `requested` is written here only for a request that is decided
        // here; an ask writes its own once the dialog has really gone up,
        // so a slot that was busy leaves no row behind.
        Approval::Auto => {
            agents::record(
                store,
                writer,
                agents::about(row(Event::Requested), &grant, None),
                now,
            );
            grant.state = State::Active;
            grant.approver = Some("policy".into());
            grant.use_once(now)?;
            agents::record(
                store,
                writer,
                agents::about(row(Event::Granted), &grant, None),
                now,
            );
            agents::record(
                store,
                writer,
                agents::about(row(Event::Used), &grant, None),
                now,
            );
            let items: Vec<VisibleItem> = candidates.into_iter().map(|c| c.visible).collect();
            queue_fill(held, &clients, client, &grant, &origin, &items);
            let reply = reply_for(&grant, items);
            held.grants.push(grant);
            Ok(Prepared::Answered(reply))
        }
        Approval::Ask | Approval::StepUp => {
            held.rate(client_key).record_ask(now);
            Ok(Prepared::Ask(Box::new(Asking {
                epoch,
                access_epoch,
                grant,
                candidates,
                client_name,
                origin,
            })))
        }
    }
}

/// The user has answered (or not): the grant becomes active for the one
/// item picked — a standing answer writes its rule first — or denied. The
/// switches come first, under the same locks as the release, and the epoch
/// the request was decided under: a yes given while a switch moved is no
/// yes, and the row names the switch.
fn settle(
    ctx: Locked<'_>,
    client: &Client,
    asking: Asking,
    verdict: Verdict,
) -> std::result::Result<GrantReply, Code> {
    let Locked {
        store,
        held,
        writer,
        now,
    } = ctx;
    let Asking {
        access_epoch,
        mut grant,
        candidates,
        origin,
        ..
    } = asking;
    let client_key = client.key.as_str();
    let clients = clients_in(store);
    let row = |event: Event| agents::draft(event, client_key, &clients);

    // The ask went up and came down: the request is on record first,
    // whatever the answer was, as it stood when the user saw it.
    agents::record(
        store,
        writer,
        agents::about(row(Event::Requested), &grant, None),
        now,
    );

    let settled = held
        .access_since(access_epoch)
        .and_then(|()| outcome(&verdict, &candidates));
    let (item, standing_days) = match settled {
        Ok((picked, standing_days)) => (picked.visible.clone(), standing_days),
        Err(reason) => {
            grant.state = State::Denied;
            agents::record(
                store,
                writer,
                agents::about(
                    row(Event::Denied),
                    &grant,
                    Some(&format!(r#"{{"reason":"{reason}"}}"#)),
                ),
                now,
            );
            held.rate(client_key).record_denial(now);
            return Err(Code::GrantDenied);
        }
    };

    grant.items = ItemSelector::Ids(vec![item.id.clone()]);
    grant.state = State::Active;
    grant.approver = Some("user".into());
    if let Some(days) = standing_days {
        let days = days.clamp(1, settings::MAX_STANDING_DAYS);
        // For the site, not the page: the rule outlives this visit.
        let rule = defaults::standing(
            client_key,
            &item.id,
            &agents::site_origin(&origin),
            days,
            now,
        );
        let mut rules = agents::live_rules(store, now);
        let rule_id = rule.id.clone();
        rules.push(rule);
        match agents::write_rules(store, &rules) {
            Ok(()) => {
                let mut added = agents::about(
                    row(Event::RuleAdded),
                    &grant,
                    Some(&format!(r#"{{"ruleId":"{rule_id}","days":{days}}}"#)),
                );
                added.grant_id = None;
                added.approval = Some(Approval::Auto);
                agents::record(store, writer, added, now);
            }
            // The fill the user said yes to still goes ahead; only the
            // memory of it is missing, and the next visit asks again.
            Err(e) => log::warn!("browser host: could not keep the standing rule: {e}"),
        }
    }
    grant.use_once(now)?;
    agents::record(
        store,
        writer,
        agents::about(row(Event::Granted), &grant, None),
        now,
    );
    agents::record(
        store,
        writer,
        agents::about(row(Event::Used), &grant, None),
        now,
    );
    held.rate(client_key).record_grant(now);
    let items = vec![item];
    queue_fill(held, &clients, client, &grant, &origin, &items);
    let reply = reply_for(&grant, items);
    held.grants.push(grant);
    Ok(reply)
}

/// Whether a grant of `client`'s is one the extensions are told about: an
/// agent's fill grant. The extension's own grants need no telling — it holds
/// the reply — and nothing but a fill is the extension's to perform.
fn signals_fill(client: &Client, grant: &Grant) -> bool {
    client.kind != ClientKind::Extension && grant.modes.contains(&Mode::Fill)
}

/// An agent's fill grant is active: every extension of the lockbox is told,
/// since the host cannot tell which browser profile has the page open
/// (`Grant::served_to` settles that at the first fill). The item is the one
/// the grant admits — the dialog's pick, or the standing rule's one item.
fn queue_fill(
    held: &mut AgentState,
    clients: &[Client],
    client: &Client,
    grant: &Grant,
    origin: &str,
    items: &[VisibleItem],
) {
    let Some(item) = items.first() else {
        return;
    };
    if !signals_fill(client, grant) {
        return;
    }
    held.signal_extensions(
        clients,
        &Signal::FillGrant {
            grant_id: grant.id.clone(),
            origin: agents::site_origin(origin),
            item: item.clone(),
            auto_submit: false,
            expires_at_ms: grant.expires_at_ms,
        },
    );
}

/// Whether grant `grant_id` is active and covers `item_id` in `mode` right
/// now, for the client holding `client_key` — a predicate with a side
/// effect, on purpose: a yes here is the login or its code about to be
/// served, so it is a use of the grant, counted on it and written as a
/// `used` row, and this is the one place the grant is in hand to say so. The
/// switches come first, under the same locks: a grant still on the list is
/// not served once the session's switches say no. The targets are not
/// checked again: the TOTP step comes with no URL, and the grant was
/// narrowed to its page when it was issued. A grant another client requested
/// — an agent's, signalled to every extension — is claimed by the first
/// extension served under it (`Grant::served_to`) and refused to any other.
fn covered(ctx: Locked<'_>, client_key: &str, grant_id: &str, item_id: &str, mode: Mode) -> bool {
    let Locked {
        store,
        held,
        writer,
        now,
    } = ctx;
    if held.access_on().is_err() {
        return false;
    }
    let Some(grant) = held.find_mut(grant_id) else {
        return false;
    };
    if !grant.is_active(now) || !grant.modes.contains(&mode) {
        return false;
    }
    if grant.client_key != client_key {
        match &grant.served_to {
            None => grant.served_to = Some(client_key.to_string()),
            Some(served) if served == client_key => {}
            Some(_) => return false,
        }
    }
    // The item as the grant's selector sees it. Every grant issued here
    // names ids, which the id alone answers; a wider selector reads the
    // row's kind and tags, nothing sealed.
    let item = match &grant.items {
        ItemSelector::Ids(_) => CandidateItem {
            id: item_id.to_string(),
            kind: String::new(),
            tags: Vec::new(),
        },
        _ => match session::meta_dto_of(store, item_id) {
            Ok(meta) => CandidateItem {
                id: meta.id,
                kind: meta.kind,
                tags: meta.tags,
            },
            Err(_) => return false,
        },
    };
    if !grant.items.admits(&item) || grant.use_once(now).is_err() {
        return false;
    }
    let clients = clients_in(store);
    let mut row = agents::about(
        agents::draft(Event::Used, &grant.client_key, &clients),
        grant,
        Some(r#"{"via":"grantId"}"#),
    );
    row.item_id = Some(item_id.to_string());
    row.mode = Some(mode);
    agents::record(store, writer, row, now);
    true
}

/// What a grant request came to before anyone was asked: answered already
/// (served from a grant held, allowed outright by a rule), or waiting on the
/// user.
enum Prepared {
    Answered(GrantReply),
    // Boxed: an ask carries the whole grant and its candidates, a reply a
    // fraction of that.
    Ask(Box<Asking>),
}

/// A grant policy let through to the dialog, as it waits: the session it was
/// decided in, the grant as it stands, and what the dialog was shown.
struct Asking {
    epoch: Epoch,
    /// The switches' epoch it was decided under (`AgentState::access_epoch`):
    /// a change since, whatever it left the switches at, settles it as a no.
    access_epoch: u64,
    grant: Grant,
    candidates: Vec<Candidate>,
    client_name: String,
    origin: String,
}

/// How the ask came down: with the user's answer — a pick, or a refusal — or
/// without one, which is a minute gone by (or a dialog of another kind
/// answering by mistake).
enum Verdict {
    Answered(Option<GrantAnswer>),
    Unanswered,
}

/// What the answer comes to: the candidate the user picked and for how many
/// days, or the reason the `denied` row names.
fn outcome<'c>(
    verdict: &Verdict,
    candidates: &'c [Candidate],
) -> std::result::Result<(&'c Candidate, Option<u32>), &'static str> {
    match verdict {
        Verdict::Answered(Some(answer)) => candidates
            .iter()
            .find(|c| c.item.id == answer.item_id)
            .map(|c| (c, answer.standing_days))
            .ok_or("item"),
        Verdict::Answered(None) => Err("user"),
        Verdict::Unanswered => Err("timeout"),
    }
}

/// One of the page's logins, as policy sees it and as the dialog shows it:
/// nothing sealed in either.
#[derive(Clone)]
struct Candidate {
    item: CandidateItem,
    visible: VisibleItem,
}

/// The items `selector` names among the open vault's rows, from the list's
/// metadata alone — kind, tags, title and username are plaintext columns;
/// no payload is opened to decide a grant.
fn candidates_in(store: &SqliteStore, selector: &ItemSelector) -> Vec<Candidate> {
    let metas = match store.list() {
        Ok(metas) => metas,
        Err(e) => {
            log::warn!("browser host: could not list the vault: {e}");
            return Vec::new();
        }
    };
    metas
        .into_iter()
        .filter_map(|meta| candidate_of(meta, selector))
        .collect()
}

/// The lockbox's logins for the site at `origin`, as an agent's request names
/// them: the rows whose plaintext `url_host` is the origin's host or a parent
/// of it ([`actions::host_matches`], the same first pass a fill makes), then
/// what `selector` keeps of those. The column only narrows: what is served
/// under the grant is still decided by the sealed website at fill time
/// (`logins_in`). An origin with no host names nothing.
fn site_candidates_in(
    store: &SqliteStore,
    origin: &str,
    selector: &ItemSelector,
) -> Vec<Candidate> {
    let Some(host) = actions::site_host(origin) else {
        return Vec::new();
    };
    let metas = match store.list() {
        Ok(metas) => metas,
        Err(e) => {
            log::warn!("browser host: could not list the vault: {e}");
            return Vec::new();
        }
    };
    metas
        .into_iter()
        .filter(|meta| meta.kind == "login" && actions::host_matches(&host, &meta.url_host))
        .filter_map(|meta| candidate_of(meta, selector))
        .collect()
}

/// `meta` as a candidate, if `selector` admits it.
fn candidate_of(meta: rowel_core::store::EntryMeta, selector: &ItemSelector) -> Option<Candidate> {
    let item = CandidateItem {
        id: meta.id.clone(),
        kind: meta.kind.clone(),
        tags: serde_json::from_str(&meta.tags).unwrap_or_default(),
    };
    selector.admits(&item).then(|| Candidate {
        visible: VisibleItem {
            id: meta.id,
            title: meta.title,
            username: meta.username.filter(|u| !u.is_empty()),
        },
        item,
    })
}

fn admitted(candidates: &[Candidate], selector: &ItemSelector) -> Vec<VisibleItem> {
    candidates
        .iter()
        .filter(|c| selector.admits(&c.item))
        .map(|c| c.visible.clone())
        .collect()
}

fn reply_for(grant: &Grant, items: Vec<VisibleItem>) -> GrantReply {
    GrantReply {
        grant_id: grant.id.clone(),
        state: grant.state,
        expires_at_ms: Some(grant.expires_at_ms),
        retry_after_ms: None,
        uses: grant.uses,
        items,
    }
}

/// The items `client` may ask for by name, in `lockbox`: what its own live
/// rules admit — the ones that name its key, never the baseline, which names
/// every client and would make the whole lockbox visible to anyone let in —
/// and what its own active grants admit. From the list's metadata alone, like
/// every other decision about a grant: no payload is opened to say what
/// exists.
fn visible_in(
    store: &SqliteStore,
    held: &AgentState,
    client: &Client,
    lockbox: &str,
    now: i64,
) -> Vec<VisibleRef> {
    let rules = agents::live_rules(store, now);
    let selectors: Vec<&ItemSelector> = rules
        .iter()
        .filter(|rule| matches!(&rule.agents, AgentSelector::Ids(ids) if ids.contains(&client.key)))
        .map(|rule| &rule.items)
        .chain(
            held.grants
                .iter()
                .filter(|g| g.client_key == client.key && g.is_active(now))
                .map(|g| &g.items),
        )
        .collect();
    if selectors.is_empty() {
        return Vec::new();
    }
    let metas = match store.list() {
        Ok(metas) => metas,
        Err(e) => {
            log::warn!("browser host: could not list the vault: {e}");
            return Vec::new();
        }
    };
    metas
        .into_iter()
        .filter(|meta| {
            let item = CandidateItem {
                id: meta.id.clone(),
                kind: meta.kind.clone(),
                tags: serde_json::from_str(&meta.tags).unwrap_or_default(),
            };
            selectors.iter().any(|selector| selector.admits(&item))
        })
        .map(|meta| VisibleRef {
            reference: format!("rowel://{lockbox}/{}", meta.id),
            title: meta.title,
            fields: fields_for(&meta.kind)
                .iter()
                .map(|f| f.to_string())
                .collect(),
            kind: meta.kind,
        })
        .collect()
}

/// The fields of an item of `kind` a grant could cover, by the names the
/// agent protocol gives them. The kinds are the store's `kind` column, which
/// is the frontend's `EntryType`: `ssh`, not `sshkey`. A kind with nothing
/// an agent could be served — a note, an identity — has none.
fn fields_for(kind: &str) -> &'static [&'static str] {
    match kind {
        "login" => &["password", "otp"],
        "apikey" => &["apiKey"],
        "ssh" => &["privateKey"],
        "card" => &["number", "cvc"],
        "env" => &["body"],
        _ => &[],
    }
}

/// Every login in `store` for the page at `page_url` (whose host is `host`),
/// secrets unsealed.
///
/// Two passes. The plaintext `url_host` column picks the candidates cheaply,
/// without unsealing a payload per row; but it is not authenticated — anyone
/// holding the SQLCipher key could re-point a row at another site — so it
/// only narrows. What decides is the website inside the sealed payload, which
/// each candidate is checked against once it is open, by the full rule
/// ([`actions::site_matches`]: host, then scheme and port). A login with no
/// website is held to the passkey rpId its column was derived from, exactly
/// as `migrate::build_record` derives it.
pub(crate) fn logins_in(
    store: &SqliteStore,
    cipher: &PayloadCipher,
    page_url: &str,
    host: &str,
) -> Vec<Login> {
    let metas = match store.list() {
        Ok(metas) => metas,
        Err(e) => {
            log::warn!("browser host: could not list the vault: {e}");
            return Vec::new();
        }
    };
    let mut logins = Vec::new();
    for meta in metas
        .iter()
        .filter(|m| m.kind == "login" && actions::host_matches(host, &m.url_host))
    {
        let Ok(Some(record)) = store.get(&meta.id) else {
            continue;
        };
        let Ok(entry) = cipher.unseal(&record.id, &record.payload) else {
            log::warn!("browser host: entry {} does not open", meta.id);
            continue;
        };
        let saved = entry
            .website
            .as_deref()
            .filter(|website| !website.trim().is_empty())
            .or_else(|| {
                entry
                    .passkeys
                    .as_deref()
                    .and_then(|keys| keys.first())
                    .map(|key| key.rp_id.as_str())
            })
            .unwrap_or_default();
        if !actions::site_matches(page_url, saved) {
            continue;
        }
        let username = entry.login_name().unwrap_or_default().to_string();
        let totp = entry
            .otp
            .as_deref()
            .filter(|otp| !otp.is_empty())
            .and_then(|otp| commands::generator::generate_otp(otp.to_string()).ok())
            .map(|otp| otp.code);
        logins.push(Login {
            id: entry.id.clone(),
            title: entry.title.clone(),
            username,
            password: entry.password.clone().unwrap_or_default(),
            totp,
        });
    }
    logins
}
