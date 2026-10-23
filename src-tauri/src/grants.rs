//! Paths the user has pointed the app at.
//!
//! A command that reads a local file (`scan_image`, `read_env_file`,
//! `attachment_add`, `import_entries`, `import_swftx`,
//! `setup_restore_from_file`, `setup_restore_from_legacy`) gets its path from the webview, and the webview's word is not authorization: a
//! script that got to run there could name any file on disk — another
//! project's `.env`, a photo in the user's library — and have the backend hand
//! it back. What the extension and shape checks in those commands decide is
//! *what kind of file* a path names, never *whether the user chose it*.
//!
//! Two events prove the user chose a file, and both reach Rust before the
//! webview ever sees the path: the OS file dialog, when it is opened from Rust
//! (`commands::tools::pick_file`), and a drop on the window, which the OS
//! delivers as a window event (`window::create`). Each grants the path here;
//! the reading command then has to `take` it. A path nobody granted is refused
//! however well it is named.
//!
//! A grant carries the purpose it was made for, because the framing the user
//! answered is part of what they agreed to. The env picker offers every file on
//! disk, the image picker only what the scanner opens; a path chosen from a
//! dialog that said "pick an environment file" was not offered up to be OCRed
//! for the card number or the passport fields in it. So the purpose is recorded
//! with the path and the reading command has to name the same one: a path
//! picked as an env file can only be read as one.
//!
//! Grants are one-shot — a take consumes them — and few: a bounded set, oldest
//! out, since a grant that was never read is a path the user pointed at and
//! then walked away from. Kept canonical so a grant cannot be sidestepped or
//! spent through a spelling of the path other than the one it was made under.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::Error;

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
    /// A `.rowel` or `.swftx` backup: what `commands::vault::import_swftx`
    /// merges and `commands::setup::setup_restore_from_file` /
    /// `setup_restore_from_legacy` restore, each only once the backup's own
    /// password has opened it.
    Backup,
}

/// The refusal every reader gives a path nobody granted it.
pub fn not_chosen() -> Error {
    Error::Unsupported("this file was not chosen in the app".into())
}

/// One path the user chose, and how.
struct Grant {
    path: PathBuf,
    purpose: Purpose,
    /// Dropped on the window rather than picked from a dialog.
    dropped: bool,
}

/// How many grants are held at once. A user picks or drops one file at a time
/// and every grant is spent by the read that follows, so anything near the cap
/// is grants nothing collected.
const CAP: usize = 64;

#[derive(Default)]
pub struct PathGrants {
    // Oldest first, so the bound evicts from the front.
    paths: Mutex<VecDeque<Grant>>,
}

impl PathGrants {
    /// Record that the user chose `path` to be read as `purpose`. A path that
    /// cannot be canonicalized (it does not exist) is not recorded: there is
    /// nothing to read.
    ///
    /// One grant per path: choosing the same file again — under this purpose or
    /// another — replaces what was held for it, so the last thing the user was
    /// asked is what the path stands for.
    pub fn grant(&self, path: &Path, purpose: Purpose) {
        self.insert(path, purpose, false);
    }

    /// [`PathGrants::grant`] for a file dropped on the window, `purpose` being
    /// what its kind makes it. A drop is also spendable by the readers that take
    /// a file into the vault — as an attachment, an export to import or a
    /// backup to merge or restore: the user handed the file to the app itself
    /// rather than to a dialog with a framing of its own, and which of those it
    /// becomes is the drop target's to say. None of them hands back more than
    /// the kind's own reader would — the env reader returns the whole text.
    pub fn grant_drop(&self, path: &Path, purpose: Purpose) {
        self.insert(path, purpose, true);
    }

    fn insert(&self, path: &Path, purpose: Purpose, dropped: bool) {
        let Ok(path) = path.canonicalize() else {
            return;
        };
        let mut paths = self.paths.lock().unwrap();
        paths.retain(|held| held.path != path);
        if paths.len() >= CAP {
            paths.pop_front();
        }
        paths.push_back(Grant {
            path,
            purpose,
            dropped,
        });
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
        let mut paths = self.paths.lock().unwrap();
        match position(&paths, &path, purpose) {
            Some(at) => {
                paths.remove(at);
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
        position(&self.paths.lock().unwrap(), &path, purpose).is_some()
    }
}

/// Where the grant spendable for `path` as `purpose` is held, if anywhere.
fn position(paths: &VecDeque<Grant>, path: &Path, purpose: Purpose) -> Option<usize> {
    paths.iter().position(|held| {
        held.path == path
            && (held.purpose == purpose
                || (held.dropped
                    && matches!(
                        purpose,
                        Purpose::Attachment | Purpose::Import | Purpose::Backup
                    )))
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
        let grants = PathGrants::default();
        let path = scratch("once");
        grants.grant(&path, Purpose::Image);
        assert!(grants.take(&path, Purpose::Image));
        assert!(!grants.take(&path, Purpose::Image));
    }

    #[test]
    fn a_path_nobody_granted_is_refused() {
        let grants = PathGrants::default();
        let path = scratch("ungranted");
        assert!(!grants.take(&path, Purpose::Image));
        assert!(!grants.take(Path::new("/definitely/not/here"), Purpose::Env));
    }

    #[test]
    fn a_grant_is_spent_through_any_spelling_of_the_path() {
        let grants = PathGrants::default();
        let path = scratch("spelled");
        let dir = path.parent().unwrap();
        let roundabout = dir
            .join("..")
            .join(dir.file_name().unwrap())
            .join("spelled");
        grants.grant(&roundabout, Purpose::Image);
        assert!(grants.take(&path, Purpose::Image));
        assert!(!grants.take(&roundabout, Purpose::Image));
    }

    #[test]
    fn the_oldest_grant_goes_first_when_the_set_is_full() {
        let grants = PathGrants::default();
        let first = scratch("evicted");
        grants.grant(&first, Purpose::Env);
        let rest: Vec<_> = (0..CAP).map(|i| scratch(&format!("kept-{i}"))).collect();
        for path in &rest {
            grants.grant(path, Purpose::Env);
        }
        assert!(!grants.take(&first, Purpose::Env));
        for path in &rest {
            assert!(grants.take(path, Purpose::Env));
        }
    }

    #[test]
    fn regranting_a_held_path_keeps_one_grant_for_it() {
        let grants = PathGrants::default();
        let path = scratch("twice");
        grants.grant(&path, Purpose::Env);
        grants.grant(&path, Purpose::Env);
        assert!(grants.take(&path, Purpose::Env));
        assert!(!grants.take(&path, Purpose::Env));
    }

    #[test]
    fn a_path_picked_as_an_env_file_cannot_be_scanned() {
        let grants = PathGrants::default();
        let path = scratch("env-not-image");
        grants.grant(&path, Purpose::Env);
        assert!(!grants.take(&path, Purpose::Image));
        // The refused take spent nothing: the reader it was picked for still
        // has its grant.
        assert!(grants.take(&path, Purpose::Env));
    }

    // A drop can be attached whatever its kind; a picked file only from the
    // attachment picker, and an attachment pick reads as nothing else.
    #[test]
    fn a_dropped_file_can_be_attached_and_a_picked_one_only_as_picked() {
        let grants = PathGrants::default();
        let dropped = scratch("dropped");
        grants.grant_drop(&dropped, Purpose::Env);
        assert!(grants.take(&dropped, Purpose::Attachment));
        assert!(!grants.take(&dropped, Purpose::Env), "spent by the attach");

        let picked = scratch("picked-env");
        grants.grant(&picked, Purpose::Env);
        assert!(!grants.take(&picked, Purpose::Attachment));

        let attached = scratch("picked-attachment");
        grants.grant(&attached, Purpose::Attachment);
        assert!(!grants.take(&attached, Purpose::Env));
        assert!(!grants.take(&attached, Purpose::Image));
        assert!(grants.take(&attached, Purpose::Attachment));
    }

    #[test]
    fn a_dropped_file_can_be_imported_or_restored() {
        let grants = PathGrants::default();
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
        let grants = PathGrants::default();
        let export = scratch("picked-export");
        grants.grant(&export, Purpose::Import);
        assert!(!grants.take(&export, Purpose::Backup));
        assert!(!grants.take(&export, Purpose::Env));
        assert!(grants.take(&export, Purpose::Import));

        let backup = scratch("picked-backup");
        grants.grant(&backup, Purpose::Backup);
        assert!(!grants.take(&backup, Purpose::Import));
        assert!(!grants.take(&backup, Purpose::Attachment));
        assert!(grants.take(&backup, Purpose::Backup));
    }

    #[test]
    fn holding_a_grant_spends_nothing() {
        let grants = PathGrants::default();
        let path = scratch("held");
        assert!(!grants.holds(&path, Purpose::Import));
        grants.grant(&path, Purpose::Import);
        assert!(grants.holds(&path, Purpose::Import));
        assert!(grants.holds(&path, Purpose::Import));
        assert!(!grants.holds(&path, Purpose::Backup));
        assert!(grants.take(&path, Purpose::Import));
        assert!(!grants.holds(&path, Purpose::Import));
    }

    #[test]
    fn regranting_under_another_purpose_replaces_the_first() {
        let grants = PathGrants::default();
        let path = scratch("repurposed");
        grants.grant(&path, Purpose::Image);
        grants.grant(&path, Purpose::Env);
        assert!(!grants.take(&path, Purpose::Image));
        assert!(grants.take(&path, Purpose::Env));
        assert!(!grants.take(&path, Purpose::Env));
    }
}
