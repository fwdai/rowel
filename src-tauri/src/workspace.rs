//! Optional additional vaults ("workspaces").
//!
//! A workspace is a whole encrypted database with its own master password. The
//! capability is opt-in and costs an untouched install nothing: the primary
//! workspace *is* the existing data dir, so a user who never creates a second
//! one has no extra directory and no `workspaces.json` on disk. Only the
//! registry — which workspaces exist and which one is current — lives outside
//! any vault, and it holds nothing secret: ids and user-chosen labels.
//!
//! Exactly one workspace is unlocked at a time. Switching locks the current one
//! and marks another active; the next unlock opens that one's database.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};
use crate::state::AppState;
use crate::storage;

// The registry itself, and where each workspace's files are, are in the core:
// the iOS AutoFill extension reads them too.
pub use rowel_core::workspace::{dir_of, Deletion, Registry, Workspace, PRIMARY_ID};

/// A registry entry as the frontend is handed it: the record, plus what is
/// true of the workspace right now and is not the registry's to keep.
///
/// `synced` is read off the workspace's own token file at the moment of
/// asking (see [`crate::storage::sync_configured_in`]). It is not `vault_id`:
/// that says a pack for the vault exists on Drive, and a disconnect leaves the
/// pack there, so a workspace disconnected and then locked would otherwise
/// keep reading as one that syncs — and be offered a delete "everywhere" it
/// has no credentials to carry out.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceStatus {
    #[serde(flatten)]
    pub workspace: Workspace,
    pub synced: bool,
}

/// Every workspace in `registry` with its live sync state, for `app_status`.
pub fn statuses(registry: &Registry, root: &Path) -> Vec<WorkspaceStatus> {
    registry
        .workspaces
        .iter()
        .map(|workspace| WorkspaceStatus {
            synced: storage::sync_configured_in(&dir_of(root, &workspace.id)),
            workspace: workspace.clone(),
        })
        .collect()
}

/// Where a delete parks files until it is safe to remove them, under the root.
const STAGING_DIR: &str = "deleting";

/// The record a delete leaves while the files and the registry are out of step,
/// beside the registry it is about.
const DELETE_JOURNAL_FILE: &str = "delete.json";

/// What a delete was doing when it was interrupted.
///
/// The registry is what says a vault exists at all, so the order of a delete is
/// forced: nothing may be removed until the registry that no longer names it
/// has been saved. Otherwise a failure in between leaves the file on disk
/// naming a workspace whose files are gone — and a retry cannot repair that,
/// because proving the password needs the database that went.
///
/// So every step before the save is a rename into `deleting/`, and this says
/// which workspace was going and which was promoted into its place. It is read
/// once per launch (see [`recover_interrupted_delete`]).
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteJournal {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    promoted: Option<String>,
}

impl DeleteJournal {
    fn path(root: &Path) -> PathBuf {
        root.join(DELETE_JOURNAL_FILE)
    }

    /// Atomic, like the registry it shadows: a torn journal is one the recovery
    /// below could not act on.
    fn write(&self, root: &Path) -> Result<()> {
        storage::atomic_write_file(&Self::path(root), &serde_json::to_string(self)?)
    }

    /// `None` for every launch but the interrupted one, for the cost of the
    /// single read. A journal that will not parse is left where it is: there is
    /// nothing to act on, and removing it would throw away the one record that
    /// says a delete was in flight.
    fn read(root: &Path) -> Option<Self> {
        let json = fs::read_to_string(Self::path(root)).ok()?;
        match serde_json::from_str(&json) {
            Ok(journal) => Some(journal),
            Err(e) => {
                log::warn!("cannot read the interrupted-delete journal: {e}");
                None
            }
        }
    }

    fn remove(root: &Path) -> Result<()> {
        storage::remove_if_present(&Self::path(root))
    }
}

fn staging(root: &Path) -> PathBuf {
    root.join(STAGING_DIR)
}

/// Carry out on disk the deletion [`Registry::without`] decided.
///
/// The doomed workspace's files are staged out of the way, the survivor — when
/// the primary is the one going — is moved into the root, the registry is
/// saved, and only then is anything removed. Every step before the save is a
/// rename, so a failure puts all of them back and reports it: the user's old
/// layout is intact and a retry can prove the password again.
pub fn apply_deletion(root: &Path, id: &str, deletion: &Deletion) -> Result<()> {
    // A journal that is still here is one an earlier delete could not undo and
    // the last launch could not resolve. Starting a second delete over it would
    // replace the only record of where the first one's files went, so this one
    // waits for the launch that clears it.
    if DeleteJournal::path(root).exists() {
        return Err(Error::Other(unfinished_delete_error()));
    }
    let journal = DeleteJournal {
        id: id.to_string(),
        promoted: deletion.promoted.clone(),
    };
    journal.write(root)?;
    match stage_and_save(root, &journal, &deletion.registry) {
        Ok(()) => {
            finish_delete(root, &journal);
            Ok(())
        }
        Err(e) => {
            if let Err(undone) = roll_back_delete(root, &journal) {
                // The journal stays behind, so the next launch tries again.
                log::error!("could not undo a workspace delete that failed: {undone}");
            }
            Err(e)
        }
    }
}

// Surfaced verbatim where the delete was asked for, so it has to read as a
// sentence.
fn unfinished_delete_error() -> String {
    "an earlier workspace delete on this device has not been finished; restart the app and try \
     again"
        .into()
}

// The moves, and the registry write that commits them. Nothing here removes
// anything.
fn stage_and_save(root: &Path, journal: &DeleteJournal, registry: &Registry) -> Result<()> {
    stage_out(root, &journal.id)?;
    if let Some(promoted) = &journal.promoted {
        // Into the root, where the next unlock looks for the primary's vault:
        // `dir_of` resolves `PRIMARY_ID` to the root itself, so a promotion is
        // a move of files, not a change of paths.
        storage::move_vault_files(&dir_of(root, promoted), root)?;
    }
    registry.save(root)
}

// Everything the doomed workspace owns, out of the root's way.
//
// A workspace with a directory of its own is one rename, which is all-or-
// nothing already. The primary's files share the root with the registry and
// every other workspace, so they go one by one into a partial name that is
// renamed into place once they are all there — as the rekey snapshot publishes
// itself (`auth::publish_snapshot`), and for the same reason: `deleting/<id>`
// exists only if the staging finished, which is what tells a rollback whether
// what is left in the root is the promoted workspace's or still the deleted
// one's.
fn stage_out(root: &Path, id: &str) -> Result<()> {
    let staged = staging(root).join(id);
    let from = dir_of(root, id);
    if id != PRIMARY_ID {
        crate::store::create_private_dir(&staging(root))?;
        return Ok(fs::rename(&from, &staged)?);
    }
    let partial = staging(root).join(format!("{id}.partial"));
    crate::store::create_private_dir(&partial)?;
    storage::move_workspace_files(&from, &partial)?;
    Ok(fs::rename(&partial, &staged)?)
}

// Put every move back, in the order the staging can be told apart: the
// promotion first, so the root is clear of the survivor's files before the
// staged ones come back on top of them — and only once `deleting/<id>` says the
// staging finished, since until then what is in the root is the deleted
// workspace's own.
fn roll_back_delete(root: &Path, journal: &DeleteJournal) -> Result<()> {
    let staged = staging(root).join(&journal.id);
    let back = dir_of(root, &journal.id);
    if staged.exists() {
        if let Some(promoted) = &journal.promoted {
            storage::move_vault_files(root, &dir_of(root, promoted))?;
        }
        if journal.id == PRIMARY_ID {
            storage::move_workspace_files(&staged, &back)?;
        } else {
            crate::store::create_private_dir(back.parent().unwrap_or(root))?;
            fs::rename(&staged, &back)?;
        }
    } else {
        // The staging never finished: its partial name holds whatever did move,
        // and nothing was promoted over it.
        let partial = staging(root).join(format!("{}.partial", journal.id));
        storage::move_workspace_files(&partial, &back)?;
    }
    let _ = fs::remove_dir_all(staging(root));
    DeleteJournal::remove(root)
}

// The delete is committed — the registry no longer names the workspace, so what
// is staged is beyond anyone's reach. Best effort: leftovers cost nothing, and
// the journal goes last so a failure here is one the next launch finishes.
fn finish_delete(root: &Path, journal: &DeleteJournal) {
    let _ = fs::remove_dir_all(staging(root));
    if let Some(promoted) = &journal.promoted {
        // Emptied by the promotion; what the vault did not take with it (the
        // biometric marker, a sync run's scratch) goes with the directory.
        let _ = fs::remove_dir_all(dir_of(root, promoted));
    }
    if let Err(e) = DeleteJournal::remove(root) {
        log::warn!("could not retire the delete journal: {e}");
    }
}

/// Finish or undo a workspace delete that a crash caught between its file moves
/// and the registry write. Called once per launch, before anything reads the
/// registry or opens a vault.
///
/// The registry on disk is what says whether the delete was committed: the old
/// one still names the workspace that went — under its own id, or, when the
/// primary went, under the id the promotion gave up. So one lookup decides
/// between putting the moves back and clearing away what they left.
pub fn recover_interrupted_delete(root: &Path) {
    let Some(journal) = DeleteJournal::read(root) else {
        return;
    };
    let named = journal.promoted.as_deref().unwrap_or(&journal.id);
    if Registry::load(root).contains(named) {
        match roll_back_delete(root, &journal) {
            Ok(()) => log::warn!("undid an interrupted workspace delete; nothing was removed"),
            // Kept for the next launch rather than reported: there is no vault
            // open yet, and no user to tell.
            Err(e) => log::error!("could not undo an interrupted workspace delete: {e}"),
        }
        return;
    }
    log::warn!("clearing up after a workspace delete that was interrupted once committed");
    finish_delete(root, &journal);
}

/// Remember which vault the active workspace holds, once a sync has settled it.
///
/// Called from inside a sync run, at the moment the id is proved to be this
/// vault's (see `sync::run`), so the registry learns of every vault that has a
/// pack on Drive — which is exactly the set a restore could duplicate. A no-op
/// when the record is already right, so a run does not rewrite the file.
///
/// Under `workspace_lock`, as every registry writer is: a rename or a create
/// landing beside this must not save a copy that lacks the other's change.
pub fn record_vault_id(app: &AppHandle, vault_id: &str) -> Result<()> {
    update_active(app, |workspace| {
        if workspace.vault_id.as_deref() == Some(vault_id) {
            return false;
        }
        workspace.vault_id = Some(vault_id.to_string());
        true
    })
}

/// Mirror a name a sync just adopted from another device into the registry.
///
/// The vault carries its own name (see [`crate::store::identity`]), but the
/// workspace list and the header read the registry — every workspace but the
/// active one is locked, so the registry is the only copy they can reach.
pub fn record_vault_name(app: &AppHandle, name: &str) -> Result<()> {
    update_active(app, named(name))
}

/// [`record_vault_name`] for a caller that already holds `workspace_lock`.
///
/// What `workspace_rename` mirrors through: it writes the vault's own copy and
/// this one without letting go in between, so it cannot take the lock again
/// here (see `commands::workspace::name_the_open_vault`).
pub(crate) fn record_vault_name_locked(root: &Path, active: &str, name: &str) -> Result<()> {
    update_active_locked(root, active, named(name))
}

// The single edit both mirrors make, so the two callers cannot drift apart.
fn named(name: &str) -> impl FnOnce(&mut Workspace) -> bool + '_ {
    move |workspace| {
        if workspace.name.as_deref() == Some(name) {
            return false;
        }
        workspace.name = Some(name.to_string());
        true
    }
}

/// Remember how many live entries workspace `id`'s vault holds, for the lock
/// screen to show while it is locked (see [`Workspace::item_count`]). Best
/// effort — a count that cannot be saved is only a stale number on the lock
/// screen — and a no-op when it has not changed.
///
/// By id rather than "the active one": a lock records the workspace it
/// sealed, which a switch landing a moment later must not redirect.
///
/// Never the write that brings the registry into being: a single-workspace
/// install has no file and no picker to show a count in, and keeps costing
/// nothing on disk. Its count arrives with the next open or lock once a
/// second workspace has made the file.
///
/// Takes `workspace_lock`, so never call it holding the session lock: every
/// reader of the two takes the registry's first.
pub fn record_item_count(app: &AppHandle, id: &str, count: u32) {
    let state = app.state::<AppState>();
    let recorded = storage::root_dir(app).and_then(|root| {
        let _paths = state.workspace_lock.lock().unwrap();
        count_in(&root, id, count)
    });
    if let Err(e) = recorded {
        log::warn!("could not record workspace {id}'s item count: {e}");
    }
}

// [`record_item_count`]'s write, under the caller's `workspace_lock`.
fn count_in(root: &Path, id: &str, count: u32) -> Result<()> {
    if Registry::read(root)?.is_none() {
        return Ok(());
    }
    update_active_locked(root, id, counted(count))
}

/// [`record_item_count`] for a caller that already holds `workspace_lock` and
/// the registry it is about to save (`workspace_select`, leaving one
/// workspace for another).
pub(crate) fn count_into(registry: &mut Registry, id: &str, count: u32) {
    if let Some(workspace) = registry.workspaces.iter_mut().find(|w| w.id == id) {
        counted(count)(workspace);
    }
}

fn counted(count: u32) -> impl FnOnce(&mut Workspace) -> bool {
    move |workspace| {
        if workspace.item_count == Some(count) {
            return false;
        }
        workspace.item_count = Some(count);
        true
    }
}

/// The active workspace's registry label, if it has one.
pub fn active_name(app: &AppHandle) -> Option<String> {
    let root = storage::root_dir(app).ok()?;
    let state = app.state::<AppState>();
    let _paths = state.workspace_lock.lock().unwrap();
    let active = state.active_workspace.lock().unwrap().clone();
    Registry::load(&root)
        .workspaces
        .into_iter()
        .find(|w| w.id == active)
        .and_then(|w| w.name)
}

/// Change the active workspace's registry entry, saving only if `change`
/// reports it changed something — so a sync run does not rewrite the file on
/// every pass.
fn update_active(app: &AppHandle, change: impl FnOnce(&mut Workspace) -> bool) -> Result<()> {
    let root = storage::root_dir(app)?;
    let state = app.state::<AppState>();
    let _paths = state.workspace_lock.lock().unwrap();
    let active = state.active_workspace.lock().unwrap().clone();
    update_active_locked(&root, &active, change)
}

/// [`update_active`]'s core, for a caller that already holds `workspace_lock`
/// and already knows which workspace is active — one with more than the
/// registry to write, which must not let go of the lock between the two.
fn update_active_locked(
    root: &Path,
    active: &str,
    change: impl FnOnce(&mut Workspace) -> bool,
) -> Result<()> {
    // Strict: a file that is there and unreadable is refused, not saved over
    // (see `Registry::read`). No file is the single-workspace default.
    let mut registry = Registry::read(root)?.unwrap_or_default();
    let Some(workspace) = registry.workspaces.iter_mut().find(|w| w.id == active) else {
        return Err(Error::NotFound);
    };
    if !change(workspace) {
        return Ok(());
    }
    registry.save(root)
}

/// Which workspace the app's paths currently resolve to.
pub fn active_id(app: &AppHandle) -> String {
    app.state::<AppState>()
        .active_workspace
        .lock()
        .unwrap()
        .clone()
}

pub fn is_primary(app: &AppHandle) -> bool {
    active_id(app) == PRIMARY_ID
}

#[cfg(test)]
mod tests {
    use super::*;
    use rowel_core::workspace::REGISTRY_FILE;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tmp_root() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rowel-workspace-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // A registry this cannot parse is refused, not replaced: saving the
    // default over it would drop every other workspace from the list for good.
    // The same guard covers the vault-id and name mirrors, which write through
    // the same path.
    #[test]
    fn a_count_leaves_a_registry_it_cannot_read_as_it_is() {
        let root = tmp_root();
        let malformed = r#"{"active":"a1b2","workspaces":[{"id":"#;
        fs::write(root.join(REGISTRY_FILE), malformed).unwrap();

        assert!(count_in(&root, PRIMARY_ID, 12).is_err());
        assert!(update_active_locked(&root, PRIMARY_ID, named("Home")).is_err());

        assert_eq!(
            fs::read_to_string(root.join(REGISTRY_FILE)).unwrap(),
            malformed
        );
    }

    // An install that never made a second workspace has no registry file, and
    // an unlock recording its size must not be what creates one.
    #[test]
    fn an_item_count_never_creates_the_registry() {
        let root = tmp_root();
        count_in(&root, PRIMARY_ID, 12).unwrap();
        assert!(!root.join(REGISTRY_FILE).exists());

        Registry::default().save(&root).unwrap();
        count_in(&root, PRIMARY_ID, 12).unwrap();
        assert_eq!(Registry::load(&root).workspaces[0].item_count, Some(12));
    }

    // A count is recorded against the workspace named, whichever is active,
    // and survives a save.
    #[test]
    fn an_item_count_is_kept_per_workspace() {
        let root = tmp_root();
        let mut registry = Registry {
            active: "a1b2".into(),
            workspaces: vec![
                Workspace {
                    id: PRIMARY_ID.into(),
                    name: None,
                    vault_id: None,
                    item_count: None,
                    color: None,
                },
                Workspace {
                    id: "a1b2".into(),
                    name: Some("Work".into()),
                    vault_id: None,
                    item_count: Some(3),
                    color: None,
                },
            ],
        };
        count_into(&mut registry, PRIMARY_ID, 284);
        registry.save(&root).unwrap();

        let loaded = Registry::load(&root);
        assert_eq!(loaded.workspaces[0].item_count, Some(284));
        assert_eq!(loaded.workspaces[1].item_count, Some(3));
        let json = fs::read_to_string(root.join(REGISTRY_FILE)).unwrap();
        assert!(json.contains(r#""itemCount":284"#), "{json}");
    }

    fn wal_of(path: &Path) -> PathBuf {
        let mut name = path.as_os_str().to_owned();
        name.push("-wal");
        PathBuf::from(name)
    }

    fn read(path: PathBuf) -> String {
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    // A vault's files, with contents that say which vault they came from. The
    // whole set, the pair an interrupted password change leaves and the
    // failed-unlock backoff included: a delete has to move exactly what a
    // workspace is, not a subset of it.
    fn seed_vault(dir: &Path, tag: &str) {
        fs::create_dir_all(dir.join("auth")).unwrap();
        for (file, what) in [
            (storage::DB_FILE, "db"),
            (storage::DB_REKEY_BACKUP_FILE, "db-backup"),
            (storage::KDF_SIDECAR_FILE, "kdf"),
            (storage::KDF_SIDECAR_REKEY_BACKUP_FILE, "kdf-backup"),
            (storage::LOCKOUT_SIDECAR_FILE, "lockout"),
            (storage::GDRIVE_FILE, "token"),
        ] {
            fs::write(dir.join(file), format!("{tag}-{what}")).unwrap();
        }
        fs::write(wal_of(&dir.join(storage::DB_FILE)), format!("{tag}-wal")).unwrap();
    }

    // The primary, and one workspace that would be promoted in its place.
    fn two_workspaces(root: &Path) -> Registry {
        let registry = Registry {
            active: PRIMARY_ID.into(),
            workspaces: vec![
                Workspace {
                    id: PRIMARY_ID.into(),
                    name: None,
                    vault_id: None,
                    item_count: None,
                    color: None,
                },
                Workspace {
                    id: "a1b2".into(),
                    name: Some("Work".into()),
                    vault_id: Some("cafe".into()),
                    item_count: None,
                    color: None,
                },
            ],
        };
        registry.save(root).unwrap();
        registry
    }

    // A delete stopped where a crash would stop it: every move made, the
    // registry not yet saved. The same steps the real one takes, minus the
    // save.
    fn interrupt_delete(root: &Path, id: &str, promoted: Option<&str>) {
        let journal = DeleteJournal {
            id: id.into(),
            promoted: promoted.map(str::to_string),
        };
        journal.write(root).unwrap();
        stage_out(root, id).unwrap();
        if let Some(promoted) = promoted {
            storage::move_vault_files(&dir_of(root, promoted), root).unwrap();
        }
    }

    // The whole point of the promotion: `dir_of(root, PRIMARY_ID)` is the root,
    // so the survivor's files have to *be* in the root for the next unlock to
    // find them — the WAL included, since the database without it is an older
    // vault, and the rekey pair and the backoff state, which are as much the
    // survivor's as its database is.
    #[test]
    fn a_promoted_workspace_lands_where_the_next_unlock_looks() {
        let root = tmp_root();
        seed_vault(&root, "primary");
        fs::write(root.join(storage::BIOMETRIC_FILE), "protected").unwrap();
        let from = dir_of(&root, "a1b2");
        seed_vault(&from, "work");
        let deletion = two_workspaces(&root).without(PRIMARY_ID).unwrap();

        apply_deletion(&root, PRIMARY_ID, &deletion).unwrap();

        assert_eq!(read(root.join(storage::DB_FILE)), "work-db");
        assert_eq!(read(wal_of(&root.join(storage::DB_FILE))), "work-wal");
        assert_eq!(read(root.join(storage::KDF_SIDECAR_FILE)), "work-kdf");
        assert_eq!(read(root.join(storage::GDRIVE_FILE)), "work-token");
        // Left behind, these would strand the promoted vault: the next unlock's
        // recovery would find a snapshot with no pair to roll back to, and the
        // backoff the survivor had earned would be reset.
        assert_eq!(
            read(root.join(storage::DB_REKEY_BACKUP_FILE)),
            "work-db-backup"
        );
        assert_eq!(
            read(root.join(storage::KDF_SIDECAR_REKEY_BACKUP_FILE)),
            "work-kdf-backup"
        );
        assert_eq!(
            read(root.join(storage::LOCKOUT_SIDECAR_FILE)),
            "work-lockout"
        );
        // Nothing of the survivor's is left at its old address, and nothing of
        // the deleted vault's in the root — its enrollment marker included,
        // whose keychain item goes with the key that is gone.
        assert!(!from.exists());
        assert!(!root.join(storage::BIOMETRIC_FILE).exists());
        assert!(!staging(&root).exists());
        assert!(!DeleteJournal::path(&root).exists());

        let registry = Registry::load(&root);
        assert_eq!(registry.workspaces.len(), 1);
        assert_eq!(registry.workspaces[0].id, PRIMARY_ID);
        assert_eq!(registry.workspaces[0].name.as_deref(), Some("Work"));
    }

    // A workspace with a directory of its own takes it with it, and the root it
    // shares with the registry and the primary's vault is left alone.
    #[test]
    fn deleting_a_workspace_of_its_own_leaves_the_root_alone() {
        let root = tmp_root();
        seed_vault(&root, "primary");
        let gone = dir_of(&root, "a1b2");
        seed_vault(&gone, "work");
        let deletion = two_workspaces(&root).without("a1b2").unwrap();

        apply_deletion(&root, "a1b2", &deletion).unwrap();

        assert!(!gone.exists());
        assert_eq!(read(root.join(storage::DB_FILE)), "primary-db");
        assert_eq!(read(root.join(storage::GDRIVE_FILE)), "primary-token");
        assert_eq!(Registry::load(&root).workspaces.len(), 1);
        assert!(!staging(&root).exists());
        assert!(!DeleteJournal::path(&root).exists());
    }

    // The registry write is the last step before anything is removed, and here
    // it cannot succeed (a directory stands where the file goes). Nothing has
    // been removed at that point, so every move has to come back — otherwise
    // the user is left with a registry naming vaults whose files are gone, and
    // no way to prove a password against them again.
    #[test]
    fn a_failed_delete_puts_the_old_layout_back() {
        let root = tmp_root();
        seed_vault(&root, "primary");
        fs::write(root.join(storage::BIOMETRIC_FILE), "protected").unwrap();
        let from = dir_of(&root, "a1b2");
        seed_vault(&from, "work");
        let deletion = two_workspaces(&root).without(PRIMARY_ID).unwrap();
        fs::remove_file(root.join(REGISTRY_FILE)).unwrap();
        fs::create_dir(root.join(REGISTRY_FILE)).unwrap();

        assert!(apply_deletion(&root, PRIMARY_ID, &deletion).is_err());

        assert_eq!(read(root.join(storage::DB_FILE)), "primary-db");
        assert_eq!(read(wal_of(&root.join(storage::DB_FILE))), "primary-wal");
        assert_eq!(read(root.join(storage::KDF_SIDECAR_FILE)), "primary-kdf");
        assert_eq!(read(root.join(storage::GDRIVE_FILE)), "primary-token");
        assert_eq!(read(root.join(storage::BIOMETRIC_FILE)), "protected");
        assert_eq!(read(from.join(storage::DB_FILE)), "work-db");
        assert_eq!(
            read(from.join(storage::LOCKOUT_SIDECAR_FILE)),
            "work-lockout"
        );
        assert!(!staging(&root).exists());
        assert!(!DeleteJournal::path(&root).exists());
    }

    // A record that is still on disk is a delete nothing could undo: the next
    // one waits for the launch that resolves it rather than writing over the
    // only note of where the first one's files went.
    #[test]
    fn a_delete_does_not_start_over_one_that_was_never_resolved() {
        let root = tmp_root();
        seed_vault(&root, "primary");
        seed_vault(&dir_of(&root, "a1b2"), "work");
        let deletion = two_workspaces(&root).without("a1b2").unwrap();
        DeleteJournal {
            id: "c3d4".into(),
            promoted: None,
        }
        .write(&root)
        .unwrap();

        assert!(matches!(
            apply_deletion(&root, "a1b2", &deletion),
            Err(Error::Other(_))
        ));
        assert_eq!(
            read(dir_of(&root, "a1b2").join(storage::DB_FILE)),
            "work-db"
        );
    }

    // A crash between the moves and the registry write: the registry still
    // names the workspace that was being promoted, so the launch that finds the
    // journal puts every file back where it was.
    #[test]
    fn an_interrupted_delete_the_registry_never_recorded_is_undone() {
        let root = tmp_root();
        seed_vault(&root, "primary");
        let from = dir_of(&root, "a1b2");
        seed_vault(&from, "work");
        two_workspaces(&root);
        interrupt_delete(&root, PRIMARY_ID, Some("a1b2"));

        recover_interrupted_delete(&root);

        assert_eq!(read(root.join(storage::DB_FILE)), "primary-db");
        assert_eq!(read(wal_of(&root.join(storage::DB_FILE))), "primary-wal");
        assert_eq!(read(root.join(storage::GDRIVE_FILE)), "primary-token");
        assert_eq!(read(from.join(storage::DB_FILE)), "work-db");
        assert_eq!(
            read(from.join(storage::DB_REKEY_BACKUP_FILE)),
            "work-db-backup"
        );
        assert!(!staging(&root).exists());
        assert!(!DeleteJournal::path(&root).exists());
    }

    // A crash inside the undo itself, between a database and its WAL: the
    // promoted vault's `vault.db` is back at its own address and its `-wal` is
    // still in the root. The launch that picks the undo up again has to carry
    // that `-wal` after it — the registry still names the workspace, and both
    // files have to end up together at the address it names, or the workspace
    // reopens the older vault the `-wal` was holding the pages for. The
    // deleted primary's own `-wal` comes back to the root behind it, so the
    // stranded one must not be written over either.
    #[test]
    fn an_undo_interrupted_between_a_database_and_its_wal_is_finished_next_launch() {
        let root = tmp_root();
        seed_vault(&root, "primary");
        let from = dir_of(&root, "a1b2");
        seed_vault(&from, "work");
        two_workspaces(&root);
        interrupt_delete(&root, PRIMARY_ID, Some("a1b2"));
        // The undo got the promoted database home and stopped there.
        fs::rename(root.join(storage::DB_FILE), from.join(storage::DB_FILE)).unwrap();

        recover_interrupted_delete(&root);

        assert_eq!(read(from.join(storage::DB_FILE)), "work-db");
        assert_eq!(read(wal_of(&from.join(storage::DB_FILE))), "work-wal");
        assert_eq!(read(from.join(storage::GDRIVE_FILE)), "work-token");
        assert_eq!(read(root.join(storage::DB_FILE)), "primary-db");
        assert_eq!(read(wal_of(&root.join(storage::DB_FILE))), "primary-wal");
        assert_eq!(
            read(root.join(storage::LOCKOUT_SIDECAR_FILE)),
            "primary-lockout"
        );
        assert!(!staging(&root).exists());
        assert!(!DeleteJournal::path(&root).exists());
    }

    // A crash just after the registry write: the delete is committed — the file
    // on disk no longer names the deleted workspace — so the launch that finds
    // the journal clears away what the moves left rather than undoing them.
    #[test]
    fn an_interrupted_delete_the_registry_recorded_is_finished() {
        let root = tmp_root();
        seed_vault(&root, "primary");
        let from = dir_of(&root, "a1b2");
        seed_vault(&from, "work");
        let deletion = two_workspaces(&root).without(PRIMARY_ID).unwrap();
        interrupt_delete(&root, PRIMARY_ID, Some("a1b2"));
        deletion.registry.save(&root).unwrap();

        recover_interrupted_delete(&root);

        assert_eq!(read(root.join(storage::DB_FILE)), "work-db");
        assert_eq!(
            read(root.join(storage::LOCKOUT_SIDECAR_FILE)),
            "work-lockout"
        );
        assert!(!from.exists());
        assert!(!staging(&root).exists());
        assert!(!DeleteJournal::path(&root).exists());
    }

    // `synced` is the token file in the workspace's own directory, not its
    // vault id: a disconnected workspace keeps the id and reads as local.
    #[test]
    fn statuses_read_sync_off_each_workspace_token_file() {
        let root = tmp_root();
        let registry = Registry {
            active: PRIMARY_ID.to_string(),
            workspaces: vec![
                Workspace {
                    id: PRIMARY_ID.to_string(),
                    name: None,
                    vault_id: Some("beef".into()),
                    item_count: None,
                    color: None,
                },
                Workspace {
                    id: "w2".into(),
                    name: Some("Work".into()),
                    vault_id: Some("cafe".into()),
                    item_count: None,
                    color: None,
                },
            ],
        };
        let token = dir_of(&root, "w2").join(storage::GDRIVE_FILE);
        fs::create_dir_all(token.parent().unwrap()).unwrap();
        fs::write(&token, b"sealed").unwrap();

        let listed = statuses(&registry, &root);
        assert_eq!(listed.len(), 2);
        assert!(!listed[0].synced, "a vault id alone is not a connection");
        assert!(listed[1].synced);
        assert_eq!(listed[1].workspace, registry.workspaces[1]);

        // An empty token file is a disconnect that never finished: not synced.
        fs::write(&token, b"").unwrap();
        assert!(!statuses(&registry, &root)[1].synced);

        let json = serde_json::to_value(&listed[1]).unwrap();
        assert_eq!(json["id"], "w2");
        assert_eq!(json["vaultId"], "cafe");
        assert_eq!(json["synced"], true);
    }
}
