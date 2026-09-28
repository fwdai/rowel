//! On-disk vault storage under the Tauri app-data dir (on iOS, the App Group
//! container): ensure-file, utf8 read, overwrite write, `.swftx` export copy.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};

pub const DB_FILE: &str = "vault.db";
// Pre-change recovery snapshot of the encrypted DB, written next to it before the
// destructive change-master-password sequence (see `change_master_password`).
pub const DB_REKEY_BACKUP_FILE: &str = "vault.db.rekey-backup";
// Plaintext KDF descriptor stored next to the DB. It holds the Argon2id params +
// salt (public by design) and is read *before* deriving the key — the salt/params
// cannot live inside the encrypted DB, since deriving the key is what opens it.
pub const KDF_SIDECAR_FILE: &str = "vault.kdf.json";
// Pre-change recovery snapshot of the KDF sidecar, taken alongside the DB one.
// Rolling the DB back to its old key is only half a rollback: the descriptor that
// says how to derive that key has to roll back with it, or nothing opens.
pub const KDF_SIDECAR_REKEY_BACKUP_FILE: &str = "vault.kdf.json.rekey-backup";
// Plaintext failed-unlock backoff state (T-AUTH-3), stored next to the DB for the
// same reason as the KDF sidecar: a wrong password never opens the encrypted DB,
// so the attempt counter cannot live in the `meta` table. Public by design —
// it only ever holds a counter and a timestamp, nothing secret.
pub const LOCKOUT_SIDECAR_FILE: &str = "vault.lock.json";
// User preferences (theme, locale, auto-lock, …). Plaintext and readable while
// locked by design: the shell has to know what to draw before the vault opens.
pub const SETTINGS_FILE: &str = "settings.json";
pub const GDRIVE_FILE: &str = "auth/gdrive.swftx";
// Marker for "biometric unlock is enabled". The key itself lives in the OS
// secure store; this flag lets us report availability without a biometric prompt.
// Its contents name the gate the key was enrolled behind (`secure_store::GateMode`)
// — not a secret: it says *how* the key is gated, never anything about the key.
pub const BIOMETRIC_FILE: &str = "biometric.enabled";
// A non-primary workspace's own vault key, sealed under the primary's (the app
// key) so that one unlock opens every workspace on the device — see
// `crate::appkey`. Ciphertext, but held to `0600` all the same; the primary
// has none, since its key *is* the app key.
pub const WRAPPED_KEY_FILE: &str = "vault.key.sealed";
// Working space for the sync engine, inside the workspace's own directory (see
// `sync_scratch_dir`).
const SYNC_SCRATCH_DIR: &str = "sync-scratch";

// What one workspace's vault is made of, named in one place so the two flows
// that move a whole vault — the delete that parks it out of the way, and the
// promotion that carries a survivor into the root — can never disagree about
// what a vault is.
//
// The rekey pair is part of it. A password change interrupted by a crash leaves
// its snapshots beside the database, and they are half of the vault until the
// next unlock reconciles them (`auth::recover_interrupted_rekey`): a promotion
// that left them behind would hand that unlock a rekeyed database with no
// sidecar to roll it back to, and one left in the root would roll the *deleted*
// vault back over the promoted one.
//
// Split only by what it takes to name each: a database is its main file plus
// whatever `-wal`/`-shm` siblings it has (see `db_files`), a sidecar is the one
// file.
const VAULT_DB_FILES: [&str; 2] = [DB_FILE, DB_REKEY_BACKUP_FILE];
const VAULT_SIDECAR_FILES: [&str; 5] = [
    KDF_SIDECAR_FILE,
    KDF_SIDECAR_REKEY_BACKUP_FILE,
    LOCKOUT_SIDECAR_FILE,
    GDRIVE_FILE,
    WRAPPED_KEY_FILE,
];

// What a workspace keeps beside its vault and never takes with it. The
// biometric marker names the one keychain item this install has, and deleting
// the primary — whose key that item holds — deletes the item with it
// (`workspace_delete`): a marker that travelled would claim an enrollment that
// no longer exists. The scratch directory is a sync run's working space, made
// again on demand.
//
// Both still belong to the workspace that holds them, so a delete moves them
// out of the way rather than removing them: what it moves it can put back.
const WORKSPACE_LOCAL_FILES: [&str; 1] = [BIOMETRIC_FILE];

// The data dir for the whole install: what every workspace hangs off, and where
// the workspace registry itself lives.
pub fn root_dir(app: &AppHandle) -> Result<PathBuf> {
    // E2E test isolation: point the whole data dir at a fresh temp dir per run.
    // Debug builds only, like the reset command that depends on it
    // (`commands::e2e`): a release binary must not let a variable in its
    // environment decide where the vault is written — or read from.
    if cfg!(debug_assertions) {
        if let Ok(dir) = std::env::var("ROWEL_DB_DIR") {
            return Ok(PathBuf::from(dir));
        }
    }

    let dir = app_data_root(app)?;
    #[cfg(target_os = "ios")]
    let dir = ios::shared_root(dir);
    Ok(dir)
}

// Where the data dir has always been: the app's own data directory.
fn app_data_root(app: &AppHandle) -> Result<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| Error::Other(e.to_string()))?;
    Ok(dev_subdir(dir))
}

// Dev builds share the prod identifier, so isolate their data in a subdir
// to avoid mutating the real vault while iterating.
fn dev_subdir(dir: PathBuf) -> PathBuf {
    if cfg!(debug_assertions) {
        dir.join("dev")
    } else {
        dir
    }
}

// On iOS the data dir is in the App Group container rather than the app's own
// sandbox, so the AutoFill extension — a separate process with a sandbox of its
// own — can open the same vault (`rowel_core::app::APP_GROUP`).
#[cfg(target_os = "ios")]
mod ios {
    use std::path::PathBuf;
    use std::sync::OnceLock;

    use objc2_foundation::{NSFileManager, NSString};
    use rowel_core::app::{APP_GROUP, APP_GROUP_DATA_DIR};

    // Resolved, and the old directory moved into it, once per process: every
    // path in the app goes through `root_dir`, and the move must have finished
    // before the first of them is read. `get_or_init` holds any other caller
    // until it has.
    //
    // A container iOS will not hand over — the App ID was never given the App
    // Group, or the entitlement is missing from the build — leaves the vault
    // where it was: the app keeps working, and only the extension cannot see
    // it. The next launch that does get a container moves it then.
    pub(super) fn shared_root(app_data: PathBuf) -> PathBuf {
        static ROOT: OnceLock<PathBuf> = OnceLock::new();
        ROOT.get_or_init(|| {
            let Some(container) = container() else {
                log::warn!("no App Group container for {APP_GROUP}; the vault stays in the app's own data dir");
                return app_data;
            };
            let shared = super::dev_subdir(container.join(APP_GROUP_DATA_DIR));
            match super::move_data_dir(&app_data, &shared) {
                Ok(moved) if moved.is_empty() => {}
                Ok(moved) => log::info!(
                    "moved the data dir into the App Group container: {}",
                    moved.join(", ")
                ),
                // Nothing is lost either way: whatever did not move is still
                // in the old directory, and the next launch tries again.
                Err(e) => log::warn!("could not move the data dir into the App Group container: {e}"),
            }
            shared
        })
        .clone()
    }

    fn container() -> Option<PathBuf> {
        NSFileManager::defaultManager()
            .containerURLForSecurityApplicationGroupIdentifier(&NSString::from_str(APP_GROUP))?
            .to_file_path()
    }
}

/// Move everything in the data directory `from` into `to`, one top-level entry
/// at a time — the workspace registry, the preferences, every workspace — then
/// remove `from`. Returns the names it moved.
///
/// Each entry is one rename, so a crash leaves every entry wholly in one
/// directory or the other, never torn. The next run finds `from` still there
/// with what is left and finishes the move: an entry already in `to` is one it
/// moved itself, and the rest have nowhere else to be.
///
/// It never overwrites. An entry about to move that is already in `to` means
/// both directories hold a vault of their own — a half-finished move cannot
/// produce one, since an entry is in one place or the other — and pairing one's
/// database with the other's sidecars would open neither. So every name is
/// checked before the first moves, and on a collision nothing does.
#[cfg(any(target_os = "ios", test))]
fn move_data_dir(from: &Path, to: &Path) -> Result<Vec<String>> {
    let entries = match fs::read_dir(from) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let names = entries
        .map(|entry| entry.map(|e| e.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    if let Some(taken) = names.iter().find(|name| to.join(name).exists()) {
        return Err(Error::Other(format!(
            "{} is in both {} and {}",
            taken.to_string_lossy(),
            from.display(),
            to.display()
        )));
    }
    for name in &names {
        move_one(&from.join(name), &to.join(name))?;
    }
    fs::remove_dir(from)?;
    Ok(names
        .iter()
        .map(|name| name.to_string_lossy().into_owned())
        .collect())
}

// The active workspace's own directory — which, for the primary, IS the root
// (see `workspace::dir_of`). Everything belonging to one vault resolves through
// here, so switching workspaces moves the whole set of paths at once.
//
// Crate-visible for callers that need several of one workspace's files to be
// guaranteed to belong to the *same* workspace: resolving each path separately
// re-reads the active id each time, so a switch landing between two lookups
// hands back a mixed set (see `auth::recover_interrupted_rekey`).
pub(crate) fn workspace_dir(app: &AppHandle) -> Result<PathBuf> {
    Ok(crate::workspace::dir_of(
        &root_dir(app)?,
        &crate::workspace::active_id(app),
    ))
}

// The SQLCipher database that supersedes the JSON vault.
pub fn db_path(app: &AppHandle) -> Result<PathBuf> {
    Ok(workspace_dir(app)?.join(DB_FILE))
}

// Working space for the sync engine: the snapshot it packs for upload and the
// database it opens a pulled snapshot in. Inside the (dev-isolated) data dir so
// a debug build never writes scratch beside the real vault, and so the files
// inherit the 0700 directory mode. Everything written here is SQLCipher
// ciphertext, and every writer removes its own files (see `pack::Scratch`).
pub fn sync_scratch_dir(app: &AppHandle) -> Result<PathBuf> {
    Ok(workspace_dir(app)?.join(SYNC_SCRATCH_DIR))
}

// Favicons used to be cached as loose `{host}.uri` / `{host}.miss` files here.
// The file *names* were the vault's host list in the clear — every site the
// user has an account at, including ones they had deleted — sitting beside the
// encrypted database for any file-level backup to pick up. The cache now lives
// in a `favicons` table inside the SQLCipher vault, so this directory is only
// ever leftovers: removed whole, once, at startup (`lib.rs`). At startup
// rather than at the first lookup, because the list is on disk whether or not
// this install ever looks an icon up again — an empty vault, a workspace whose
// rows have no hosts, every host rejected — and a cache that is never read is
// exactly the one that would keep it forever. Best effort: a directory we
// could not remove is logged, not fatal, and is tried again next launch.
pub fn remove_legacy_icons_dir(app: &AppHandle) {
    let Ok(dir) = root_dir(app).map(|root| root.join("icons")) else {
        return;
    };
    if dir.exists() {
        if let Err(e) = fs::remove_dir_all(&dir) {
            log::warn!("could not remove the legacy favicon directory: {e}");
        }
    }
}

// Whether the SQLite store has been created (non-empty file present).
pub fn db_exists(app: &AppHandle) -> bool {
    db_path(app)
        .ok()
        .filter(|p| p.exists())
        .and_then(|p| fs::metadata(p).ok())
        .is_some_and(|m| m.len() > 0)
}

// Public to the crate because the sync token file's guarded transitions are
// built on a plain path (`sync::auth::TokenFile`) so they can be exercised
// without an `AppHandle`.
pub(crate) fn gdrive_path(app: &AppHandle) -> Result<PathBuf> {
    Ok(workspace_dir(app)?.join(GDRIVE_FILE))
}

// Public because the sync restore path addresses the sidecar by path (its core
// runs without an `AppHandle`) rather than duplicating the layout constants.
pub fn kdf_sidecar_path(app: &AppHandle) -> Result<PathBuf> {
    Ok(workspace_dir(app)?.join(KDF_SIDECAR_FILE))
}

// The KDF descriptor JSON, or `None` when absent (a sidecar-less legacy/dev DB).
pub fn read_kdf_sidecar(app: &AppHandle) -> Result<Option<String>> {
    let path = kdf_sidecar_path(app)?;
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(fs::read_to_string(path)?))
}

// Write (or overwrite) the KDF descriptor sidecar. The sidecar is the single
// source of truth for opening the vault, so the write is durable + atomic: a
// crash never leaves it truncated.
pub fn write_kdf_sidecar(app: &AppHandle, json: &str) -> Result<()> {
    atomic_write_file(&kdf_sidecar_path(app)?, json)
}

// Remove a SQLite database and its WAL/SHM siblings, ignoring whatever is
// already absent. Both callers (the pack scratch snapshot, the failed-restore
// rollback) must leave no *half* a database behind: a stale `-wal` beside a
// missing or recreated main file is its own corruption, not a clean slate.
pub fn remove_db_files(path: &Path) {
    for file in db_files(path) {
        let _ = fs::remove_file(file);
    }
}

// Every path one SQLite database occupies: the main file, then the WAL/SHM
// siblings written beside it. The main file alone is not the database — a
// `-wal` holds committed pages it does not have — so whatever moves or removes
// a database walks all three, main file first.
fn db_files(base: &Path) -> [PathBuf; 3] {
    let sibling = |suffix: &str| {
        let mut name = base.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name)
    };
    [base.to_path_buf(), sibling("-wal"), sibling("-shm")]
}

// Move a workspace's vault from one directory to another, whole: every file of
// the set that is there, each database with its WAL/SHM siblings. What is not
// there is skipped — a local vault has no token file, a cleanly closed database
// no WAL, and only an interrupted password change leaves a rekey pair.
//
// Every file is reconciled on its own: each is moved if it is at the source,
// never because some other file of the vault is. So a move a failure caught
// half way through — a database moved but its `-wal` not — is *finished* by
// running the same move again, and *undone* by running it the other way, which
// is what the delete's rollback and the recovery on the next launch (see
// `workspace::roll_back_delete`) do. Gating a database's siblings on its main
// file would strand a `-wal` holding committed pages at an address nothing
// looks at again, and the vault would reopen as the older one it was.
//
// Renames only, so the move is reversible until something removes the source:
// both callers (the promotion into the root, and the delete that stages a
// doomed workspace away) have to be able to put every file back.
pub fn move_vault_files(from: &Path, to: &Path) -> Result<()> {
    for file in VAULT_DB_FILES {
        let sources = db_files(&from.join(file));
        let targets = db_files(&to.join(file));
        for (source, target) in sources.iter().zip(&targets) {
            move_one(source, target)?;
        }
    }
    for file in VAULT_SIDECAR_FILES {
        move_one(&from.join(file), &to.join(file))?;
    }
    Ok(())
}

// A whole workspace directory's contents: its vault and what it keeps beside
// it. What a delete moves out of the way, so that a failure anywhere after it
// leaves the directory exactly as it was.
pub fn move_workspace_files(from: &Path, to: &Path) -> Result<()> {
    move_vault_files(from, to)?;
    for file in WORKSPACE_LOCAL_FILES {
        move_one(&from.join(file), &to.join(file))?;
    }
    // A directory, moved by the same rename as any file.
    move_one(&from.join(SYNC_SCRATCH_DIR), &to.join(SYNC_SCRATCH_DIR))
}

// One entry moved only if it is there, with the target's directory made first:
// the token file lives in a subdirectory of its own, which a root that never
// synced does not have, and a staging directory does not exist at all yet.
//
// An entry at *both* ends is refused rather than moved. A rename-based move
// cannot produce one — a file is at one address or the other, never both — so
// it means two vaults' files have met, and a rename would answer that by
// silently dropping the one at the target. Refusing keeps both on disk and
// leaves the delete journal for the next launch, which is recoverable; a
// discarded `-wal` is not.
fn move_one(source: &Path, target: &Path) -> Result<()> {
    if !source.exists() {
        return Ok(());
    }
    if target.exists() {
        return Err(Error::Other(format!(
            "cannot move {}: something is already at {}",
            source.display(),
            target.display()
        )));
    }
    if let Some(parent) = target.parent() {
        crate::store::create_private_dir(parent)?;
    }
    Ok(fs::rename(source, target)?)
}

// Preferences belong to the install, not to a vault, so they sit on the root
// and follow the user across workspaces.
pub fn settings_path(app: &AppHandle) -> Result<PathBuf> {
    Ok(root_dir(app)?.join(SETTINGS_FILE))
}

// Write (or overwrite) the preferences file. Atomic like the sidecars: a torn
// write here would boot the next launch on the defaults.
pub fn write_settings(app: &AppHandle, json: &str) -> Result<()> {
    atomic_write_file(&settings_path(app)?, json)
}

// The failed-unlock backoff state JSON, or `None` when absent (no failed
// attempts recorded yet, or a fresh vault).
//
// On a directory rather than on the app, because the backoff belongs to the
// workspace whose password is being proved, which is not always the active one:
// `workspace_delete` proves a locked workspace's password against its own files.
pub fn read_lockout_sidecar_in(dir: &Path) -> Result<Option<String>> {
    let path = dir.join(LOCKOUT_SIDECAR_FILE);
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(fs::read_to_string(path)?))
}

// Write (or overwrite) the backoff state sidecar, atomically (same durability
// rationale as the KDF sidecar: never leave a torn/partial file behind).
pub fn write_lockout_sidecar_in(dir: &Path, json: &str) -> Result<()> {
    atomic_write_file(&dir.join(LOCKOUT_SIDECAR_FILE), json)
}

// The same pair for the workspace that is open, which is what an unlock asks.
pub fn read_lockout_sidecar(app: &AppHandle) -> Result<Option<String>> {
    read_lockout_sidecar_in(&workspace_dir(app)?)
}

pub fn write_lockout_sidecar(app: &AppHandle, json: &str) -> Result<()> {
    write_lockout_sidecar_in(&workspace_dir(app)?, json)
}

// Durably replace `path`: create a uniquely named temp sibling, fsync it,
// atomically rename it over the target, then fsync the directory. The target
// ends up as either the complete old bytes or the complete new bytes — never a
// truncated/empty file. `write` is injected so failure after a partial temp
// write is testable; the partial sibling is removed on every failure.
//
// `private` makes the temp sibling owner-readable from the instant it exists
// (`owner_only::create_new`: `0600` on Unix, a protected DACL supplied at
// creation on Windows), so there is no moment at which the umask or the
// folder's inherited permissions govern it — a handle opened in such a moment
// would keep its access after the permissions changed — and the replaced file
// keeps that restriction whatever an existing file at `path` allowed.
fn atomic_replace_with<F>(path: &Path, private: bool, write: F) -> Result<()>
where
    F: FnOnce(&mut fs::File) -> std::io::Result<()>,
{
    let parent = path
        .parent()
        .ok_or_else(|| Error::Other("destination has no parent directory".into()))?;
    fs::create_dir_all(parent)?;

    let mut staged = Staged::from(create_temp_sibling(path, private)?);
    write(staged.file())?;
    staged.file().sync_all()?;
    staged.close();
    fs::rename(&staged.path, path)?;
    staged.keep();

    // Persist the directory entry for the rename where the platform supports it
    // (opening a directory as a file fails on Windows — best-effort there).
    if let Ok(dir) = fs::File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

// A temp sibling that is removed unless the replacement it was staged for goes
// through. In `Drop` so every early exit — a failed write, sync or rename —
// takes it with it. It owns the open handle too, and closes it *before* the
// removal: on Windows the private writer opens the file with no delete
// sharing, so a removal attempted while the handle is still open would fail
// and leave the partial plaintext behind. Two separate locals would drop in
// the wrong order for that (last declared, first dropped).
struct Staged {
    path: PathBuf,
    file: Option<fs::File>,
    remove: bool,
}

impl From<(PathBuf, fs::File)> for Staged {
    fn from((path, file): (PathBuf, fs::File)) -> Self {
        Self {
            path,
            file: Some(file),
            remove: true,
        }
    }
}

impl Staged {
    fn file(&mut self) -> &mut fs::File {
        self.file
            .as_mut()
            .expect("closed only once, before the rename")
    }

    // Release the handle so the file can be renamed (and, on failure, removed).
    fn close(&mut self) {
        self.file.take();
    }

    fn keep(mut self) {
        self.remove = false;
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        self.close();
        if self.remove {
            let _ = fs::remove_file(&self.path);
        }
    }
}

// A fresh sibling of `path` — `<name>.<random>.tmp` beside it — created for
// writing, and never over an existing file. Owner-only from creation when
// `private`; otherwise a plain file that keeps the `0600` these temp files
// have always had on Unix.
fn create_temp_sibling(path: &Path, private: bool) -> Result<(PathBuf, fs::File)> {
    let name = path
        .file_name()
        .ok_or_else(|| Error::Other("destination has no file name".into()))?;
    for _ in 0..8 {
        let mut candidate = name.to_os_string();
        candidate.push(format!(".{:016x}.tmp", rand::random::<u64>()));
        let candidate = path.with_file_name(candidate);
        let created = if private {
            crate::owner_only::create_new(&candidate)
        } else {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options.open(&candidate)
        };
        match created {
            Ok(file) => return Ok((candidate, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(Error::Other(
        "could not find a free name for a temporary sibling".into(),
    ))
}

/// Atomically write the UTF-8 sidecars that gate vault opening and lockout.
pub fn atomic_write_file(path: &Path, data: &str) -> Result<()> {
    atomic_replace_with(path, false, |file| file.write_all(data.as_bytes()))
}

/// Atomically replace a secret with one only its owner can read: `0600` on
/// Unix, an owner-and-SYSTEM protected DACL on Windows — regardless of the
/// umask, of the folder's inheritable permissions, or of how an existing file
/// at `path` was permissioned.
pub fn atomic_write_private(path: &Path, data: &[u8]) -> Result<()> {
    atomic_replace_with(path, true, |file| file.write_all(data))
}

// Read a file as utf8, returning "" when it doesn't exist (legacy ensure-file).
pub(crate) fn read_file(path: &Path) -> Result<String> {
    if !path.exists() {
        return Ok(String::new());
    }
    Ok(fs::read_to_string(path)?)
}

/// Read a whole file the user pointed at, refusing anything past `cap`.
///
/// Every decision here is made on one open handle, and there is no `stat`
/// before it: a `stat` and the open that follows are two different files
/// whenever the path is swapped in between, so anything the first one settled
/// the second would have to settle again. The handle is the one thing that
/// cannot be exchanged under us. What it has to say is that this is a regular
/// file: a FIFO reports a length of zero and then serves bytes for as long as a
/// writer feels like it, and a device serves them without end.
///
/// `take(cap + 1)` is what bounds the read. The extra byte is how a file at
/// the cap is told from one past it without trusting any reported length — if
/// it arrives, the file is too large and nothing beyond it is ever read.
pub fn read_regular_file_capped(path: &Path, cap: u64) -> Result<Vec<u8>> {
    let file = open_without_blocking(path)?;
    if !file.metadata()?.file_type().is_file() {
        return Err(not_a_regular_file());
    }

    let mut bytes = Vec::new();
    file.take(cap.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > cap {
        return Err(Error::FileTooLarge);
    }
    Ok(bytes)
}

/// Open for reading without letting the open itself block: `O_NONBLOCK` is what
/// makes opening a writer-less FIFO return a handle instead of parking until
/// some writer turns up, which is what lets the check above be made on the
/// handle rather than guessed at from a `stat` beforehand. On a regular file the
/// flag means nothing — it governs the open and, afterwards, reads of the very
/// things this refuses.
#[cfg(unix)]
fn open_without_blocking(path: &Path) -> Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    Ok(fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?)
}

/// No such flag off Unix, and no FIFO at a user-picked path to need it: a
/// Windows named pipe is addressed as `\\.\pipe\...`, not as a file in a folder.
#[cfg(not(unix))]
fn open_without_blocking(path: &Path) -> Result<fs::File> {
    Ok(fs::File::open(path)?)
}

fn not_a_regular_file() -> Error {
    Error::Other("that path is not a regular file".into())
}

// What the importer can afford to hold, not what the old app could write. The
// import decodes the file one layer at a time and frees each layer as the next
// is built from it, so the peak is the file plus the base64 output beside it
// for that one step — about 1.75x the file — and every step after that is
// smaller. The app ships on iOS, where a foreground process on a 3 GB device is
// killed around 1.3 GiB resident (an approximate, device-dependent figure), so
// 256 MiB peaks at well under half a gigabyte and stays far inside that with
// the app's own baseline on top.
//
// It turns away no real backup. A vault is text only — there are no
// attachments — and a typical entry is a few hundred bytes of fields, about a
// kilobyte once hex-encoded and wrapped in base64, so 256 MiB is on the order
// of a quarter of a million entries. (Nothing the legacy Electron app wrote
// came near even the old limit: the whole hex backup was one V8 string, and
// those stop just under 1 GiB.)
//
// The cap also has to exist at all so that a file which is not a backup of ours
// cannot be buffered and decoded before the AES-GCM tag — checked at the very
// end of all of it — gets to reject it.
const MAX_BACKUP_BYTES: u64 = 256 * 1024 * 1024;

// Read an arbitrary backup file chosen by the user (absolute path).
pub fn read_backup(path: &str) -> Result<String> {
    let bytes = read_regular_file_capped(Path::new(path), MAX_BACKUP_BYTES)?;
    String::from_utf8(bytes).map_err(|_| Error::FileNotText)
}

// The sealed Drive tokens, into a workspace directory named outright rather
// than the active one: for a workspace being made *beside* the open one, whose
// paths must stay where they are (`commands::autojoin`). The active
// workspace's own token file is read and written by `sync::auth::TokenFile`,
// which holds its path.
//
// Atomic, so a crash mid-write leaves the previous grant rather than a
// truncated file that reads as "not connected" — and owner-only from creation,
// like every other secret the app writes: the seal is the real protection, the
// mode is the second line.
pub fn write_gdrive_in(dir: &Path, data: &str) -> Result<()> {
    atomic_write_private(&dir.join(GDRIVE_FILE), data.as_bytes())
}

// Remove the token file, whatever state a failed write left it in. "No file" is
// the goal, so an already-absent one is success — but every other failure is
// reported rather than swallowed: a token file that outlives a disconnect is a
// live refresh token, and a caller told the delete worked would never know.
pub fn remove_gdrive(app: &AppHandle) -> Result<()> {
    remove_if_present(&gdrive_path(app)?)
}

// The file-level half, on a plain path so it is testable without an `AppHandle`.
pub(crate) fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

// The recorded gate marker, or `None` when biometric unlock is not enabled.
// Enrollment decides the gate once; every later retrieval reads it back from
// here rather than re-probing what the platform would do today.
//
// In the root, whichever workspace is active: the enrolled key is the app key
// (the primary's), and it opens every workspace (`crate::appkey`), so the
// enrollment belongs to the install rather than to one workspace.
pub fn biometric_marker(app: &AppHandle) -> Option<String> {
    let path = root_dir(app).ok()?.join(BIOMETRIC_FILE);
    fs::read_to_string(path).ok()
}

// Whether the user opted into biometric unlock (marker file present).
pub fn biometric_enrolled(app: &AppHandle) -> bool {
    root_dir(app)
        .map(|d| d.join(BIOMETRIC_FILE).exists())
        .unwrap_or(false)
}

// Record the enrolled gate, or clear the marker entirely with `None`. Idempotent.
// Atomic like the sidecars: a torn marker would read as the legacy gate
// (`GateMode::from_marker`) and send the next unlock through the wrong one.
pub fn set_biometric_marker(app: &AppHandle, marker: Option<&str>) -> Result<()> {
    let path = root_dir(app)?.join(BIOMETRIC_FILE);
    match marker {
        Some(marker) => atomic_write_file(&path, marker),
        None if path.exists() => {
            fs::remove_file(&path)?;
            Ok(())
        }
        None => Ok(()),
    }
}

pub fn sync_configured(app: &AppHandle) -> bool {
    workspace_dir(app).is_ok_and(|dir| sync_configured_in(&dir))
}

/// Whether the workspace at `dir` holds a Drive connection: its token file is
/// there and non-empty. The one fact about a *locked* workspace's sync that is
/// readable from outside it — the registry's vault id says a pack exists on
/// Drive, which outlives a disconnect, not that anything still syncs to it.
pub fn sync_configured_in(dir: &Path) -> bool {
    // A single metadata call answers both "exists" and "non-empty".
    fs::metadata(dir.join(GDRIVE_FILE)).is_ok_and(|m| m.len() > 0)
}

#[cfg(test)]
mod tests {
    use super::{
        atomic_replace_with, atomic_write_file, move_data_dir, move_vault_files,
        move_workspace_files, read_backup, read_regular_file_capped, remove_if_present, Error,
        BIOMETRIC_FILE, DB_FILE, DB_REKEY_BACKUP_FILE, GDRIVE_FILE, KDF_SIDECAR_FILE,
        KDF_SIDECAR_REKEY_BACKUP_FILE, LOCKOUT_SIDECAR_FILE, SYNC_SCRATCH_DIR, WRAPPED_KEY_FILE,
    };
    use std::fs;
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    // What SQLite writes beside `DB_FILE`, as the tests have to name it.
    const WAL_FILE: &str = "vault.db-wal";

    fn tmp_sidecar() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rowel-storage-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir.join("vault.kdf.json")
    }

    fn tmp_sibling(path: &Path) -> PathBuf {
        let mut tmp = path.to_path_buf().into_os_string();
        tmp.push(".tmp");
        PathBuf::from(tmp)
    }

    #[test]
    fn a_file_past_the_cap_is_refused_without_being_read_whole() {
        let path = tmp_sidecar().with_file_name("huge.swftx");
        // Sized, not written: a sparse 64 MiB file against a 16-byte cap, so a
        // pass that buffered it whole would be doing something this one does not.
        let file = fs::File::create(&path).unwrap();
        file.set_len(64 * 1024 * 1024).unwrap();
        drop(file);

        let err = read_regular_file_capped(&path, 16).unwrap_err();
        assert!(matches!(err, Error::FileTooLarge), "{err}");
    }

    #[test]
    fn a_file_exactly_at_the_cap_is_accepted() {
        let path = tmp_sidecar().with_file_name("exact.swftx");
        fs::write(&path, "deadbeef").unwrap();
        assert_eq!(read_regular_file_capped(&path, 8).unwrap(), b"deadbeef");
    }

    #[test]
    fn a_directory_is_not_a_file_to_read() {
        let dir = tmp_sidecar().parent().unwrap().to_path_buf();
        let err = read_regular_file_capped(&dir, 1024).unwrap_err();
        assert!(matches!(err, Error::Other(_) | Error::Io(_)), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_is_refused_rather_than_opened() {
        let path = tmp_sidecar().with_file_name("pipe.swftx");
        let made = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap();
        assert!(made.success());

        // Nothing is writing to this pipe, so a blocking open would park here
        // until something did rather than fail — the test finishing is half of
        // what it asserts, and `O_NONBLOCK` is what makes it finish.
        let err = read_regular_file_capped(&path, 1024).unwrap_err();
        assert!(matches!(err, Error::Other(_)), "{err}");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_backup_under_the_cap_is_read_whole() {
        let path = tmp_sidecar().with_file_name("small.swftx");
        fs::write(&path, "deadbeef").unwrap();
        assert_eq!(read_backup(path.to_str().unwrap()).unwrap(), "deadbeef");
    }

    #[test]
    fn atomic_write_round_trips_and_overwrites() {
        let path = tmp_sidecar();
        atomic_write_file(&path, "{\"algo\":\"argon2id\"}").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\"algo\":\"argon2id\"}"
        );

        // Overwrite in place with shorter content — the target is fully replaced.
        atomic_write_file(&path, "{}").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "{}");

        // A completed write leaves no temp file behind.
        assert!(!tmp_sibling(&path).exists());
    }

    #[test]
    fn lockout_sidecar_round_trips_through_the_atomic_writer() {
        // Same primitive as the KDF sidecar, exercised with the lockout shape.
        let path = tmp_sidecar().with_file_name("vault.lock.json");
        let json = "{\"failed_attempts\":4,\"locked_until_ms\":1700000002000}";
        atomic_write_file(&path, json).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), json);
    }

    #[test]
    fn leftover_temp_file_does_not_affect_the_target() {
        let path = tmp_sidecar();
        atomic_write_file(&path, "real").unwrap();

        // Simulate a crash before a prior rename: a stale, partial temp sibling.
        fs::write(tmp_sibling(&path), "garbage-partial").unwrap();

        // Reading the sidecar (the target path) is unaffected by the temp file.
        assert_eq!(fs::read_to_string(&path).unwrap(), "real");
    }

    #[test]
    fn a_failed_replacement_keeps_the_complete_old_file() {
        let path = tmp_sidecar();
        fs::write(&path, "complete old bytes").unwrap();

        let result = atomic_replace_with(&path, true, |temp| {
            temp.write_all(b"partial new bytes")?;
            Err(io::Error::new(io::ErrorKind::StorageFull, "disk full"))
        });

        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "complete old bytes");
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    // Every file of the set, laid out as a workspace holds them.
    fn seed_vault(dir: &Path) -> [&'static str; 7] {
        let files = [
            DB_FILE,
            DB_REKEY_BACKUP_FILE,
            KDF_SIDECAR_FILE,
            KDF_SIDECAR_REKEY_BACKUP_FILE,
            LOCKOUT_SIDECAR_FILE,
            GDRIVE_FILE,
            WRAPPED_KEY_FILE,
        ];
        fs::create_dir_all(dir.join("auth")).unwrap();
        for file in files {
            fs::write(dir.join(file), file).unwrap();
        }
        fs::write(dir.join(WAL_FILE), "wal").unwrap();
        files
    }

    // The set is this module's answer to "what a vault is", and a move has to
    // take all of it: the database with its WAL, the pair an interrupted
    // password change left behind, the backoff state, the sealed account and
    // the key sealed under the app key.
    #[test]
    fn a_moved_vault_takes_every_file_it_is_made_of() {
        let from = tmp_sidecar().parent().unwrap().to_path_buf();
        let to = from.join("moved");
        let files = seed_vault(&from);
        fs::write(from.join(BIOMETRIC_FILE), "protected").unwrap();
        fs::create_dir_all(from.join(SYNC_SCRATCH_DIR)).unwrap();

        move_vault_files(&from, &to).unwrap();

        for file in files {
            assert_eq!(fs::read_to_string(to.join(file)).unwrap(), file);
            assert!(!from.join(file).exists(), "{file} was left behind");
        }
        assert_eq!(fs::read_to_string(to.join(WAL_FILE)).unwrap(), "wal");
        // What is not the vault stays where it is.
        assert!(from.join(BIOMETRIC_FILE).exists());
        assert!(from.join(SYNC_SCRATCH_DIR).exists());
    }

    // The whole directory's contents, which is what a delete has to be able to
    // put back — the marker and the scratch included.
    #[test]
    fn a_moved_workspace_takes_what_sits_beside_its_vault_too() {
        let from = tmp_sidecar().parent().unwrap().to_path_buf();
        let to = from.join("moved");
        seed_vault(&from);
        fs::write(from.join(BIOMETRIC_FILE), "protected").unwrap();
        fs::create_dir_all(from.join(SYNC_SCRATCH_DIR)).unwrap();

        move_workspace_files(&from, &to).unwrap();

        assert_eq!(
            fs::read_to_string(to.join(BIOMETRIC_FILE)).unwrap(),
            "protected"
        );
        assert!(to.join(SYNC_SCRATCH_DIR).is_dir());
        assert!(!from.join(BIOMETRIC_FILE).exists());
        assert!(!from.join(SYNC_SCRATCH_DIR).exists());
    }

    // A local vault has no token file and a cleanly closed one no WAL: a move
    // takes what is there and does not fail over what is not.
    #[test]
    fn moving_a_vault_skips_the_files_it_does_not_have() {
        let from = tmp_sidecar().parent().unwrap().to_path_buf();
        let to = from.join("moved");
        fs::write(from.join(DB_FILE), "db").unwrap();
        fs::write(from.join(KDF_SIDECAR_FILE), "kdf").unwrap();

        move_vault_files(&from, &to).unwrap();

        assert_eq!(fs::read_to_string(to.join(DB_FILE)).unwrap(), "db");
        assert!(!to.join(WAL_FILE).exists());
        assert!(!to.join(GDRIVE_FILE).exists());
    }

    // A move stopped between a database and its WAL is finished by running the
    // same move again: the main file is already at the target, and the retry
    // has to carry the `-wal` after it rather than skip a bundle whose main
    // file it cannot see at the source. Left behind, that `-wal` holds
    // committed pages the moved database does not, and the vault reopens as
    // the older one it was.
    #[test]
    fn a_move_interrupted_between_a_database_and_its_wal_is_finished_by_the_next_one() {
        let from = tmp_sidecar().parent().unwrap().to_path_buf();
        let to = from.join("moved");
        fs::create_dir_all(&to).unwrap();
        fs::write(to.join(DB_FILE), "db").unwrap();
        fs::write(from.join(WAL_FILE), "wal").unwrap();
        fs::write(from.join(KDF_SIDECAR_FILE), "kdf").unwrap();

        move_vault_files(&from, &to).unwrap();

        assert_eq!(fs::read_to_string(to.join(WAL_FILE)).unwrap(), "wal");
        assert_eq!(
            fs::read_to_string(to.join(KDF_SIDECAR_FILE)).unwrap(),
            "kdf"
        );
        assert!(!from.join(WAL_FILE).exists());
    }

    // The other direction of the same property: what the move put at the
    // target, the opposite move puts back — database and `-wal` together,
    // which is what the delete's rollback needs of it.
    #[test]
    fn the_opposite_move_puts_a_database_and_its_wal_back() {
        let from = tmp_sidecar().parent().unwrap().to_path_buf();
        let to = from.join("moved");
        fs::write(from.join(DB_FILE), "db").unwrap();
        fs::write(from.join(WAL_FILE), "wal").unwrap();

        move_vault_files(&from, &to).unwrap();
        move_vault_files(&to, &from).unwrap();

        assert_eq!(fs::read_to_string(from.join(DB_FILE)).unwrap(), "db");
        assert_eq!(fs::read_to_string(from.join(WAL_FILE)).unwrap(), "wal");
        assert!(!to.join(DB_FILE).exists());
        assert!(!to.join(WAL_FILE).exists());
    }

    // A rename-based move never leaves a file at both ends, so one that is at
    // both is two vaults' files meeting. Neither is ours to discard: the move
    // fails with both still on disk.
    #[test]
    fn a_file_at_both_ends_is_refused_rather_than_written_over() {
        let from = tmp_sidecar().parent().unwrap().to_path_buf();
        let to = from.join("moved");
        fs::create_dir_all(&to).unwrap();
        fs::write(from.join(DB_FILE), "ours").unwrap();
        fs::write(to.join(DB_FILE), "theirs").unwrap();

        let err = move_vault_files(&from, &to).unwrap_err();

        assert!(matches!(err, Error::Other(_)), "{err}");
        assert_eq!(fs::read_to_string(from.join(DB_FILE)).unwrap(), "ours");
        assert_eq!(fs::read_to_string(to.join(DB_FILE)).unwrap(), "theirs");
    }

    // The delete behind a Drive disconnect: gone is the goal, so already-gone is
    // success — but a delete that actually failed must not read as one, or a
    // caller would report a disconnect over a token file that is still there.
    // An install from before the App Group: the whole data dir, registry and
    // preferences and every workspace, lands in the container as it was laid
    // out, and the old directory is gone.
    #[test]
    fn the_data_dir_moves_whole_into_an_empty_container() {
        let root = tmp_sidecar().parent().unwrap().to_path_buf();
        let (from, to) = (root.join("app-data"), root.join("group/app.rowel.mobile"));
        fs::create_dir_all(from.join("workspaces/w1")).unwrap();
        fs::write(from.join(DB_FILE), "db").unwrap();
        fs::write(from.join("workspaces.json"), "{}").unwrap();
        fs::write(from.join("settings.json"), "{}").unwrap();
        fs::write(from.join("workspaces/w1").join(DB_FILE), "w1").unwrap();

        let mut moved = move_data_dir(&from, &to).unwrap();
        moved.sort();

        assert_eq!(
            moved,
            ["settings.json", DB_FILE, "workspaces", "workspaces.json"]
        );
        assert!(!from.exists());
        assert_eq!(fs::read_to_string(to.join(DB_FILE)).unwrap(), "db");
        assert_eq!(
            fs::read_to_string(to.join("workspaces/w1").join(DB_FILE)).unwrap(),
            "w1"
        );
    }

    // A crash between two renames: some entries are in the container already,
    // the rest still in the old directory. The next launch finishes the move
    // rather than taking the container for a vault of its own.
    #[test]
    fn a_half_moved_data_dir_is_finished_on_the_next_run() {
        let root = tmp_sidecar().parent().unwrap().to_path_buf();
        let (from, to) = (root.join("app-data"), root.join("group"));
        fs::create_dir_all(&from).unwrap();
        fs::create_dir_all(&to).unwrap();
        fs::write(to.join(DB_FILE), "db").unwrap();
        fs::write(from.join(KDF_SIDECAR_FILE), "kdf").unwrap();

        assert_eq!(move_data_dir(&from, &to).unwrap(), [KDF_SIDECAR_FILE]);
        assert!(!from.exists());
        assert_eq!(fs::read_to_string(to.join(DB_FILE)).unwrap(), "db");
        assert_eq!(
            fs::read_to_string(to.join(KDF_SIDECAR_FILE)).unwrap(),
            "kdf"
        );
    }

    // The crash after the last rename, before the old directory went: an empty
    // directory is all that is left, and it goes.
    #[test]
    fn an_emptied_data_dir_is_removed() {
        let root = tmp_sidecar().parent().unwrap().to_path_buf();
        let (from, to) = (root.join("app-data"), root.join("group"));
        fs::create_dir_all(&from).unwrap();

        assert!(move_data_dir(&from, &to).unwrap().is_empty());
        assert!(!from.exists());
    }

    // Already moved, or a fresh install: nothing to do.
    #[test]
    fn no_old_data_dir_moves_nothing() {
        let root = tmp_sidecar().parent().unwrap().to_path_buf();
        let (from, to) = (root.join("app-data"), root.join("group"));

        assert!(move_data_dir(&from, &to).unwrap().is_empty());
        assert!(!to.exists());
    }

    // Two vaults, one in each place: nothing moves — not even the entries that
    // do not collide, which would pair one vault's sidecars with the other's
    // database — and both stay as they were.
    #[test]
    fn a_container_with_a_vault_of_its_own_is_never_written_over() {
        let root = tmp_sidecar().parent().unwrap().to_path_buf();
        let (from, to) = (root.join("app-data"), root.join("group"));
        fs::create_dir_all(&from).unwrap();
        fs::create_dir_all(&to).unwrap();
        fs::write(from.join(DB_FILE), "ours").unwrap();
        fs::write(from.join(KDF_SIDECAR_FILE), "our kdf").unwrap();
        fs::write(to.join(DB_FILE), "theirs").unwrap();

        let err = move_data_dir(&from, &to).unwrap_err();

        assert!(matches!(err, Error::Other(_)), "{err}");
        assert_eq!(fs::read_to_string(from.join(DB_FILE)).unwrap(), "ours");
        assert!(from.join(KDF_SIDECAR_FILE).exists());
        assert_eq!(fs::read_to_string(to.join(DB_FILE)).unwrap(), "theirs");
        assert!(!to.join(KDF_SIDECAR_FILE).exists());
    }

    #[test]
    fn removing_an_absent_file_succeeds_and_a_present_one_goes() {
        let path = tmp_sidecar().with_file_name("gdrive.swftx");
        remove_if_present(&path).unwrap();

        fs::write(&path, "sealed tokens").unwrap();
        remove_if_present(&path).unwrap();
        assert!(!path.exists());
    }
}
