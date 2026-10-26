//! The two halves of the sync exchange that need no app around them: the
//! `.rowel` pack a device pushes ([`pack`]) and the reader that opens the
//! SQLCipher snapshot inside one ([`snapshot`]).
//!
//! They live in the core rather than in the app because a Rowel Server runs
//! the receiving half of the sync engine over its own store — unpack what a
//! device pushed, read the snapshot out, merge it, pack its state back — and a
//! server links the core, never the app: the app is GPL, the core is MIT, and
//! one implementation of the format is the only way two sides ever agree on
//! it. The engine that decides what to pull and push stays with the app; it
//! is the part that knows about sessions, providers and the account, none of
//! which a server has.

pub mod pack;
pub mod snapshot;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A scratch database under a directory of the caller's choosing, named for
/// this process and this call, and removed — `-wal` and `-shm` siblings
/// included — however the caller returns, the `?` early returns too.
///
/// Both halves need one: SQLite's backup API writes a snapshot to a file, not
/// a buffer, and SQLCipher reads one from a file. Unique per call so two
/// concurrent users (a scheduled sync and a manual one, a pack and a pull)
/// cannot write over each other's file. The siblings go with the main file
/// because a stale `-wal` beside a missing one is its own corruption, not a
/// clean slate, and the next database under the same name would inherit it.
pub(crate) struct Scratch(PathBuf);

impl Scratch {
    /// Claim a fresh path for `role` under `dir`, creating `dir` if it is not
    /// there. Nothing is on disk yet: the file is the caller's to write.
    pub(crate) fn new(dir: &Path, role: &str) -> std::io::Result<Self> {
        static N: AtomicU64 = AtomicU64::new(0);
        fs::create_dir_all(dir)?;
        Ok(Self(dir.join(format!(
            "rowel-sync-{role}-{}-{}.db",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ))))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        remove_db_files(&self.0);
    }
}

// Remove a SQLite database and the WAL/SHM siblings SQLite writes beside it,
// ignoring what is already absent. The main file alone is not the database —
// see [`Scratch`] for why the siblings must go with it.
fn remove_db_files(base: &Path) {
    let sibling = |suffix: &str| {
        let mut name = base.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name)
    };
    for file in [base.to_path_buf(), sibling("-wal"), sibling("-shm")] {
        let _ = fs::remove_file(file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rowel-scratch-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // The guard is what makes every exit — including the `?` early returns —
    // leave no encrypted scratch file behind.
    #[test]
    fn the_scratch_guard_removes_the_snapshot_and_its_siblings() {
        let dir = tmp_dir();
        let base = dir.join("snap.db");
        let siblings = [
            base.clone(),
            dir.join("snap.db-wal"),
            dir.join("snap.db-shm"),
        ];
        for p in &siblings {
            fs::write(p, b"x").unwrap();
        }

        drop(Scratch(base));

        for p in &siblings {
            assert!(!p.exists(), "{} survived the guard", p.display());
        }
    }

    #[test]
    fn two_scratch_files_never_share_a_path() {
        let dir = tmp_dir();
        let a = Scratch::new(&dir, "pack").unwrap();
        let b = Scratch::new(&dir, "pack").unwrap();
        assert_ne!(a.path(), b.path());
    }
}
