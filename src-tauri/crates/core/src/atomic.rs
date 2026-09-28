//! Durable, atomic file replacement: what every small file the vault's
//! directory holds beside the database is written with — the sidecars, the
//! preferences, the workspace registry, the sealed secrets.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

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

#[cfg(test)]
mod tests {
    use super::{atomic_replace_with, atomic_write_file};
    use std::fs;
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};

    fn tmp_sidecar() -> PathBuf {
        let dir = tempfile::tempdir().unwrap().keep();
        dir.join("vault.kdf.json")
    }

    fn tmp_sibling(path: &Path) -> PathBuf {
        let mut tmp = path.to_path_buf().into_os_string();
        tmp.push(".tmp");
        PathBuf::from(tmp)
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
}
