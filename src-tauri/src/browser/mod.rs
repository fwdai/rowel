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

pub mod actions;
pub mod frame;
pub mod manifest;
pub mod passkeys;
pub mod protocol;
pub mod proxy;
pub mod server;
#[cfg(test)]
mod tests;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Mutex;
use std::time::Duration;

use interprocess::local_socket::{prelude::*, Name};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

pub use self::actions::Client;
use self::actions::{Host, Login};
use self::passkeys::{Assertion, Registration};
use self::protocol::Code;
use crate::crypto::PayloadCipher;
use crate::error::Result;
use crate::models::{Entry, EntryMetaDto, GeneratorOptions, Passkey};
use crate::passkey::store::{PasskeyVault, SessionVault, Stored};
use crate::passkey::{Ceremony, UserConsent};
use crate::session::Epoch;
use crate::settings;
use crate::state::AppState;
use crate::store::{migrate, SqliteStore, VaultStore};
use crate::{commands, events, session, window};

/// The bundle identifier, which is the app-data directory's name on every
/// desktop platform. The proxy has no `AppHandle` to ask, so it is spelled out
/// here; a test holds it to `tauri.conf.json`.
pub const IDENTIFIER: &str = "app.rowel.desktop";
pub const SOCKET_FILE: &str = "browser.sock";

/// How long an `associate` or a passkey ceremony waits for the user's answer
/// before it is refused.
pub const CONSENT_TIMEOUT: Duration = Duration::from_secs(60);

/// The app's data directory, resolved the way `storage::root_dir` resolves it
/// but without Tauri: the debug override, then the platform's app-data
/// directory under the identifier, with a `dev` subdirectory in debug builds.
pub fn root_dir() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        if let Ok(dir) = std::env::var("ROWEL_DB_DIR") {
            return Some(PathBuf::from(dir));
        }
    }
    let dir = dirs::data_dir()?.join(IDENTIFIER);
    Some(if cfg!(debug_assertions) {
        dir.join("dev")
    } else {
        dir
    })
}

/// The socket the app listens on for `root`: a file beside the vault on Unix,
/// and on Windows a named pipe whose name carries a digest of `root`, so a
/// debug build and a release build — or a test — never share one.
pub fn socket_name(root: &Path) -> io::Result<Name<'static>> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::GenericFilePath;
        root.join(SOCKET_FILE).to_fs_name::<GenericFilePath>()
    }
    #[cfg(windows)]
    {
        use interprocess::local_socket::GenericNamespaced;
        let digest = Sha256::digest(root.to_string_lossy().as_bytes());
        format!("{IDENTIFIER}.browser.{}", hex::encode(&digest[..8]))
            .to_ns_name::<GenericNamespaced>()
    }
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
// extension's retry.

/// An ask waiting on the user: its tag, its number — by which the asker tells
/// its own slot from a later ask's — and the way back to it.
struct Ask<T> {
    serial: u64,
    tag: String,
    sender: SyncSender<T>,
}

struct Pending<T> {
    slot: Mutex<Option<Ask<T>>>,
    serial: AtomicU64,
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
    fn ask(&self, tag: &str, emit: impl FnOnce()) -> Option<T> {
        let (serial, receiver) = self.begin(tag)?;
        emit();
        let answer = receiver.recv_timeout(CONSENT_TIMEOUT).ok();
        self.end(serial);
        answer
    }

    /// Take the slot for an ask under `tag`: its number, and the end its answer
    /// arrives on. `None` while another ask is up.
    fn begin(&self, tag: &str) -> Option<(u64, Receiver<T>)> {
        let mut slot = self.slot();
        if slot.is_some() {
            return None;
        }
        let (sender, receiver) = sync_channel(1);
        let serial = self.serial.fetch_add(1, Ordering::Relaxed);
        *slot = Some(Ask {
            serial,
            tag: tag.to_string(),
            sender,
        });
        Some((serial, receiver))
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

/// What the user answered, in the terms of the ask it is for. An answer of
/// the other kind — a passkey dialog somehow answering an `associate` — is no
/// answer at all.
enum Answer {
    /// The name given to an extension, or `None` for a refusal.
    Name(Option<String>),
    /// The account picked for a passkey ceremony, or `None` for a refusal.
    Account(Option<usize>),
}

static CONSENT: Pending<Answer> = Pending::new();

/// Ask the user whether the extension holding `key` may connect: the name
/// they gave it, or `None`. Blocks the calling thread (see [`Pending::ask`]).
/// The ask is tagged with an id of its own rather than with the key: the
/// same extension asks again after a timeout, under the same key, and a
/// dialog left up from the first ask must not be the yes to the second.
pub fn ask(app: &AppHandle, key: &str) -> Option<String> {
    let id = crate::crypto::random_hex_id();
    let answer = CONSENT.ask(&id, || {
        events::browser_associate(app, &id, key);
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

/// Ask the user whether the page at `origin` may have `ceremony`: the account
/// they picked for a sign-in (any `Some` for a registration), or `None` for a
/// no — which no answer is. Blocks the calling thread (see [`Pending::ask`]).
/// The ask is tagged with an id of its own, which the dialog's answer names.
pub fn ask_passkey(app: &AppHandle, origin: &str, ceremony: Ceremony<'_>) -> Option<usize> {
    let id = crate::crypto::random_hex_id();
    let answer = CONSENT.ask(&id, || {
        events::browser_passkey(app, &id, origin, ceremony);
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
/// told which page is asking.
struct AskUser {
    app: AppHandle,
    origin: String,
}

impl UserConsent for AskUser {
    fn approve(&self, ceremony: Ceremony<'_>) -> Option<usize> {
        ask_passkey(&self.app, &self.origin, ceremony)
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

fn clients_in(store: &SqliteStore) -> Vec<Client> {
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

/// The extensions let into the open vault; none while it is locked.
pub fn clients(app: &AppHandle) -> Vec<Client> {
    let state = app.state::<AppState>();
    let session = state.session.lock().unwrap();
    session.store().map(clients_in).unwrap_or_default()
}

/// Take an extension's access to the open vault back.
pub fn forget(app: &AppHandle, key: &str) -> Result<()> {
    let state = app.state::<AppState>();
    let session = state.session.lock().unwrap();
    let store = session.store()?;
    let mut clients = clients_in(store);
    clients.retain(|c| c.key != key);
    write_clients(store, &clients)
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
    fn associate(&self, key: &str) -> std::result::Result<String, Code> {
        let state = self.0.state::<AppState>();
        let epoch = commands::unlocked_epoch(&state).map_err(|_| Code::DatabaseNotOpened)?;
        let name = ask(&self.0, key).ok_or(Code::ActionCancelledOrDenied)?;
        let session = state.session.lock().unwrap();
        let store = session.store_at(epoch).map_err(|_| {
            log::warn!("browser host: the vault changed hands while the user was asked");
            Code::AssociationFailed
        })?;
        let mut clients = clients_in(store);
        if !clients.iter().any(|c| c.key == key) {
            clients.push(Client {
                name: name.clone(),
                key: key.to_string(),
            });
            write_clients(store, &clients).map_err(|e| {
                log::warn!("browser host: could not remember the extension: {e}");
                Code::AssociationFailed
            })?;
            // Settings › Browser extension, if it is open, lists it now.
            events::browser_clients(&self.0);
        }
        Ok(name)
    }

    fn logins_for(&self, host: &str) -> Vec<Login> {
        let state = self.0.state::<AppState>();
        let session = state.session.lock().unwrap();
        let (Ok(cipher), Ok(store)) = (session.payload_cipher(), session.store()) else {
            return Vec::new();
        };
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

    fn passkey_register(&self, registration: Registration) -> std::result::Result<Value, Code> {
        let vault = self.open_vault()?;
        let epoch = vault.epoch;
        let consent = self.ask_user(&registration.origin);
        let credential = passkeys::register(vault, consent, registration)?;
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

    fn passkey_get(&self, assertion: Assertion) -> std::result::Result<Value, Code> {
        let consent = self.ask_user(&assertion.origin);
        passkeys::assert(self.open_vault()?, consent, assertion)
    }

    fn lock(&self) {
        session::lock(&self.0);
    }

    fn unlock_requested(&self) {
        window::raise(&self.0);
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

    fn ask_user(&self, origin: &str) -> AskUser {
        AskUser {
            app: self.0.clone(),
            origin: origin.to_string(),
        }
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

fn save_failed(e: crate::error::Error) -> Code {
    log::warn!("browser host: could not save a login: {e}");
    Code::ActionCancelledOrDenied
}

/// One row sealed and upserted, as `commands::vault::save_entry` writes one:
/// over the login `id` — its username and password, the rest of the entry
/// kept — or as a new login for `url` titled `host`. The stamps are the
/// editor's: `updatedAt` on every save, and the rotation stamp only when the
/// password actually changed. `now` is passed in so a test can pin it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn save_login_in(
    store: &SqliteStore,
    cipher: &PayloadCipher,
    id: Option<&str>,
    url: &str,
    host: &str,
    username: &str,
    password: &str,
    now: &str,
) -> std::result::Result<(), Code> {
    let entry = match id {
        Some(id) => {
            let record = store
                .get(id)
                .map_err(|e| save_failed(session::store_err(e)))?
                .ok_or(Code::NoValidUuidProvided)?;
            let mut entry = cipher
                .unseal(&record.id, &record.payload)
                .map_err(save_failed)?;
            if entry.kind != "login" {
                return Err(Code::NoValidUuidProvided);
            }
            if entry.password.as_deref().unwrap_or_default() != password {
                entry.password_updated_at = Some(now.to_string());
            }
            // Written back to the field it was served from: `logins_for` fills
            // the extension's username from `username`, or from `email` when
            // that is blank, so a login kept by its email must not grow a
            // second name beside it — unless what the page sent is not an
            // email at all, which the email field would only reject; that
            // goes to `username`, and the email stays what it was.
            let blank = |field: &Option<String>| field.as_deref().unwrap_or_default().is_empty();
            if blank(&entry.username) && !blank(&entry.email) && username.contains('@') {
                entry.email = Some(username.to_string());
            } else {
                entry.username = Some(username.to_string());
            }
            entry.password = Some(password.to_string());
            entry.updated_at = Some(now.to_string());
            entry
        }
        None => Entry {
            id: migrate::new_entry_id(),
            kind: "login".into(),
            title: host.to_string(),
            website: Some(url.to_string()),
            username: Some(username.to_string()),
            password: Some(password.to_string()),
            password_updated_at: (!password.is_empty()).then(|| now.to_string()),
            created_at: Some(now.to_string()),
            updated_at: Some(now.to_string()),
            ..Default::default()
        },
    };
    let payload = cipher.seal(&entry).map_err(save_failed)?;
    let record = migrate::build_record(&entry, payload).map_err(save_failed)?;
    store
        .upsert(&record)
        .map_err(|e| save_failed(session::store_err(e)))
}
