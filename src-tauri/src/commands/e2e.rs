//! Per-spec state reset for the E2E suite.
//!
//! The suite runs ONE app process against ONE data dir for the whole run, so
//! without a reset seam every spec inherits whatever vault the previous spec
//! left behind. This command gives each spec an explicit starting state.
//!
//! Triple-gated, because a "wipe the data dir" command is exactly the thing that
//! must never reach a user:
//!   1. `#[cfg(debug_assertions)]` on the module *and* on the `invoke_handler`
//!      registration in `lib.rs` — a release binary does not contain it at all.
//!      (The suite drives the debug binary, which is also the only build that
//!      carries the in-app WebDriver server.)
//!   2. `ROWEL_E2E=1` must be set — a developer's own `tauri dev` run is a
//!      debug build too, and must not expose this to a stray `invoke`.
//!   3. `ROWEL_DB_DIR` must be set — the wipe targets *that* dir and nothing
//!      else, so even a debug binary run by hand can never touch the real
//!      app-data dir (which is what `storage::app_dir` falls back to).

use std::fs;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, State};

use crate::error::{Error, Result};
use crate::session::create_vault;
use crate::settings;
use crate::state::AppState;

// The state a spec wants to start from.
enum ResetMode {
    // No vault on disk: the app boots into the first-run setup choice screen.
    Pristine,
    // A freshly created, entry-less vault, left locked: the app boots into unlock.
    Empty,
}

impl ResetMode {
    fn parse(mode: &str) -> Result<Self> {
        match mode {
            "pristine" => Ok(Self::Pristine),
            "empty" => Ok(Self::Empty),
            other => Err(Error::Other(format!("unknown e2e reset mode: {other}"))),
        }
    }
}

// Gates 2 and 3. Returns the one directory this command is allowed to erase.
// `storage::app_dir` returns `ROWEL_DB_DIR` verbatim when it is set, so the dir
// resolved here is by construction the same one the app reads and writes.
fn e2e_data_dir() -> Result<PathBuf> {
    if std::env::var("ROWEL_E2E").as_deref() != Ok("1") {
        return Err(Error::Other("e2e_reset requires ROWEL_E2E=1".into()));
    }
    match std::env::var("ROWEL_DB_DIR") {
        Ok(dir) if !dir.is_empty() => Ok(PathBuf::from(dir)),
        _ => Err(Error::Other("e2e_reset requires ROWEL_DB_DIR".into())),
    }
}

// Empty the directory without removing the directory itself (the app already
// holds its path). Covers the DB triple, both sidecars, the biometric marker,
// the preferences file, cached icons, sync scratch — everything, so no spec
// inherits a stray file.
fn wipe_dir(dir: &Path) -> Result<()> {
    if !dir.exists() {
        fs::create_dir_all(dir)?;
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        // `file_type` here does not follow symlinks, so a link is unlinked
        // rather than recursed into.
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        } else {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Reset the E2E data dir to `mode`. `password` is required for `"empty"`.
///
/// The frontend reloads itself after this resolves (see `src/lib/e2e.ts`), so
/// the app re-runs `app_status` against the state left here.
#[tauri::command]
pub fn e2e_reset(
    mode: String,
    password: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<()> {
    let dir = e2e_data_dir()?;
    let mode = ResetMode::parse(&mode)?;

    // Drop the session BEFORE touching the files. It owns the open SQLCipher
    // connection; deleting the DB out from under a live handle leaves the
    // connection writing WAL frames for a file that no longer exists, and the
    // next `create_vault` would open beside them. Through `session::lock`, so a
    // suite whose spec ran against an unlocked app sees the same `vault:locked`
    // a real lock emits rather than a webview left pointing at a gone vault.
    crate::session::lock(&app);
    // The vault the published identities named is about to go.
    crate::credential_identities::clear();

    wipe_dir(&dir)?;
    // The preferences file went with it, so the copy held in memory has to
    // follow — otherwise the next `app_status` hands the frontend the theme and
    // sort order the previous spec left behind.
    settings::boot(&app);
    // The wipe took `workspaces.json` with it, so the app is back to one
    // workspace — and the vault created below has to land in the root.
    *state.active_workspace.lock().unwrap() = crate::workspace::PRIMARY_ID.to_string();

    match mode {
        ResetMode::Pristine => Ok(()),
        ResetMode::Empty => {
            let password = password
                .ok_or_else(|| Error::Other("e2e_reset mode \"empty\" needs a password".into()))?;
            // Same path the real setup command uses, so the vault an "empty"
            // spec unlocks is byte-for-byte what a user's first run produces.
            let (_key, store) = create_vault(&app, &password)?;
            // Leave the session locked: the reload lands on the unlock screen.
            drop(store);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tmp_dir() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rowel-e2e-reset-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_accepts_only_the_two_known_modes() {
        assert!(matches!(
            ResetMode::parse("pristine"),
            Ok(ResetMode::Pristine)
        ));
        assert!(matches!(ResetMode::parse("empty"), Ok(ResetMode::Empty)));
        assert!(ResetMode::parse("Pristine").is_err());
        assert!(ResetMode::parse("").is_err());
    }

    #[test]
    fn wipe_removes_files_and_subdirs_but_keeps_the_dir() {
        let dir = tmp_dir();
        fs::write(dir.join("vault.db"), "db").unwrap();
        fs::write(dir.join("vault.db-wal"), "wal").unwrap();
        fs::write(dir.join("vault.kdf.json"), "{}").unwrap();
        fs::create_dir_all(dir.join("icons")).unwrap();
        fs::write(dir.join("icons/example.png"), "png").unwrap();

        wipe_dir(&dir).unwrap();

        assert!(dir.exists(), "the data dir itself must survive");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn wipe_recreates_a_missing_dir() {
        let dir = tmp_dir();
        fs::remove_dir_all(&dir).unwrap();
        wipe_dir(&dir).unwrap();
        assert!(dir.is_dir());
    }
}
