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

pub mod server;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};
use zeroize::Zeroizing;

// The protocol's half, the grant engine and the lockbox-side functions are
// the broker crate's, so the app, a headless broker and a server link the
// same code; they are reached here under the paths they had.
pub use rowel_broker::agents::{
    client_covered, covered, decide, outcome, resolved, settle, visible_in, Asking, Candidate,
    GrantAnswer, Lockbox, Locked, Prepared, Verdict,
};
pub use rowel_broker::lockbox::{clients_in, env_var, item_by_name, logins_in};
pub use rowel_broker::socket::{root_dir, socket_name, IDENTIFIER, SOCKET_FILE, SUN_PATH_MAX};
pub use rowel_broker::{actions, frame, manifest, passkeys, protocol};

pub use self::actions::Client;
use self::actions::{ClientKind, Hello, Host, Login, VisibleRef};
use self::passkeys::{Assertion, CeremonyLog, Registration};
use self::protocol::Code;
use crate::error::Result;
use crate::models::{EntryMetaDto, GeneratorOptions, Passkey};
use crate::passkey::store::{PasskeyVault, SessionVault, Stored};
use crate::passkey::{Ceremony, UserConsent};
use crate::session::Epoch;
use crate::state::AppState;
use crate::{agents, commands, events, session, settings, storage, window, workspace};
use rowel_broker::agents::{
    candidates_in, deny_unasked, reply_for, report_exposure_in, visible_access, GrantEnd,
};
use rowel_broker::audit::Event;
use rowel_broker::grant::{
    Approval, GrantReply, GrantRequest, Mode, ResolveContext, State, Target,
};
use rowel_broker::lockbox::{mark_seen, save_failed, save_login_in, totp_in, write_clients};
use rowel_broker::policy::defaults;
use rowel_broker::reference::Reference;
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
    /// The name given to a client and whether an agent drives its browser,
    /// or `None` for a refusal.
    Pairing(Option<Pairing>),
    /// The account picked for a passkey ceremony, or `None` for a refusal.
    Account(Option<usize>),
    /// The item an agent may use, and for how long, or `None` for a refusal.
    Grant(Option<GrantAnswer>),
}

static CONSENT: Pending<Answer> = Pending::new();

/// What the user answered a client asking to connect with: the name to
/// remember it by, and — for an extension — whether an AI agent drives the
/// browser it is in ([`Client::driven_by_agent`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pairing {
    pub name: String,
    pub driven_by_agent: bool,
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

/// Another decision is on screen: the ask did not go up, and the caller asks
/// again once that dialog has closed.
#[derive(Debug)]
pub struct Busy;

/// A grant ask a Rowel Server routed to this member (`org::approvals`), put
/// to the person through the one consent slot as the app's own grant asks
/// are: `emit` puts the dialog up under `id` and the window comes forward,
/// and the answer arrives through `agent_grant_respond` like any other — a
/// step-up ask is verified there before its yes counts, since the slot
/// records `step_up`. Blocks the calling thread for the answer, up to the
/// slot's minute; an unanswered ask is told apart from a refusal, because
/// the server decides a timed-out ask for itself and must not be told "no".
pub fn ask_remote_grant(
    app: &AppHandle,
    id: &str,
    step_up: bool,
    emit: impl FnOnce(),
) -> std::result::Result<Verdict, Busy> {
    let asked = CONSENT.try_ask_with(id, AskKind::Grant { step_up }, || {
        emit();
        window::raise(app);
        Ok::<(), std::convert::Infallible>(())
    });
    match asked {
        Err(NotAsked::Busy) => Err(Busy),
        Err(NotAsked::Refused(never)) => match never {},
        Ok(Some(Answer::Grant(answer))) => Ok(Verdict::Answered(answer)),
        Ok(_) => Ok(Verdict::Unanswered),
    }
}

/// Ask the user whether the client holding `key`, which says it is `who`, may
/// connect: their answer, or `None`. Blocks the calling thread (see
/// [`Pending::ask`]). The ask is tagged with an id of its own rather than
/// with the key: the same client asks again after a timeout, under the same
/// key, and a dialog left up from the first ask must not be the yes to the
/// second.
pub fn ask(app: &AppHandle, key: &str, who: &Hello) -> Option<Pairing> {
    let id = crate::crypto::random_hex_id();
    let answer = CONSENT.ask(&id, AskKind::Associate, || {
        events::browser_associate(app, &id, key, who.kind, who.platform.as_deref());
        window::raise(app);
    });
    match answer {
        Some(Answer::Pairing(pairing)) => pairing,
        _ => None,
    }
}

/// The user's answer to the ask up under `id`, or `None` for a refusal.
/// Returns whether that ask was waiting.
pub fn respond(id: &str, pairing: Option<Pairing>) -> bool {
    CONSENT.answer(id, Answer::Pairing(pairing))
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
// A `meta` row of the vault they were let into, sealed with the rest of it
// (`rowel_broker::lockbox`, which says why there and not in `settings.json`).
// Every read and write happens under the session lock, so two approvals
// cannot each write a list that misses the other's.

/// The clients let into the open vault; none while it is locked.
pub fn clients(app: &AppHandle) -> Vec<Client> {
    let state = app.state::<AppState>();
    let session = state.session.lock().unwrap();
    session.store().map(clients_in).unwrap_or_default()
}

/// Mark the extension holding `key` as one in a browser an AI agent drives,
/// or not: every fill it asks for is then a grant
/// ([`Client::driven_by_agent`]). Only an extension carries the mark; an
/// agent never receives a secret to begin with.
pub fn set_driven_by_agent(app: &AppHandle, key: &str, driven: bool) -> Result<()> {
    let state = app.state::<AppState>();
    let session = state.session.lock().unwrap();
    let store = session.store()?;
    let mut clients = clients_in(store);
    let client = clients
        .iter_mut()
        .find(|c| c.key == key && c.kind == ClientKind::Extension)
        .ok_or(crate::error::Error::NotFound)?;
    client.driven_by_agent = driven;
    write_clients(store, &clients)
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
        agents::note(
            store,
            &writer,
            agents::draft(Event::RevokedClient, key, &before),
            now,
        );
    }
    Ok(())
}

// --- passkeys ------------------------------------------------------------------

/// The open vault's passkeys, reached one operation at a time. The session
/// lock is taken for each read or write and let go in between, so the user is
/// asked — in the middle of the ceremony — with it let go, as `associate`
/// asks; and every operation is pinned to the session the ceremony began in,
/// so a lock or a workspace switch while the user decides fails the write
/// rather than landing it in another vault. Every write commits with the
/// ceremony's `used` row ([`CeremonyLog`]).
struct OpenVault {
    app: AppHandle,
    epoch: Epoch,
    log: CeremonyLog,
}

impl OpenVault {
    fn with<T>(&self, op: impl FnOnce(&SessionVault<'_>) -> Result<T>) -> Result<T> {
        let state = self.app.state::<AppState>();
        let session = state.session.lock().unwrap();
        let store = session.store_at(self.epoch)?;
        let cipher = session.payload_cipher()?;
        self.log.vault(store, &cipher, op)
    }

    /// The ceremony's `outcome` as it may go out, and what it leaves on the
    /// log, in the session it began in (`CeremonyLog::settle`).
    fn settle(
        &self,
        outcome: std::result::Result<Value, Code>,
    ) -> std::result::Result<Value, Code> {
        let state = self.app.state::<AppState>();
        let session = state.session.lock().unwrap();
        self.log.settle(session.store_at(self.epoch).ok(), outcome)
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
        let pairing = ask(app, key, who).ok_or(Code::ActionCancelledOrDenied)?;
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
            name: pairing.name,
            key: key.to_string(),
            kind: who.kind,
            platform: who.platform.clone(),
            created_at_ms: now,
            last_seen_at_ms: Some(now),
            agent_id: None,
            driven_by_agent: pairing.driven_by_agent && who.kind == ClientKind::Extension,
        };
        clients.push(client.clone());
        write_clients(store, &clients).map_err(|e| {
            log::warn!("browser host: could not remember the client: {e}");
            Code::AssociationFailed
        })?;
        agents::note(
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
        totp_in(session.store().ok()?, &cipher, id)
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
        let vault = self.open_vault(client, &registration.origin)?;
        let epoch = vault.epoch;
        let consent = self.ask_user(&registration.origin, client);
        let outcome = passkeys::register(&vault, consent, registration);
        let credential = vault.settle(outcome)?;
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
        let vault = self.open_vault(client, &assertion.origin)?;
        let consent = self.ask_user(&assertion.origin, client);
        let outcome = passkeys::assert(&vault, consent, assertion);
        vault.settle(outcome)
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
        let (prepared, epoch) = self.decide_grant(&lockbox, &writer, client, req)?;
        let asking = match prepared {
            Prepared::Answered(reply) => return Ok(reply),
            Prepared::Ask(asking) => *asking,
        };

        let ask_id = crate::crypto::random_hex_id();
        let step_up = asking.grant.approval == Approval::StepUp;
        let verify = step_up.then(StepUp::current);
        let targets: Vec<events::AskTarget> = asking.grant.targets.iter().map(ask_target).collect();
        let mode = asking.grant.modes.first().copied().unwrap_or(Mode::Fill);
        let default_standing_days = settings::current(app).browser.agents.default_standing_days;
        let state = app.state::<AppState>();
        let asked = CONSENT.try_ask_with(&ask_id, AskKind::Grant { step_up }, || {
            // Under the slot, before the dialog goes up: a switch that moved
            // since the decision means no dialog, and one that moves from
            // here on finds the ask in the slot and refuses it there. The
            // ask goes on the client's record here and nowhere earlier,
            // because the limit is on asks that reach a person: a slot
            // another dialog holds never gets this far, so asking again
            // once it closes — which is what "ask again shortly" invites —
            // is not counted against the client.
            {
                let mut held = state.agents.lock().unwrap();
                held.access_since(asking.access_epoch)?;
                held.rate(&client.key).record_ask(agents::now_ms());
            }
            events::agent_grant(
                app,
                events::AgentGrantAsk {
                    id: &ask_id,
                    client_name: &asking.client_name,
                    client_kind: client.kind,
                    targets,
                    mode: mode.as_str(),
                    reason: asking.grant.reason.as_deref(),
                    // Session end is the grant's, whoever asked: a fill
                    // through the extension in agent mode ends the login it
                    // enabled too, so that dialog warns the same. A secret
                    // grant signs nothing in, so nothing out.
                    ends_session: asking.grant.modes.contains(&Mode::Fill),
                    step_up,
                    verify: verify.map(StepUp::as_str),
                    candidates: asking
                        .candidates
                        .iter()
                        .map(|c| c.visible.clone())
                        .collect(),
                    default_standing_days,
                    run_hours: asking.grant.run_id.is_some().then_some(defaults::RUN_HOURS),
                    server: None,
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
                return self.deny_unasked(&writer, client, epoch, &asking, reason)
            }
            Ok(Some(Answer::Grant(answer))) => Verdict::Answered(answer),
            Ok(_) => Verdict::Unanswered,
        };
        self.settle_grant(&writer, client, epoch, asking, answer)
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
        agents::initialise(app);
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
            agents::grants_changed(app);
        }
        Ok(reply)
    }

    fn end_grant(&self, client: &Client, id: &str) -> std::result::Result<(), Code> {
        agents::initialise(&self.0);
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
    fn visible_items(
        &self,
        client: &Client,
        lockbox: Option<&str>,
    ) -> std::result::Result<Vec<VisibleRef>, Code> {
        let app = &self.0;
        agents::initialise(app);
        let state = app.state::<AppState>();
        let now = agents::now_ms();
        let open = workspace::active_id(app);
        let session = state.session.lock().unwrap();
        let store = session.store().map_err(|_| Code::DatabaseNotOpened)?;
        let held = state.agents.lock().unwrap();
        visible_access(&held)?;
        if lockbox.is_some_and(|named| named != open) {
            return Ok(Vec::new());
        }
        visible_in(store, &held, client, &open, now)
    }

    fn take_signals(&self, client: &Client) -> Vec<Signal> {
        agents::initialise(&self.0);
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
    fn grant_covers(
        &self,
        client: &Client,
        grant_id: &str,
        item_id: &str,
        mode: Mode,
    ) -> std::result::Result<bool, Code> {
        self.with_locked(|ctx| covered(ctx, &client.key, grant_id, item_id, mode))
    }

    fn client_grant_covers(
        &self,
        client: &Client,
        item_id: &str,
        mode: Mode,
    ) -> std::result::Result<bool, Code> {
        self.with_locked(|ctx| client_covered(ctx, &client.key, item_id, mode))
    }

    // The page showed what was filled: the grant ends now, whatever was left
    // of its window, the log says why, and every extension signs out of the
    // site — when the reporter is the grant's client or the extension served
    // under it, and once (`report_exposure_in`). With the vault locked there
    // is no grant left to end.
    fn report_exposure(&self, client: &Client, grant_id: &str) {
        let app = &self.0;
        agents::initialise(app);
        let writer = storage::install_id(app);
        let state = app.state::<AppState>();
        let now = agents::now_ms();
        let end = {
            let session = state.session.lock().unwrap();
            let Ok(store) = session.store() else {
                return;
            };
            let mut held = state.agents.lock().unwrap();
            report_exposure_in(
                Locked {
                    store,
                    held: &mut held,
                    writer: &writer,
                    now,
                },
                grant_id,
                &client.key,
            )
        };
        match end {
            GrantEnd::Ended => agents::grants_changed(app),
            GrantEnd::AlreadyOver => {}
            GrantEnd::NotFound => {
                log::debug!("browser host: exposure reported for {grant_id}, not the reporter's")
            }
        }
    }

    // The one reply that carries a value to something other than an
    // extension, made in the same critical section every other release is
    // made in: the switches, the grant, the item and the field are checked
    // and the value unsealed under the session and agents locks, so a switch
    // going off cannot interleave with it. The lockbox's registry name is
    // read first, under `workspace_lock`, which is the outer lock and must
    // not be taken inside the session's.
    fn resolve(
        &self,
        client: &Client,
        grant_id: &str,
        reference: &Reference,
        ctx: &ResolveContext,
    ) -> std::result::Result<Zeroizing<String>, Code> {
        let app = &self.0;
        agents::initialise(app);
        let lockbox = Lockbox {
            id: workspace::active_id(app),
            name: workspace::active_name(app),
        };
        let writer = storage::install_id(app);
        let state = app.state::<AppState>();
        let now = agents::now_ms();
        let session = state.session.lock().unwrap();
        let store = session.store().map_err(|_| Code::DatabaseNotOpened)?;
        let cipher = session
            .payload_cipher()
            .map_err(|_| Code::DatabaseNotOpened)?;
        let mut held = state.agents.lock().unwrap();
        resolved(
            Locked {
                store,
                held: &mut held,
                writer: &writer,
                now,
            },
            &cipher,
            &lockbox,
            client,
            grant_id,
            reference,
            ctx,
        )
    }
}

impl AppHost {
    /// The open vault for a ceremony `client` runs for the page at `origin`.
    fn open_vault(&self, client: &Client, origin: &str) -> std::result::Result<OpenVault, Code> {
        let state = self.0.state::<AppState>();
        let epoch = commands::unlocked_epoch(&state).map_err(|_| Code::DatabaseNotOpened)?;
        Ok(OpenVault {
            app: self.0.clone(),
            epoch,
            log: CeremonyLog::new(&storage::install_id(&self.0), client, origin),
        })
    }

    fn ask_user(&self, origin: &str, client: &Client) -> AskUser {
        AskUser {
            app: self.0.clone(),
            origin: origin.to_string(),
            client: client.clone(),
        }
    }

    // `check` under the session and agents locks, over the open store, once
    // the session has read the switches; false while locked.
    fn with_locked(
        &self,
        check: impl FnOnce(Locked<'_>) -> std::result::Result<bool, Code>,
    ) -> std::result::Result<bool, Code> {
        let app = &self.0;
        agents::initialise(app);
        let writer = storage::install_id(app);
        let state = app.state::<AppState>();
        let now = agents::now_ms();
        let session = state.session.lock().unwrap();
        let Ok(store) = session.store() else {
            return Ok(false);
        };
        let mut held = state.agents.lock().unwrap();
        check(Locked {
            store,
            held: &mut held,
            writer: &writer,
            now,
        })
    }

    // Everything a grant request decides before anyone is asked, under the
    // session and agents locks (`decide`): a grant already held, the rules,
    // the rate, policy — and the grant itself when policy allows without
    // asking. Whatever was served, the list and the timer follow, and the
    // extensions are woken for a fill grant an agent now holds. The session
    // it was decided in comes back beside it: the engine knows no sessions,
    // and an ask settles only in the one that asked.
    fn decide_grant(
        &self,
        lockbox: &str,
        writer: &str,
        client: &Client,
        req: GrantRequest,
    ) -> std::result::Result<(Prepared, Epoch), Code> {
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
            lockbox,
            client,
            req,
            // The app's rules are all in the lockbox; it holds none beside it.
            &[],
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
        Ok((prepared, epoch))
    }

    // A grant decided but never asked about: the switches moved between the
    // decision and the dialog. The refusal goes on record in the session
    // that decided it (`deny_unasked`), and nowhere if that session is gone.
    fn deny_unasked(
        &self,
        writer: &str,
        client: &Client,
        epoch: Epoch,
        asking: &Asking,
        reason: &str,
    ) -> std::result::Result<GrantReply, Code> {
        let state = self.0.state::<AppState>();
        let now = agents::now_ms();
        let session = state.session.lock().unwrap();
        if let Ok(store) = session.store_at(epoch) {
            deny_unasked(store, writer, client, asking, reason, now);
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
        epoch: Epoch,
        asking: Asking,
        verdict: Verdict,
    ) -> std::result::Result<GrantReply, Code> {
        let app = &self.0;
        let state = app.state::<AppState>();
        let now = agents::now_ms();
        let session = state.session.lock().unwrap();
        let store = session.store_at(epoch).map_err(|_| {
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

// --- the grant dialog -------------------------------------------------------------

/// A target as the grant dialog names it: the site of a page, the request
/// pattern or the command line as the agent wrote it.
fn ask_target(target: &Target) -> events::AskTarget {
    match target {
        Target::Origin(origin) => events::AskTarget::Origin {
            origin: agents::site_origin(origin),
        },
        Target::Http {
            host,
            path,
            methods,
        } => events::AskTarget::Http {
            host: host.clone(),
            path: path.clone(),
            methods: methods.clone(),
        },
        Target::Command(line) => events::AskTarget::Command { line: line.clone() },
    }
}
