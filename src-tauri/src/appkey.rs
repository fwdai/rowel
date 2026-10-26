//! The app-level unlock: every workspace's key, held once the app is open.
//!
//! One password, or one biometric prompt, opens the whole device. The primary
//! workspace's key is the *app key*: every other workspace keeps a copy of its
//! own key sealed under it, in a sidecar beside its database
//! ([`storage::WRAPPED_KEY_FILE`]), so opening the primary opens them all —
//! without an Argon2id run per workspace, and without a biometric prompt per
//! workspace. The sidecar is local to this device on purpose: the pack a
//! workspace syncs stays under its own password-derived key, so another device
//! restores it with a password exactly as before, and the app key differs from
//! device to device (each has its own primary and its own salt).
//!
//! What is held here is key *material* rather than a [`VaultKey`]: the same
//! opaque bytes the biometric store keeps, interpreted by sidecar presence when
//! a database is opened with them ([`VaultKey::from_material`]).
//!
//! The ring outlives a workspace switch — that is its whole point — and ends
//! with every lock (`session::lock`), the auto-lock included. It is filled by
//! whichever unlock happens first: a primary unlock unwraps every sidecar; a
//! non-primary unlock adds its own key, and writes its sidecar once the app key
//! is known ([`adopt`]).

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use subtle::ConstantTimeEq;
use tauri::{AppHandle, Manager};
use zeroize::Zeroizing;

use crate::crypto::{self, VaultKey};
use crate::error::{Error, Result};
use crate::state::AppState;
use crate::storage;
use crate::workspace::{self, Registry, PRIMARY_ID};

/// The keys of every workspace the app has open, by workspace id.
#[derive(Default)]
pub struct Keyring {
    keys: HashMap<String, Zeroizing<Vec<u8>>>,
}

impl Keyring {
    pub fn insert(&mut self, id: &str, material: &[u8]) {
        self.keys
            .insert(id.to_string(), Zeroizing::new(material.to_vec()));
    }

    /// A scrubbed copy of one workspace's key material.
    pub fn get(&self, id: &str) -> Option<Zeroizing<Vec<u8>>> {
        self.keys.get(id).cloned()
    }

    pub fn has(&self, id: &str) -> bool {
        self.keys.contains_key(id)
    }

    pub fn remove(&mut self, id: &str) -> Option<Zeroizing<Vec<u8>>> {
        self.keys.remove(id)
    }

    /// Drop every key. Says whether there was anything to drop, so a lock can
    /// tell an app that was open at this level from one that was not.
    pub fn clear(&mut self) -> bool {
        let held = !self.keys.is_empty();
        self.keys.clear();
        held
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The app key: the primary's material, if the primary has been opened.
    pub fn app_key(&self) -> Option<Zeroizing<Vec<u8>>> {
        self.get(PRIMARY_ID)
    }

    /// Whether `app_key` is the app key the ring already holds — or the ring
    /// holds none yet. A ring that holds a *different* one is the newer fact:
    /// a password change replaced the primary's key while some slower proof
    /// of the old one was still running, and that proof must not win.
    pub fn accepts_app_key(&self, app_key: &[u8]) -> bool {
        match self.keys.get(PRIMARY_ID) {
            Some(current) => bool::from(current.ct_eq(app_key)),
            None => true,
        }
    }

    /// Every workspace but the primary, with its material.
    fn others(&self) -> Vec<(String, Zeroizing<Vec<u8>>)> {
        self.keys
            .iter()
            .filter(|(id, _)| id.as_str() != PRIMARY_ID)
            .map(|(id, material)| (id.clone(), material.clone()))
            .collect()
    }
}

// --- the sidecar ---------------------------------------------------------------

fn sidecar(root: &Path, id: &str) -> std::path::PathBuf {
    workspace::dir_of(root, id).join(storage::WRAPPED_KEY_FILE)
}

/// Whether workspace `id` has a key sealed under the app key on this device —
/// what the launch probe asks to say whether a biometric unlock can open it.
pub fn is_wrapped(root: &Path, id: &str) -> bool {
    id != PRIMARY_ID && sidecar(root, id).exists()
}

/// Seal workspace `id`'s key under the app key, beside its database. The seal
/// itself is the core's, so the iOS AutoFill extension opens the same file.
fn wrap(root: &Path, id: &str, app_key: &[u8], material: &[u8]) -> Result<()> {
    let sealed = crypto::seal_workspace_key(app_key, id, material)?;
    storage::atomic_write_private(&sidecar(root, id), sealed.as_bytes())
}

/// The key sealed for workspace `id`, if it has a sidecar the app key opens.
///
/// A sidecar that is whole but sealed under an app key this device no
/// longer has — what deleting the primary leaves behind — is stale and is
/// removed, so the next unlock of that workspace writes a fresh one rather
/// than the probe offering a biometric unlock that cannot work. Only on that
/// verdict (`Error::SealedUnderAnotherKey`): a read that fails, or a file
/// that is not a sealed blob, says nothing about the key — a disk that is
/// slow or briefly unavailable is not a stale sidecar — so it is reported
/// and the file left alone.
fn unwrap(root: &Path, id: &str, app_key: &[u8]) -> Result<Option<Zeroizing<Vec<u8>>>> {
    let path = sidecar(root, id);
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path)?;
    match crypto::open_workspace_key(app_key, id, &text) {
        Ok(material) => Ok(Some(material)),
        Err(Error::SealedUnderAnotherKey) => {
            log::warn!("workspace {id}'s sealed key is under an app key this device no longer has; removing it");
            let _ = fs::remove_file(&path);
            Err(Error::SealedUnderAnotherKey)
        }
        Err(e) => {
            log::warn!("workspace {id}'s sealed key could not be read and is left as it is: {e}");
            Err(e)
        }
    }
}

// --- filling the ring ------------------------------------------------------------
//
// Every write below is made under the session lock, and only for a session
// that is live. A lock takes the session first and clears the ring under it
// (`session::lock`), so a write that lands after a lock — a detached password
// proof finishing late, a password change whose session was locked while it
// ran, an account vault joined after the user walked away — finds no live
// session and stands down, rather than putting keys behind the lock screen
// that a switch would then open the vault with.
//
// And each of them runs whole under one more lock, `AppState::sidecars`: the
// check of which app key the ring holds, the sidecar reads and writes made on
// the strength of it, and the write back into the ring. The check alone would
// not do. A slower proof of an old app key could pass it, and a password
// change then replace the key and reseal every sidecar while the proof was
// still writing sidecars under the old one — or reading a freshly resealed one
// with the old key, failing its tag, and removing it as stale. Under the lock,
// either the change runs first and the proof stands down at its check, or the
// proof runs to its end and the change reseals what it wrote. The lock is taken
// before the session lock and never inside it, so `session::lock` never waits
// on filesystem work; and nothing here holds it across an Argon2id run — the
// proof itself happens before any of this is called.

fn sidecars(state: &AppState) -> std::sync::MutexGuard<'_, ()> {
    state.sidecars.lock().unwrap()
}

// A sidecar written under an app key the ring holds, or a note of why not.
fn seal(root: &Path, id: &str, app_key: &[u8], material: &[u8]) {
    if let Err(e) = wrap(root, id, app_key, material) {
        log::warn!("could not seal workspace {id}'s key under the app key: {e}");
    }
}

/// The primary has been opened: its key is the app key, and every workspace
/// with a sidecar it opens joins the ring with it. Best effort per workspace —
/// one that will not open is left for its own password.
///
/// A workspace already in the ring — opened with its own password before the
/// primary was, with no app key to seal it under at the time — is sealed now,
/// which is the moment its password and the app key are first both known.
pub fn open_all(app: &AppHandle, app_key: &[u8]) {
    if let Ok(root) = storage::root_dir(app) {
        open_all_in(&app.state::<AppState>(), &root, app_key);
    }
}

fn open_all_in(state: &AppState, root: &Path, app_key: &[u8]) {
    let _sidecars = sidecars(state);
    // What the ring already holds, read under the session guards and then
    // released: the filesystem work below — a sidecar per workspace — must not
    // hold up every command that needs the session, a lock included.
    let held = {
        let session = state.session.lock().unwrap();
        if !session.is_live() {
            return;
        }
        let ring = state.keyring.lock().unwrap();
        // Never over a different app key. This is the key some proof of the
        // primary's password produced — an unlock, the biometric item, the
        // detached try `auth::unlock` makes with another workspace's password
        // — and a password change that landed meanwhile has since replaced
        // it (`replace_app_key`). Sealing the sidecars under the old one would
        // leave every workspace closed to the next app unlock.
        if !ring.accepts_app_key(app_key) {
            log::info!("the app key changed while it was being proved; the older key stands down");
            return;
        }
        ring.others()
    };
    tests::pause();

    let mut opened = Vec::new();
    for workspace in Registry::load(root).workspaces {
        if workspace.id == PRIMARY_ID {
            continue;
        }
        match held.iter().find(|(id, _)| *id == workspace.id) {
            Some((_, material)) => seal(root, &workspace.id, app_key, material),
            None => {
                if let Ok(Some(material)) = unwrap(root, &workspace.id, app_key) {
                    opened.push((workspace.id, material));
                }
            }
        }
    }

    // The install-level files sealed under the app key — the server
    // connections, the enrollment key — are read by this key from here on;
    // one it does not open was sealed under a key this device no longer has,
    // and is set aside now so the next sign-in starts clean. Renamed, never
    // removed: unlike a workspace sidecar above, which the workspace's own
    // password rewrites, nothing rewrites the enrollment key's private half.
    crate::org::set_aside_stale(root, app_key);

    // Into the ring under the session guards again, and only if the session is
    // still live: a lock that landed during the reads above cleared the ring,
    // and what was read before it does not undo that. The app key is still the
    // one checked above — the only writes that replace it wait on the sidecar
    // lock this holds — so the check is repeated only as the last word against
    // a writer added later.
    let session = state.session.lock().unwrap();
    if !session.is_live() {
        return;
    }
    let mut ring = state.keyring.lock().unwrap();
    if !ring.accepts_app_key(app_key) {
        log::error!(
            "the app key changed under the sidecar lock; the sidecars just written are stale"
        );
        return;
    }
    ring.insert(PRIMARY_ID, app_key);
    for (id, material) in opened {
        ring.insert(&id, &material);
    }
}

/// The primary's key was changed by a master-password change: the ring takes
/// the new app key over whatever it held, and every sidecar is resealed under
/// it. The one write that replaces an app key on purpose — every other route
/// into the ring refuses to (`Keyring::accepts_app_key`).
pub fn replace_app_key(app: &AppHandle, app_key: &[u8]) {
    if let Ok(root) = storage::root_dir(app) {
        replace_app_key_in(&app.state::<AppState>(), &root, app_key);
    }
}

fn replace_app_key_in(state: &AppState, root: &Path, app_key: &[u8]) {
    let _sidecars = sidecars(state);
    let (replaced, others) = {
        let session = state.session.lock().unwrap();
        if !session.is_live() {
            return;
        }
        let mut ring = state.keyring.lock().unwrap();
        // The key being retired, read before the ring takes the new one: the
        // install-level files are sealed under it and open under nothing else.
        let replaced = ring.app_key();
        ring.insert(PRIMARY_ID, app_key);
        (replaced, ring.others())
    };
    for (id, material) in others {
        seal(root, &id, app_key, &material);
    }
    // The server connections and the enrollment key follow the sidecars
    // onto the new app key. A ring that held no app key — the primary opened
    // by nothing this session — leaves them under the old one, and they go
    // as stale at the next primary unlock.
    if let Some(replaced) = replaced {
        crate::org::reseal_under(root, &replaced, app_key);
    }
}

/// Whether the next unlock of the main lockbox opens a workspace too: what
/// [`adopt`] and [`rekeyed`] say they did, for a caller that has to know.
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adoption {
    /// Sealed beside its database under the app key — or the primary, whose
    /// key the app key is.
    Sealed,
    /// Not sealed: no app key was there to seal it under, so the next unlock
    /// opens it only with its own password. In the ring for this session if
    /// one is live.
    Unsealed,
}

impl Adoption {
    /// For a workspace with no password of its own — an org lockbox, whose
    /// key only its server hands over — a key the next unlock will not have
    /// is a failure, and is said as one rather than reported as done.
    pub fn required(self) -> Result<()> {
        match self {
            Self::Sealed => Ok(()),
            Self::Unsealed => Err(Error::Other(
                "the main lockbox is not open, so this lockbox's key could not be kept for the \
                 next unlock; open the main lockbox and refresh the key"
                    .into(),
            )),
        }
    }
}

/// A workspace other than the primary has been opened with its own key: it
/// joins the ring, and — when the app key is known — its sidecar is written so
/// the next app unlock opens it too. Without the app key the sidecar waits for
/// an unlock that has it (see `commands::auth::unlock`), and the answer says
/// so. `Locked` when no session is live, since nothing joins an empty ring,
/// and the sidecar's write error when it cannot be written.
pub fn adopt(app: &AppHandle, id: &str, material: &[u8]) -> Result<Adoption> {
    adopt_in(
        &app.state::<AppState>(),
        &storage::root_dir(app)?,
        id,
        material,
    )
}

/// [`adopt`] where nothing waits on the outcome — an unlock, a switch, a
/// create or a restore, whose workspace is open either way and keeps a
/// password of its own — and a failure is noted rather than returned. A lock
/// that landed first is not a failure: there is no session to open it in.
pub fn adopt_or_log(app: &AppHandle, id: &str, material: &[u8]) {
    match adopt(app, id, material) {
        Ok(_) | Err(Error::Locked) => {}
        Err(e) => log::warn!("could not keep workspace {id}'s key for the next unlock: {e}"),
    }
}

fn adopt_in(state: &AppState, root: &Path, id: &str, material: &[u8]) -> Result<Adoption> {
    if id == PRIMARY_ID {
        if !state.session.lock().unwrap().is_live() {
            return Err(Error::Locked);
        }
        open_all_in(state, root, material);
        return Ok(Adoption::Sealed);
    }
    let _sidecars = sidecars(state);
    let app_key = {
        let session = state.session.lock().unwrap();
        if !session.is_live() {
            return Err(Error::Locked);
        }
        let mut ring = state.keyring.lock().unwrap();
        ring.insert(id, material);
        ring.app_key()
    };
    match app_key {
        Some(app_key) => {
            wrap(root, id, &app_key, material)?;
            Ok(Adoption::Sealed)
        }
        None => Ok(Adoption::Unsealed),
    }
}

/// Workspace `id`'s database has just been re-keyed onto `material`: the
/// sealed copy beside it follows, or goes. The rule this keeps is that the
/// copy holds the key the database is under, or there is none — a copy of
/// the retired key would hand the next unlock a key that no longer opens the
/// database, and an org lockbox has no password to fall back on.
///
/// Unlike [`adopt`], a live session is not needed: a lock that landed during
/// the re-key clears the ring, but the database is on the new key either way
/// and the copy beside it is what the next unlock trusts. It is sealed under
/// the app key the ring holds, else `app_key`, the one the caller read from
/// the ring before the re-key began (a lock does not change the app key; a
/// password change that did is the ring's newer word, and wins). With no app
/// key at all, or when the write fails, the old copy is removed, and a
/// removal that fails too is the error. The ring takes the new key when a
/// session is live. The primary's re-key is [`replace_app_key`]'s.
pub fn rekeyed(
    app: &AppHandle,
    id: &str,
    app_key: Option<Zeroizing<Vec<u8>>>,
    material: &[u8],
) -> Result<Adoption> {
    rekeyed_in(
        &app.state::<AppState>(),
        &storage::root_dir(app)?,
        id,
        app_key,
        material,
    )
}

fn rekeyed_in(
    state: &AppState,
    root: &Path,
    id: &str,
    captured: Option<Zeroizing<Vec<u8>>>,
    material: &[u8],
) -> Result<Adoption> {
    // The primary keeps no sealed copy: its key is the app key, replaced on
    // purpose and nowhere else.
    if id == PRIMARY_ID {
        return Err(Error::Other(
            "the main lockbox's key is the app key; it is replaced, not sealed".into(),
        ));
    }
    let _sidecars = sidecars(state);
    let app_key = {
        let session = state.session.lock().unwrap();
        let mut ring = state.keyring.lock().unwrap();
        if session.is_live() {
            ring.insert(id, material);
        }
        ring.app_key().or(captured)
    };
    let Some(app_key) = app_key else {
        remove_obsolete(root, id)?;
        return Ok(Adoption::Unsealed);
    };
    if let Err(e) = wrap(root, id, &app_key, material) {
        if let Err(removal) = remove_obsolete(root, id) {
            log::warn!(
                "workspace {id}'s retired sealed key could not be removed either: {removal}"
            );
        }
        return Err(e);
    }
    Ok(Adoption::Sealed)
}

// The sealed copy of a key the database is no longer under: gone, so the
// next unlock cannot be handed it. Already absent is the goal reached.
fn remove_obsolete(root: &Path, id: &str) -> Result<()> {
    match fs::remove_file(sidecar(root, id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Workspace `id` has just been moved onto the vault password, which the caller
/// proved as `app_key`: its key is sealed under that app key, and joins the
/// ring if a session is live.
///
/// Unlike [`adopt`], the sidecar is written with no live session too: the move
/// can be made from a lockbox's lock screen, where the ring is empty, and the
/// app key it is sealed under was proved by the vault password a moment ago —
/// the sidecar adds nothing an attacker without that key could open. Not over a
/// *different* app key the ring holds, by the rule every writer here keeps.
///
/// The move is a re-key, so the rule [`rekeyed`] keeps holds here too: a copy
/// sealed earlier holds the key the lockbox just left, and is removed when the
/// new one cannot be written in its place — on a failed write, which is the
/// error, and when the app key changed meanwhile and this stands down.
pub fn seal_under(app: &AppHandle, id: &str, app_key: &[u8], material: &[u8]) -> Result<()> {
    seal_under_in(
        &app.state::<AppState>(),
        &storage::root_dir(app)?,
        id,
        app_key,
        material,
    )
}

fn seal_under_in(
    state: &AppState,
    root: &Path,
    id: &str,
    app_key: &[u8],
    material: &[u8],
) -> Result<()> {
    if id == PRIMARY_ID {
        return Ok(());
    }
    let _sidecars = sidecars(state);
    {
        let session = state.session.lock().unwrap();
        let mut ring = state.keyring.lock().unwrap();
        if !ring.accepts_app_key(app_key) {
            log::info!("the app key changed while lockbox {id} was being moved; it stands down");
            return remove_obsolete(root, id);
        }
        if session.is_live() {
            ring.insert(PRIMARY_ID, app_key);
            ring.insert(id, material);
        }
    }
    if let Err(e) = wrap(root, id, app_key, material) {
        if let Err(removal) = remove_obsolete(root, id) {
            log::warn!("lockbox {id}'s retired sealed key could not be removed either: {removal}");
        }
        return Err(e);
    }
    Ok(())
}

/// Workspace `id` was deleted: its key leaves the ring. When another workspace
/// was promoted into the primary's place, its key is the app key now: the
/// sealed copy of it that came into the root with its other files goes (the
/// primary keeps none), and everything the ring still holds is resealed under
/// it. A sidecar the ring does not hold the key for is left as it is, and goes
/// when the next `open_all` fails to open it.
pub fn forget(app: &AppHandle, id: &str, promoted: Option<&str>) {
    let Ok(root) = storage::root_dir(app) else {
        return;
    };
    let state = app.state::<AppState>();
    let _sidecars = sidecars(&state);
    let mut ring = state.keyring.lock().unwrap();
    let removed = ring.remove(id);
    let Some(promoted) = promoted else {
        return;
    };
    if let Some(material) = ring.remove(promoted) {
        ring.insert(PRIMARY_ID, &material);
    }
    let app_key = ring.app_key();
    let others = ring.others();
    drop(ring);
    let _ = storage::remove_if_present(&root.join(storage::WRAPPED_KEY_FILE));
    let Some(app_key) = app_key else {
        return;
    };
    for (id, material) in others {
        seal(&root, &id, &app_key, &material);
    }
    // A promotion only ever follows the primary's deletion, so what was just
    // removed from the ring is the app key the install-level files are
    // sealed under: they move onto the promoted key while the old one is
    // still here to open them. Deleted from the lock screen, with no key in
    // the ring, they go as stale at the next primary unlock instead.
    if let Some(replaced) = removed {
        crate::org::reseal_under(&root, &replaced, &app_key);
    }
}

/// Open the primary's sealed copy of workspace `id`'s key with `app_key`, as a
/// [`VaultKey`] ready to open that workspace's database. `None` when it has no
/// sidecar, or one the app key does not open.
pub fn unwrap_key(app: &AppHandle, id: &str, app_key: &[u8]) -> Option<VaultKey> {
    let root = storage::root_dir(app).ok()?;
    // Under the sidecar lock like every other read: a sidecar being resealed
    // by a password change must not be read half-way, nor removed as stale
    // for failing under a key that is a moment out of date.
    let state = app.state::<AppState>();
    let _sidecars = sidecars(&state);
    let material = unwrap(&root, id, app_key).ok()??;
    Some(VaultKey::from_material(
        material,
        workspace::dir_of(&root, id)
            .join(storage::KDF_SIDECAR_FILE)
            .exists(),
    ))
}

// A point a test can hold `open_all` at, between its check of the ring and the
// sidecars it writes on the strength of it — the window the sidecar lock
// closes. Nothing outside tests.
#[cfg(not(test))]
mod tests {
    pub fn pause() {}
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use super::*;
    use crate::store::SqliteStore;
    use crate::workspace::Workspace;

    type Hook = Box<dyn Fn()>;
    thread_local! {
        // Per thread, not per process: tests run in parallel, and a hook one
        // test hung in a shared slot fired inside any other test's
        // `open_all_in` that overlapped it — asserting that test's ring, and
        // poisoning the slot for the one that owned it. `open_all_in` calls
        // `pause` on its caller's thread, so the hook only ever meets the open
        // its own test started.
        static PAUSE: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }

    /// What `open_all` runs between its check of the ring and its sidecar
    /// writes: nothing, unless this thread's test has hung something there.
    pub fn pause() {
        PAUSE.with_borrow(|hook| {
            if let Some(hook) = hook {
                hook();
            }
        });
    }

    fn tmp_root() -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rowel-keyring-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    const APP_KEY: &[u8] = &[7u8; 32];

    #[test]
    fn a_wrapped_key_round_trips_under_the_app_key() {
        let root = tmp_root();
        wrap(&root, "a1b2", APP_KEY, b"vault-key").unwrap();

        assert!(is_wrapped(&root, "a1b2"));
        assert_eq!(
            &**unwrap(&root, "a1b2", APP_KEY).unwrap().unwrap(),
            b"vault-key"
        );
    }

    #[test]
    fn a_workspace_without_a_sidecar_has_nothing_to_unwrap() {
        let root = tmp_root();
        assert!(!is_wrapped(&root, "a1b2"));
        assert!(unwrap(&root, "a1b2", APP_KEY).unwrap().is_none());
    }

    // Deleting the primary leaves every sidecar sealed under a key this device
    // no longer has. Such a sidecar is an error to open, and it goes.
    #[test]
    fn a_sidecar_under_another_app_key_is_stale_and_removed() {
        let root = tmp_root();
        wrap(&root, "a1b2", APP_KEY, b"vault-key").unwrap();

        assert!(unwrap(&root, "a1b2", &[9u8; 32]).is_err());
        assert!(!is_wrapped(&root, "a1b2"));
    }

    // A file that is not a sealed key — truncated, overwritten, not base64 —
    // says nothing about the app key, so it is an error and nothing more: it
    // stays, and only the wrong-key verdict ever removes a sidecar.
    #[test]
    fn a_malformed_sidecar_is_left_in_place() {
        let root = tmp_root();
        fs::create_dir_all(workspace::dir_of(&root, "a1b2")).unwrap();
        fs::write(sidecar(&root, "a1b2"), "not a sealed key").unwrap();
        assert!(matches!(
            unwrap(&root, "a1b2", APP_KEY),
            Err(Error::Crypto(_))
        ));
        assert!(is_wrapped(&root, "a1b2"));

        wrap(&root, "a1b2", APP_KEY, b"vault-key").unwrap();
        let sealed = fs::read_to_string(sidecar(&root, "a1b2")).unwrap();
        fs::write(sidecar(&root, "a1b2"), &sealed[..sealed.len() / 2]).unwrap();
        assert!(unwrap(&root, "a1b2", APP_KEY).is_err());
        assert!(is_wrapped(&root, "a1b2"), "truncated, not stale");
    }

    // The id is authenticated: a sidecar moved under another workspace's
    // directory must not open there.
    #[test]
    fn a_sidecar_is_bound_to_its_workspace() {
        let root = tmp_root();
        wrap(&root, "a1b2", APP_KEY, b"vault-key").unwrap();
        fs::create_dir_all(workspace::dir_of(&root, "c3d4")).unwrap();
        fs::rename(sidecar(&root, "a1b2"), sidecar(&root, "c3d4")).unwrap();

        assert!(unwrap(&root, "c3d4", APP_KEY).is_err());
    }

    #[test]
    fn the_primary_is_never_wrapped() {
        let root = tmp_root();
        assert!(!is_wrapped(&root, PRIMARY_ID));
    }

    // A proof of the old primary password finishing after a password change
    // must not put the old key back: the ring's key is the newer fact.
    #[test]
    fn the_ring_refuses_a_different_app_key_once_it_holds_one() {
        let mut ring = Keyring::default();
        assert!(ring.accepts_app_key(APP_KEY));
        ring.insert(PRIMARY_ID, APP_KEY);

        assert!(ring.accepts_app_key(APP_KEY));
        assert!(!ring.accepts_app_key(&[9u8; 32]));
    }

    // --- the ring under a live session --------------------------------------------

    const OLD_KEY: &[u8] = &[1u8; 32];
    const NEW_KEY: &[u8] = &[2u8; 32];
    const X_KEY: &[u8] = &[3u8; 32];
    const Y_KEY: &[u8] = &[4u8; 32];

    // A root with the primary and two more workspaces, `x` and `y`, and an app
    // state whose session is live on the primary.
    fn live_state(root: &Path) -> Arc<AppState> {
        let registry = Registry {
            active: PRIMARY_ID.to_string(),
            workspaces: ["default", "x", "y"]
                .into_iter()
                .map(|id| Workspace {
                    id: id.to_string(),
                    name: None,
                    vault_id: None,
                    item_count: None,
                    color: None,
                    agents: false,
                    server: None,
                })
                .collect(),
        };
        registry.save(root).unwrap();
        for id in ["x", "y"] {
            fs::create_dir_all(workspace::dir_of(root, id)).unwrap();
        }
        let state = Arc::new(AppState::default());
        let key = VaultKey::legacy_from_password("primary");
        let store = SqliteStore::open(&root.join("vault.db"), &*key.sqlcipher_key()).unwrap();
        state.session.lock().unwrap().set(key, store, false);
        state
    }

    fn sealed_under(root: &Path, id: &str, app_key: &[u8]) -> Vec<u8> {
        unwrap(root, id, app_key).unwrap().unwrap().to_vec()
    }

    #[test]
    fn a_primary_unlock_seals_what_the_ring_holds_and_opens_what_it_does_not() {
        let root = tmp_root();
        let state = live_state(&root);
        state.keyring.lock().unwrap().insert("x", X_KEY);
        wrap(&root, "y", OLD_KEY, Y_KEY).unwrap();

        open_all_in(&state, &root, OLD_KEY);

        let ring = state.keyring.lock().unwrap();
        assert_eq!(&**ring.app_key().unwrap(), OLD_KEY);
        assert_eq!(&**ring.get("y").unwrap(), Y_KEY, "opened from its sidecar");
        drop(ring);
        assert_eq!(sealed_under(&root, "x", OLD_KEY), X_KEY, "sealed now");
    }

    // The server connections and the enrollment key are sealed under the app
    // key too: a password change carries them onto the new one, and a primary
    // unlock under a key that does not open them sets what is left aside.
    #[test]
    fn a_password_change_reseals_the_org_files_and_a_primary_unlock_sets_stale_ones_aside() {
        use crate::org::{connections, enrollment};

        let root = tmp_root();
        let state = live_state(&root);
        state.keyring.lock().unwrap().insert(PRIMARY_ID, OLD_KEY);
        connections::upsert(
            &root,
            OLD_KEY,
            connections::tests::connection("https://a.example", "a1"),
        )
        .unwrap();
        let minted = enrollment::load_or_create(&root, OLD_KEY).unwrap();

        replace_app_key_in(&state, &root, NEW_KEY);

        assert!(connections::load(&root, OLD_KEY).is_err());
        assert_eq!(
            connections::load(&root, NEW_KEY).unwrap()[0].access_token,
            "a1"
        );
        assert_eq!(
            enrollment::load_or_create(&root, NEW_KEY)
                .unwrap()
                .public_key_b64(),
            minted.public_key_b64()
        );

        // The primary deleted and another key proved in its place: the files
        // the old key sealed do not open, and are not left to fail forever —
        // nor deleted: both are moved aside with their bytes.
        state.keyring.lock().unwrap().clear();
        open_all_in(&state, &root, X_KEY);
        assert!(!root.join(storage::SERVERS_FILE).exists());
        assert!(!root.join(storage::ENROLLMENT_KEY_FILE).exists());
        let aside = fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|name| name.contains(".stale-"))
            .count();
        assert_eq!(aside, 2);
    }

    #[test]
    fn a_proof_of_an_app_key_the_ring_has_replaced_writes_nothing() {
        let root = tmp_root();
        let state = live_state(&root);
        state.keyring.lock().unwrap().insert(PRIMARY_ID, NEW_KEY);
        state.keyring.lock().unwrap().insert("x", X_KEY);

        open_all_in(&state, &root, OLD_KEY);

        assert!(!is_wrapped(&root, "x"), "no sidecar under the retired key");
        assert_eq!(&**state.keyring.lock().unwrap().app_key().unwrap(), NEW_KEY);
    }

    // The window the sidecar lock closes. A proof of the old app key has passed
    // its check of the ring and is about to write sidecars by it; meanwhile a
    // password change replaces the app key, rekeys another workspace, and a
    // lock lands. Without the lock, the proof would seal `x` under the retired
    // key over the change's fresh sidecar, read `y`'s fresh sidecar with the
    // retired key and remove it as stale, and the lock would leave nothing to
    // put either right — so the next app unlock would open neither. Under it,
    // the change waits for the proof to finish and reseals everything it wrote.
    #[test]
    fn a_password_change_waits_for_a_proof_of_the_old_app_key_to_finish() {
        let root = tmp_root();
        let state = live_state(&root);
        state.keyring.lock().unwrap().insert("x", X_KEY);
        wrap(&root, "y", OLD_KEY, Y_KEY).unwrap();

        let change: Arc<Mutex<Option<JoinHandle<()>>>> = Arc::default();
        {
            let (state, root, change) = (state.clone(), root.clone(), change.clone());
            PAUSE.set(Some(Box::new(move || {
                let worker = {
                    let (state, root) = (state.clone(), root.clone());
                    thread::spawn(move || {
                        replace_app_key_in(&state, &root, NEW_KEY);
                        assert_eq!(
                            adopt_in(&state, &root, "y", Y_KEY).unwrap(),
                            Adoption::Sealed
                        );
                        // A lock: the session and the ring end (`session::lock`).
                        state.session.lock().unwrap().clear();
                        state.keyring.lock().unwrap().clear();
                    })
                };
                thread::sleep(Duration::from_millis(200));
                assert!(
                    state.keyring.lock().unwrap().accepts_app_key(OLD_KEY),
                    "the change is waiting on the proof"
                );
                *change.lock().unwrap() = Some(worker);
            })));
        }

        open_all_in(&state, &root, OLD_KEY);
        PAUSE.set(None);
        change.lock().unwrap().take().unwrap().join().unwrap();

        assert_eq!(sealed_under(&root, "x", NEW_KEY), X_KEY);
        assert_eq!(sealed_under(&root, "y", NEW_KEY), Y_KEY);
        assert!(
            state.keyring.lock().unwrap().is_empty(),
            "the lock had the last word"
        );
    }

    // A lockbox moved onto the vault password from its lock screen: no session,
    // so nothing goes into the ring, but the sidecar is written under the app
    // key the vault password proved.
    #[test]
    fn a_moved_lockbox_is_sealed_with_no_session_and_joins_a_live_ring() {
        let root = tmp_root();
        let state = live_state(&root);
        state.session.lock().unwrap().clear();

        seal_under_in(&state, &root, "x", OLD_KEY, X_KEY).unwrap();
        assert_eq!(sealed_under(&root, "x", OLD_KEY), X_KEY);
        assert!(state.keyring.lock().unwrap().is_empty());

        let root = tmp_root();
        let live = live_state(&root);
        seal_under_in(&live, &root, "y", OLD_KEY, Y_KEY).unwrap();
        assert_eq!(sealed_under(&root, "y", OLD_KEY), Y_KEY);
        let ring = live.keyring.lock().unwrap();
        assert_eq!(&**ring.get("y").unwrap(), Y_KEY);
        assert_eq!(&**ring.app_key().unwrap(), OLD_KEY);
    }

    // A move proved under an app key the ring has since replaced writes
    // nothing — and leaves no sealed copy of the key the lockbox just left.
    #[test]
    fn a_move_proved_under_a_replaced_app_key_writes_nothing() {
        let root = tmp_root();
        let state = live_state(&root);
        state.keyring.lock().unwrap().insert(PRIMARY_ID, NEW_KEY);
        fs::create_dir_all(workspace::dir_of(&root, "x")).unwrap();
        wrap(&root, "x", NEW_KEY, b"the key it left").unwrap();

        seal_under_in(&state, &root, "x", OLD_KEY, X_KEY).unwrap();
        assert!(!is_wrapped(&root, "x"));
        assert!(!state.keyring.lock().unwrap().has("x"));
    }

    // An adopt says what it did: `Locked` with no session, the ring alone
    // with no app key to seal under, and sealed otherwise. An org lockbox's
    // flows require the last.
    #[test]
    fn an_adopt_says_whether_the_next_unlock_has_the_key() {
        let root = tmp_root();
        let state = live_state(&root);
        assert_eq!(
            adopt_in(&state, &root, "x", X_KEY).unwrap(),
            Adoption::Unsealed
        );
        assert!(Adoption::Unsealed.required().is_err());
        assert!(state.keyring.lock().unwrap().has("x"));

        state.keyring.lock().unwrap().insert(PRIMARY_ID, OLD_KEY);
        assert_eq!(
            adopt_in(&state, &root, "x", X_KEY).unwrap(),
            Adoption::Sealed
        );
        assert_eq!(sealed_under(&root, "x", OLD_KEY), X_KEY);

        state.session.lock().unwrap().clear();
        assert!(matches!(
            adopt_in(&state, &root, "y", Y_KEY),
            Err(Error::Locked)
        ));
        assert!(matches!(
            adopt_in(&state, &root, PRIMARY_ID, OLD_KEY),
            Err(Error::Locked)
        ));
        assert!(!is_wrapped(&root, "y"));
    }

    // A sealed copy that cannot be written is the error, never a quiet
    // success: here the sidecar's path is taken by a directory.
    #[test]
    fn a_seal_that_fails_is_reported() {
        let root = tmp_root();
        let state = live_state(&root);
        state.keyring.lock().unwrap().insert(PRIMARY_ID, OLD_KEY);
        fs::create_dir_all(sidecar(&root, "x").join("in-the-way")).unwrap();
        assert!(adopt_in(&state, &root, "x", X_KEY).is_err());
        assert!(rekeyed_in(&state, &root, "x", None, X_KEY).is_err());
    }

    // After a re-key the sealed copy holds the new key or is gone: written
    // under the ring's app key, under the one read before the re-key when a
    // lock cleared the ring meanwhile, and removed when there is no app key
    // at all, so the key just retired is never handed to the next unlock.
    #[test]
    fn after_a_re_key_the_sealed_copy_holds_the_new_key_or_is_gone() {
        let root = tmp_root();
        let state = live_state(&root);
        fs::create_dir_all(workspace::dir_of(&root, "x")).unwrap();
        state.keyring.lock().unwrap().insert(PRIMARY_ID, OLD_KEY);
        wrap(&root, "x", OLD_KEY, b"retired").unwrap();

        assert_eq!(
            rekeyed_in(&state, &root, "x", None, X_KEY).unwrap(),
            Adoption::Sealed
        );
        assert_eq!(sealed_under(&root, "x", OLD_KEY), X_KEY);
        assert_eq!(&**state.keyring.lock().unwrap().get("x").unwrap(), X_KEY);

        // A lock lands during the next re-key: the ring is empty, but the app
        // key read before it still seals the copy, and nothing joins the ring.
        state.session.lock().unwrap().clear();
        state.keyring.lock().unwrap().clear();
        assert_eq!(
            rekeyed_in(
                &state,
                &root,
                "x",
                Some(Zeroizing::new(OLD_KEY.to_vec())),
                Y_KEY
            )
            .unwrap(),
            Adoption::Sealed
        );
        assert_eq!(sealed_under(&root, "x", OLD_KEY), Y_KEY);
        assert!(state.keyring.lock().unwrap().is_empty());

        // No app key anywhere: the copy of the retired key goes.
        assert_eq!(
            rekeyed_in(&state, &root, "x", None, X_KEY).unwrap(),
            Adoption::Unsealed
        );
        assert!(!is_wrapped(&root, "x"));

        assert!(rekeyed_in(&state, &root, PRIMARY_ID, None, X_KEY).is_err());
    }

    #[test]
    fn the_ring_tells_the_app_key_from_the_rest() {
        let mut ring = Keyring::default();
        assert!(ring.app_key().is_none());
        ring.insert("a1b2", b"a");
        ring.insert(PRIMARY_ID, b"p");

        assert_eq!(&**ring.app_key().unwrap(), b"p");
        assert_eq!(ring.others().len(), 1);
        assert!(ring.clear());
        assert!(!ring.clear());
    }
}
