//! Paths the user has pointed the app at.
//!
//! A command that reads a local file (`scan_image`, `read_env_file`,
//! `attachment_add`, `import_entries`, `import_swftx`,
//! `setup_restore_from_file`, `setup_restore_from_legacy`,
//! `workspace_restore_from_file`) gets its path from the webview, and the webview's word is not authorization: a
//! script that got to run there could name any file on disk — another
//! project's `.env`, a photo in the user's library — and have the backend hand
//! it back. What the extension and shape checks in those commands decide is
//! *what kind of file* a path names, never *whether the user chose it*.
//!
//! Three events prove the user chose a file, and all reach Rust before the
//! webview ever sees the path: the OS file dialog, when it is opened from Rust
//! (`commands::tools::pick_file`), a drop on the window, which the OS
//! delivers as a window event (`window::create`), and the OS opening a backup
//! with the app (`opened::accept`). Each grants the path here; the reading
//! command then has to `take` it. A path nobody granted is refused however
//! well it is named.
//!
//! A grant carries the purpose it was made for, because the framing the user
//! answered is part of what they agreed to. The env picker offers every file on
//! disk, the image picker only what the scanner opens; a path chosen from a
//! dialog that said "pick an environment file" was not offered up to be OCRed
//! for the card number or the passport fields in it. So the purpose is recorded
//! with the path and the reading command has to name the same one: a path
//! picked as an env file can only be read as one.
//!
//! A grant also belongs to the session the choice was made in, because which
//! vault it was made for is part of it too: a file picked to attach to an
//! entry of one lockbox was not offered to whichever lockbox is unlocked next.
//! The webview outlives every session — the same page runs before a lock and
//! after the next unlock, of this vault or another — so a script that kept a
//! path from one session could otherwise spend its grant in the next. Time is
//! cut into stretches at every change of session, each numbered ([`Scope`]):
//! a session runs from one announced open to the end that follows it
//! (`session::announce_opened`, `session::announce_ended`, which tell this set
//! under the session guard), and the time with no vault open between them is a
//! stretch of its own. So:
//!
//! - A choice belongs to the stretch it began in: the one current when the
//!   dialog was opened (read with [`PathGrants::scope`] before it opens), at
//!   the moment of the drop, or when the OS handed the file over. A dialog
//!   that answers in a later stretch grants nothing.
//! - Only the current stretch's grants are spendable, and every change of
//!   session drops the rest: a lock, an unlock, a workspace switch, a lockbox
//!   created or restored beside the open one taking its place. A password
//!   change is not one — the same vault stays open under a new key, and its
//!   grants with it. That rests on a lease handed back unannounced
//!   (`Session::take_out`, then `adopt` or `restore`: a password change, its
//!   rollback, a lockbox made or restored beside that fails and puts the open
//!   one back) carrying the vault it took, as each of those callers does; a
//!   different vault taking over goes through `session::continue_lease`, which
//!   announces an open. Nothing checks it at run time: `adopt` takes any
//!   store, and comparing vault ids under the session guard would mean a
//!   database read there.
//! - What may be read depends on whether a vault is open. With none, the only
//!   read is a first-run restore ([`Purpose::Restore`]:
//!   `setup_restore_from_file`, `setup_restore_from_legacy`), and that read is
//!   refused once one is open; every other purpose is read only in a session.
//!   A choice made on the setup screens or the lock screen is restored on a
//!   fresh install or spent by nothing.
//!
//! Two flows cross from no vault open into a session, and each is meant to.
//! A first-run restore spends its grant before the vault it restored is opened
//! (`commands::setup`), so the choice is used up in the stretch it was made
//! in; one left unspent would be dropped by that open like any other. And a
//! backup the OS opened with the app while no vault was open waits for the
//! session opened next, which is where the frontend takes it (Settings ›
//! Import, once the vault is unlocked): the user chose it of the app as a
//! whole, not of a session, and the only one that can take it is the next.
//! That is the one grant carried across a change of session, and only across
//! the open that ends its stretch ([`PathGrants::grant_opened`]).
//!
//! Grants are one-shot — a take consumes them — and few: a bounded set, oldest
//! out, since a grant that was never read is a path the user pointed at and
//! then walked away from. Kept canonical so a grant cannot be sidestepped or
//! spent through a spelling of the path other than the one it was made under.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{Error, Result};

/// What a granted path may be read as — one variant per reading command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// An image to scan: what `commands::tools::scan_image` reads, OCRing it
    /// and handing back the card or identity fields found in it.
    Image,
    /// An environment file to read: what `commands::env::read_env_file` reads,
    /// handing its whole text back to the webview.
    Env,
    /// A file to attach to an entry: what `commands::attachments::attachment_add`
    /// reads, sealing it into the vault. Nothing of it goes back to the webview.
    Attachment,
    /// A foreign password export: what `commands::import::import_entries`
    /// reads, previewing it and then writing its rows into the vault.
    Import,
    /// A `.rowel` or `.swftx` backup taken into the open vault: what
    /// `commands::vault::import_swftx` merges and
    /// `commands::workspace::workspace_restore_from_file` restores as a lockbox
    /// beside the open one, each only once the backup's own password has
    /// opened it.
    Backup,
    /// A `.rowel` or `.swftx` backup made a fresh install's vault: what
    /// `commands::setup::setup_restore_from_file` / `setup_restore_from_legacy`
    /// restore, only once its own password has opened it. The one read that
    /// runs with no vault open, and only then (see [`position`]).
    Restore,
}

/// The refusal every reader gives a path nobody granted it.
pub fn not_chosen() -> Error {
    Error::Unsupported("this file was not chosen in the app".into())
}

/// The stretch of time a choice was made in: one session, or one stretch with
/// no vault open (see the module docs). Read with [`PathGrants::scope`].
///
/// Counted here rather than read off `session::Epoch`, which moves whenever
/// the key changes hands — a lease taken out for a password change or for a
/// lockbox made or restored beside the open one moves it too, and a restore
/// beside checks its grant before its lease and spends it after. The stretches
/// counted here change only where the session the user sees does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scope {
    generation: u64,
    /// A vault was open through it.
    session: bool,
}

/// How the user made a choice, which says what else it may be read as and
/// whether it may wait for a session.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Via {
    /// Picked from a dialog opened for one purpose.
    Dialog,
    /// Dropped on the window ([`PathGrants::grant_drop`]).
    Drop,
    /// Opened with the app from the OS ([`PathGrants::grant_opened`]).
    Os,
}

/// One path the user chose, how, and when.
struct Grant {
    path: PathBuf,
    purpose: Purpose,
    via: Via,
    scope: Scope,
}

/// How many grants are held at once. A user picks or drops one file at a time
/// and every grant is spent by the read that follows, so anything near the cap
/// is grants nothing collected.
const CAP: usize = 64;

#[derive(Default)]
pub struct PathGrants {
    held: Mutex<Held>,
}

#[derive(Default)]
struct Held {
    // Oldest first, so the bound evicts from the front. Every one is of
    // `scope`: a change of session drops or re-scopes the rest.
    paths: VecDeque<Grant>,
    scope: Scope,
}

impl Held {
    /// Hold `path`, canonical, in the current stretch.
    fn record(&mut self, path: PathBuf, purpose: Purpose, via: Via) {
        self.paths.retain(|grant| grant.path != path);
        if self.paths.len() >= CAP {
            self.paths.pop_front();
        }
        self.paths.push_back(Grant {
            path,
            purpose,
            via,
            scope: self.scope,
        });
    }
}

impl PathGrants {
    /// The stretch a choice beginning now belongs to. Read when the choice
    /// begins — before the dialog opens, not once it answers — and handed to
    /// [`PathGrants::grant`] with what was chosen.
    pub fn scope(&self) -> Scope {
        self.held.lock().unwrap().scope
    }

    /// Record that the user chose `path`, from a dialog opened in `scope`, to
    /// be read as `purpose`. Whether it was recorded: not if the session
    /// changed while the dialog was up, since the answer belongs to a session
    /// that has ended and that session hears nothing of it. Otherwise a path
    /// that cannot be resolved (it is not there) is an error, and nothing is
    /// recorded: there is nothing to read.
    ///
    /// One grant per path: choosing the same file again — under this purpose or
    /// another — replaces what was held for it, so the last thing the user was
    /// asked is what the path stands for.
    pub fn grant(&self, scope: Scope, path: &Path, purpose: Purpose) -> Result<bool> {
        // Resolved outside the lock: on a network volume it can take a while.
        let resolved = path.canonicalize();
        let mut held = self.held.lock().unwrap();
        if held.scope != scope {
            return Ok(false);
        }
        held.record(resolved?, purpose, Via::Dialog);
        Ok(true)
    }

    /// [`PathGrants::grant`] for a file dropped on the window now, `purpose`
    /// being what its kind makes it. A drop is also spendable by the readers
    /// that take a file into the vault — as an attachment, an export to import
    /// or a backup to merge or restore, on a fresh install or beside the open
    /// lockbox: the user handed the file to the app itself rather than to a
    /// dialog with a framing of its own, and which of those it becomes is the
    /// drop target's to say. None of them hands back more than the kind's own
    /// reader would — the env reader returns the whole text.
    ///
    /// A drop waits on nothing, so it belongs to the stretch current when it
    /// is recorded. One that cannot be resolved is not recorded; the webview
    /// hears of the drop from tauri all the same, and its reader refuses it.
    pub fn grant_drop(&self, path: &Path, purpose: Purpose) {
        if let Ok(path) = path.canonicalize() {
            self.held.lock().unwrap().record(path, purpose, Via::Drop);
        }
    }

    /// [`PathGrants::grant`] for a backup the OS opened with the app now
    /// (`opened::accept`), which is a backup to whichever reader of one can
    /// run: restored on a fresh install, merged or restored beside once a vault
    /// is open. Like a drop it waits on nothing, and belongs to the stretch
    /// current when it is recorded. One that arrives with a vault open belongs
    /// to that session like any choice. One that arrives with none is spendable
    /// while none is — by a first-run restore — and is carried into the session
    /// opened next, where the frontend offers it for import; it ends with that
    /// session. A path that cannot be resolved is an error, and the caller is
    /// not to offer it.
    ///
    /// Only the desktop has a caller: the phones declare no file associations.
    #[cfg_attr(not(desktop), allow(dead_code))]
    pub fn grant_opened(&self, path: &Path) -> Result<()> {
        let path = path.canonicalize()?;
        self.held
            .lock()
            .unwrap()
            .record(path, Purpose::Backup, Via::Os);
        Ok(())
    }

    /// A session opened: an unlock, a workspace switch that lands on an open
    /// vault, a lockbox created or restored beside the open one taking its
    /// place. Told by `session::announce_opened`, under the session guard.
    ///
    /// Nothing chosen before it is spendable in it, and it is dropped — except
    /// a backup the OS opened with the app while no vault was open, which
    /// becomes this session's (see [`PathGrants::grant_opened`]).
    pub fn session_opened(&self) {
        let mut held = self.held.lock().unwrap();
        let after_none = !held.scope.session;
        let scope = Scope {
            generation: held.scope.generation + 1,
            session: true,
        };
        held.scope = scope;
        held.paths.retain_mut(|grant| {
            let carried = after_none && grant.via == Via::Os;
            grant.scope = scope;
            carried
        });
    }

    /// The session ended: a lock of any kind, or a workspace switch leaving it.
    /// Told by `session::announce_ended`, under the session guard. Every grant
    /// goes with it. An end with no session open — a lock that only empties the
    /// app-level key ring — leaves the stretch as it was.
    pub fn session_ended(&self) {
        let mut held = self.held.lock().unwrap();
        if !held.scope.session {
            return;
        }
        held.scope = Scope {
            generation: held.scope.generation + 1,
            session: false,
        };
        held.paths.clear();
    }

    /// Spend the grant for `path` as `purpose`: whether the user chose it for
    /// this reader, and if so, no longer — the next read of the same path needs
    /// the user's say again.
    ///
    /// A path granted for another purpose is refused and left where it is: the
    /// choice the user did make is not burned by a read it was not made for,
    /// and the reader it was meant for can still spend it.
    pub fn take(&self, path: &Path, purpose: Purpose) -> bool {
        let Ok(path) = path.canonicalize() else {
            return false;
        };
        let mut held = self.held.lock().unwrap();
        match position(&held, &path, purpose) {
            Some(at) => {
                held.paths.remove(at);
                true
            }
            None => false,
        }
    }

    /// Whether `path` is granted as `purpose`, spending nothing. For a reader
    /// that may read the same choice more than once before it is done with it:
    /// an import previews the file before it writes it, and a backup is read
    /// again after a mistyped password. Such a reader checks here before each
    /// read and [`take`](PathGrants::take)s the grant once the file has done
    /// what it was chosen for.
    pub fn holds(&self, path: &Path, purpose: Purpose) -> bool {
        let Ok(path) = path.canonicalize() else {
            return false;
        };
        position(&self.held.lock().unwrap(), &path, purpose).is_some()
    }
}

#[cfg(test)]
impl PathGrants {
    /// A set with a vault open, as every reader but the first-run restores
    /// finds it.
    pub fn in_session() -> Self {
        let grants = Self::default();
        grants.session_opened();
        grants
    }

    /// A dialog for `purpose` opened and answered with `path` just now.
    pub fn pick(&self, path: &Path, purpose: Purpose) {
        assert!(self.grant(self.scope(), path, purpose).unwrap());
    }
}

/// Where the grant spendable for `path` as `purpose` is held, if anywhere.
///
/// The one place a spend is decided, for every reader: a grant of the current
/// stretch only, and a read the state allows — a first-run restore with no
/// vault open and only then, every other read only with one open. So what is
/// chosen on the setup screens or the lock screen is restored on a fresh
/// install or spent by nothing, however late the reader asking for it checks
/// for a session.
fn position(held: &Held, path: &Path, purpose: Purpose) -> Option<usize> {
    if held.scope.session == (purpose == Purpose::Restore) {
        return None;
    }
    held.paths.iter().position(|grant| {
        grant.scope == held.scope
            && grant.path == path
            && (grant.purpose == purpose
                || match grant.via {
                    Via::Dialog => false,
                    Via::Drop => matches!(
                        purpose,
                        Purpose::Attachment | Purpose::Import | Purpose::Backup | Purpose::Restore
                    ),
                    Via::Os => purpose == Purpose::Restore,
                })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rowel-grants-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, b"x").unwrap();
        path
    }

    #[test]
    fn a_granted_path_is_taken_once() {
        let grants = PathGrants::in_session();
        let path = scratch("once");
        grants.pick(&path, Purpose::Image);
        assert!(grants.take(&path, Purpose::Image));
        assert!(!grants.take(&path, Purpose::Image));
    }

    #[test]
    fn a_path_nobody_granted_is_refused() {
        let grants = PathGrants::in_session();
        let path = scratch("ungranted");
        assert!(!grants.take(&path, Purpose::Image));
        assert!(!grants.take(Path::new("/definitely/not/here"), Purpose::Env));
    }

    #[test]
    fn a_grant_is_spent_through_any_spelling_of_the_path() {
        let grants = PathGrants::in_session();
        let path = scratch("spelled");
        let dir = path.parent().unwrap();
        let roundabout = dir
            .join("..")
            .join(dir.file_name().unwrap())
            .join("spelled");
        grants.pick(&roundabout, Purpose::Image);
        assert!(grants.take(&path, Purpose::Image));
        assert!(!grants.take(&roundabout, Purpose::Image));
    }

    #[test]
    fn the_oldest_grant_goes_first_when_the_set_is_full() {
        let grants = PathGrants::in_session();
        let first = scratch("evicted");
        grants.pick(&first, Purpose::Env);
        let rest: Vec<_> = (0..CAP).map(|i| scratch(&format!("kept-{i}"))).collect();
        for path in &rest {
            grants.pick(path, Purpose::Env);
        }
        assert!(!grants.take(&first, Purpose::Env));
        for path in &rest {
            assert!(grants.take(path, Purpose::Env));
        }
    }

    #[test]
    fn regranting_a_held_path_keeps_one_grant_for_it() {
        let grants = PathGrants::in_session();
        let path = scratch("twice");
        grants.pick(&path, Purpose::Env);
        grants.pick(&path, Purpose::Env);
        assert!(grants.take(&path, Purpose::Env));
        assert!(!grants.take(&path, Purpose::Env));
    }

    #[test]
    fn a_path_picked_as_an_env_file_cannot_be_scanned() {
        let grants = PathGrants::in_session();
        let path = scratch("env-not-image");
        grants.pick(&path, Purpose::Env);
        assert!(!grants.take(&path, Purpose::Image));
        // The refused take spent nothing: the reader it was picked for still
        // has its grant.
        assert!(grants.take(&path, Purpose::Env));
    }

    // A drop can be attached whatever its kind; a picked file only from the
    // attachment picker, and an attachment pick reads as nothing else.
    #[test]
    fn a_dropped_file_can_be_attached_and_a_picked_one_only_as_picked() {
        let grants = PathGrants::in_session();
        let dropped = scratch("dropped");
        grants.grant_drop(&dropped, Purpose::Env);
        assert!(grants.take(&dropped, Purpose::Attachment));
        assert!(!grants.take(&dropped, Purpose::Env), "spent by the attach");

        let picked = scratch("picked-env");
        grants.pick(&picked, Purpose::Env);
        assert!(!grants.take(&picked, Purpose::Attachment));

        let attached = scratch("picked-attachment");
        grants.pick(&attached, Purpose::Attachment);
        assert!(!grants.take(&attached, Purpose::Env));
        assert!(!grants.take(&attached, Purpose::Image));
        assert!(grants.take(&attached, Purpose::Attachment));
    }

    #[test]
    fn a_dropped_file_can_be_imported_or_restored() {
        let grants = PathGrants::in_session();
        let export = scratch("dropped-export");
        grants.grant_drop(&export, Purpose::Env);
        assert!(grants.take(&export, Purpose::Import));

        let backup = scratch("dropped-backup");
        grants.grant_drop(&backup, Purpose::Env);
        assert!(grants.take(&backup, Purpose::Backup));
        assert!(!grants.take(&backup, Purpose::Env), "spent by the restore");
    }

    #[test]
    fn a_picked_export_or_backup_reads_only_as_picked() {
        let grants = PathGrants::in_session();
        let export = scratch("picked-export");
        grants.pick(&export, Purpose::Import);
        assert!(!grants.take(&export, Purpose::Backup));
        assert!(!grants.take(&export, Purpose::Env));
        assert!(grants.take(&export, Purpose::Import));

        let backup = scratch("picked-backup");
        grants.pick(&backup, Purpose::Backup);
        assert!(!grants.take(&backup, Purpose::Import));
        assert!(!grants.take(&backup, Purpose::Attachment));
        assert!(grants.take(&backup, Purpose::Backup));
    }

    #[test]
    fn holding_a_grant_spends_nothing() {
        let grants = PathGrants::in_session();
        let path = scratch("held");
        assert!(!grants.holds(&path, Purpose::Import));
        grants.pick(&path, Purpose::Import);
        assert!(grants.holds(&path, Purpose::Import));
        assert!(grants.holds(&path, Purpose::Import));
        assert!(!grants.holds(&path, Purpose::Backup));
        assert!(grants.take(&path, Purpose::Import));
        assert!(!grants.holds(&path, Purpose::Import));
    }

    #[test]
    fn regranting_under_another_purpose_replaces_the_first() {
        let grants = PathGrants::in_session();
        let path = scratch("repurposed");
        grants.pick(&path, Purpose::Image);
        grants.pick(&path, Purpose::Env);
        assert!(!grants.take(&path, Purpose::Image));
        assert!(grants.take(&path, Purpose::Env));
        assert!(!grants.take(&path, Purpose::Env));
    }

    // A path the webview kept from one session buys nothing in the next: not
    // after a lock and an unlock of another workspace, and not after a lockbox
    // created or restored beside the open one takes the session over.
    #[test]
    fn a_pick_is_not_spent_in_another_session() {
        let grants = PathGrants::in_session();
        let path = scratch("other-session");
        grants.pick(&path, Purpose::Attachment);
        grants.session_ended();
        grants.session_opened();
        assert!(!grants.holds(&path, Purpose::Attachment));
        assert!(!grants.take(&path, Purpose::Attachment));

        grants.pick(&path, Purpose::Attachment);
        grants.session_opened();
        assert!(!grants.take(&path, Purpose::Attachment));
    }

    #[test]
    fn a_dialog_answered_after_a_lock_grants_nothing() {
        let grants = PathGrants::in_session();
        let path = scratch("answered-late");

        let opened_in = grants.scope();
        grants.session_ended();
        assert!(!grants.grant(opened_in, &path, Purpose::Restore).unwrap());
        assert!(
            !grants.holds(&path, Purpose::Restore),
            "nor on the lock screen"
        );
        grants.session_opened();
        assert!(!grants.grant(opened_in, &path, Purpose::Attachment).unwrap());
        assert!(
            !grants.take(&path, Purpose::Attachment),
            "nor after the unlock"
        );

        // Opened on the lock screen and answered after an unlock: the same.
        grants.session_ended();
        let opened_in = grants.scope();
        grants.session_opened();
        assert!(!grants.grant(opened_in, &path, Purpose::Backup).unwrap());
        assert!(!grants.take(&path, Purpose::Backup));

        // Answered in the session it was opened in, it grants.
        let opened_in = grants.scope();
        assert!(grants.grant(opened_in, &path, Purpose::Attachment).unwrap());
        assert!(grants.take(&path, Purpose::Attachment));
    }

    // A dialog's answer that cannot be resolved is an error the user sees, not
    // a dismissal — unless the session the dialog opened in has ended, which
    // hears nothing of it.
    #[test]
    fn a_pick_that_cannot_be_resolved_is_an_error_unless_its_session_ended() {
        let grants = PathGrants::in_session();
        let missing = Path::new("/definitely/not/here");
        let opened_in = grants.scope();
        assert!(grants.grant(opened_in, missing, Purpose::Env).is_err());
        grants.session_ended();
        assert!(!grants.grant(opened_in, missing, Purpose::Env).unwrap());
    }

    #[test]
    fn a_drop_belongs_to_the_session_it_landed_in() {
        let grants = PathGrants::in_session();
        let path = scratch("dropped-then-locked");
        grants.grant_drop(&path, Purpose::Env);
        grants.session_ended();
        grants.session_opened();
        for purpose in [
            Purpose::Env,
            Purpose::Attachment,
            Purpose::Import,
            Purpose::Backup,
            Purpose::Restore,
        ] {
            assert!(!grants.take(&path, purpose), "{purpose:?}");
        }

        grants.grant_drop(&path, Purpose::Env);
        assert!(grants.take(&path, Purpose::Attachment));
    }

    // First-run setup and the lock screen: what is chosen there is read only
    // by the restores that run without a session — not by a reader that needs
    // one and has yet to find out it is locked, and not by any reader once a
    // vault is open.
    #[test]
    fn with_no_vault_open_a_choice_is_spendable_only_by_a_first_run_restore() {
        let grants = PathGrants::default();
        let backup = scratch("setup-backup");
        grants.pick(&backup, Purpose::Restore);
        assert!(grants.holds(&backup, Purpose::Restore));
        assert!(
            !grants.holds(&backup, Purpose::Backup),
            "not beside a vault"
        );

        let env = scratch("lock-screen-env");
        grants.pick(&env, Purpose::Env);
        assert!(!grants.take(&env, Purpose::Env));

        let dropped = scratch("lock-screen-drop");
        grants.grant_drop(&dropped, Purpose::Env);
        for purpose in [
            Purpose::Env,
            Purpose::Attachment,
            Purpose::Import,
            Purpose::Backup,
        ] {
            assert!(!grants.holds(&dropped, purpose), "{purpose:?}");
        }
        assert!(
            grants.holds(&dropped, Purpose::Restore),
            "a backup dropped on the restore step"
        );

        grants.session_opened();
        assert!(!grants.take(&backup, Purpose::Restore));
        assert!(!grants.take(&backup, Purpose::Backup), "not merged after");
        assert!(!grants.take(&dropped, Purpose::Attachment));
        assert!(!grants.take(&env, Purpose::Env));
    }

    // The reverse: what is chosen in a session is read in it, and never by a
    // first-run restore — in it, or on the lock screen after it.
    #[test]
    fn a_backup_chosen_in_a_session_is_not_restored_on_a_fresh_install() {
        let grants = PathGrants::in_session();
        let backup = scratch("session-backup");
        grants.pick(&backup, Purpose::Backup);
        assert!(grants.holds(&backup, Purpose::Backup), "merged in it");
        assert!(!grants.holds(&backup, Purpose::Restore));
        let dropped = scratch("session-drop");
        grants.grant_drop(&dropped, Purpose::Env);
        assert!(!grants.holds(&dropped, Purpose::Restore));

        grants.session_ended();
        for path in [&backup, &dropped] {
            assert!(!grants.holds(path, Purpose::Restore));
            assert!(!grants.holds(path, Purpose::Backup));
        }
    }

    #[test]
    fn the_end_of_a_session_drops_every_grant_however_it_was_made() {
        let grants = PathGrants::in_session();
        grants.pick(&scratch("ended-picked"), Purpose::Env);
        grants.grant_drop(&scratch("ended-dropped"), Purpose::Image);
        grants.grant_opened(&scratch("ended-opened")).unwrap();
        assert_eq!(grants.held.lock().unwrap().paths.len(), 3);
        grants.session_ended();
        assert!(grants.held.lock().unwrap().paths.is_empty());
    }

    // A lock that only empties the key ring, or a switch made from the lock
    // screen, ends no session: that screen's choices stand.
    #[test]
    fn an_end_with_no_session_open_drops_nothing() {
        let grants = PathGrants::default();
        let backup = scratch("ring-lock");
        grants.pick(&backup, Purpose::Restore);
        grants.session_ended();
        assert!(grants.take(&backup, Purpose::Restore));
    }

    // The restore step's choice is read again after a mistyped password and
    // spent once; spent or not, it is gone once a vault opens, so whether the
    // restore spends it before or after opening the vault it made decides
    // nothing.
    #[test]
    fn a_restore_grant_lasts_until_it_is_spent_or_a_vault_opens() {
        let grants = PathGrants::default();
        let spent = scratch("restore-spent");
        grants.pick(&spent, Purpose::Restore);
        assert!(grants.holds(&spent, Purpose::Restore));
        assert!(grants.holds(&spent, Purpose::Restore));
        assert!(grants.take(&spent, Purpose::Restore));
        assert!(!grants.holds(&spent, Purpose::Restore));

        let unspent = scratch("restore-unspent");
        grants.pick(&unspent, Purpose::Restore);
        grants.session_opened();
        assert!(grants.held.lock().unwrap().paths.is_empty());
    }

    // Double-clicking a backup launches the app locked: it waits for the
    // unlock, where Settings › Import takes it — or for a first-run restore.
    #[test]
    fn a_backup_opened_from_the_os_while_locked_waits_for_the_next_session() {
        let grants = PathGrants::default();
        let backup = scratch("opened-at-launch");
        grants.grant_opened(&backup).unwrap();
        assert!(grants.holds(&backup, Purpose::Restore), "a fresh install");
        assert!(!grants.holds(&backup, Purpose::Backup));
        // A switch made from the lock screen ends no session.
        grants.session_ended();
        grants.session_opened();
        assert!(!grants.take(&backup, Purpose::Attachment), "a backup only");
        assert!(!grants.take(&backup, Purpose::Restore));
        assert!(grants.take(&backup, Purpose::Backup));

        // The next session, not any later one.
        let unused = scratch("opened-unused");
        grants.session_ended();
        grants.grant_opened(&unused).unwrap();
        grants.session_opened();
        grants.session_ended();
        grants.session_opened();
        assert!(!grants.take(&unused, Purpose::Backup));

        // Nor into a vault that takes over from that one (made or restored
        // beside it) without a lock between.
        let replaced = scratch("opened-replaced");
        grants.session_ended();
        grants.grant_opened(&replaced).unwrap();
        grants.session_opened();
        assert!(grants.holds(&replaced, Purpose::Backup));
        grants.session_opened();
        assert!(!grants.take(&replaced, Purpose::Backup));

        // Opened with a vault open, it is that session's like any choice.
        let unlocked = scratch("opened-unlocked");
        grants.grant_opened(&unlocked).unwrap();
        grants.session_ended();
        grants.session_opened();
        assert!(!grants.take(&unlocked, Purpose::Backup));
    }

    #[test]
    fn a_hand_over_that_cannot_be_resolved_is_an_error_and_held_nowhere() {
        let grants = PathGrants::default();
        let missing = Path::new("/definitely/not/here.rowel");
        assert!(grants.grant_opened(missing).is_err());
        assert!(grants.held.lock().unwrap().paths.is_empty());
    }
}
