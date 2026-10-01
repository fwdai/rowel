//! Writing an export to wherever the user says, on two platforms that mean
//! opposite things by "save dialog".
//!
//! Desktop hands back a destination to write to. iOS has no such API: its only
//! export mechanism is "move this file to a place the user picks", so the
//! plugin fakes the desktop shape by creating an *empty* file in the app's
//! Documents directory and exporting that (`tauri-plugin-dialog` 2.7.2,
//! `ios/Sources/DialogPlugin.swift`: `saveFileDialog` at line 138 writes `""`
//! to `<Documents>/<fileName>` at lines 148-154, but only if nothing is there
//! yet, then hands that file to
//! `UIDocumentPickerViewController(url:in:.exportToService)` at line 168, which
//! copies it at pick time). The copy has therefore already happened by the time
//! the path comes back, so writing to it achieves nothing: on iOS the content
//! has to be in place *before* the dialog opens.
//!
//! Both callers (`vault::export_vault`, `import::export_entries`) go through
//! [`save_export`], so that ordering lives in exactly one place.

use std::path::{Path, PathBuf};

use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;
use zeroize::Zeroize;

use crate::error::{Error, Result};

/// Ask the user where `file_name` should go and put `bytes` there.
///
/// `filter` labels the file-type filter the desktop dialog shows; the extension
/// it filters on is `file_name`'s. Returns the chosen path, or `None` if the
/// dialog was dismissed.
pub async fn save_export(
    app: &AppHandle,
    file_name: &str,
    filter: &str,
    bytes: Vec<u8>,
) -> Result<Option<PathBuf>> {
    let extension = Path::new(file_name)
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();

    #[cfg(mobile)]
    {
        use tauri::Manager;

        let dir = app
            .path()
            .document_dir()
            .map_err(|e| Error::Other(e.to_string()))?;
        let (staged, asked) = staging(&dir, file_name);
        stage_then(&staged, file_name, bytes, || {
            ask(app, &asked, filter, &extension)
        })
        .await
    }
    #[cfg(desktop)]
    {
        let Some(dest) = ask(app, file_name, filter, &extension).await? else {
            return Ok(None);
        };
        let dest = with_extension(dest, &extension);
        write_and_scrub(&dest, bytes)?;
        Ok(Some(dest))
    }
}

/// Ask where a plaintext `.env` should go and write `text` there, readable by
/// its owner alone (0600 on unix). Desktop only: the mobile picker copies the
/// file itself and so cannot be told what mode to give it.
///
/// Unlike [`save_export`] this applies no file-type filter — `.env` has no
/// extension in the dialog's sense, `.env.production` has the wrong one — and
/// does not force one onto the chosen name for the same reason.
#[cfg(desktop)]
pub async fn save_private_text(
    app: &AppHandle,
    file_name: &str,
    text: String,
) -> Result<Option<PathBuf>> {
    let Some(dest) = ask(app, file_name, "", "").await? else {
        return Ok(None);
    };
    write_and_scrub(&dest, text.into_bytes())?;
    Ok(Some(dest))
}

/// Where one export stages inside `dir`, and the name to ask the dialog for so
/// that it exports exactly that: a directory of its own, plus `file_name`
/// prefixed with it.
///
/// Staging directly at `<Documents>/vault.swftx` made the export the owner of a
/// path it had not created — it truncated whatever was there and then deleted
/// it — and `<Documents>` is itself one of the destinations the iOS picker
/// offers, so a user who exported there watched the export vanish. A per-export
/// directory is the only part of the path free to change: `file_name` is what
/// the picker labels the file with, and the plugin resolves what to export from
/// the name we ask for (`<Documents>/<fileName>`), which is why the directory
/// has to travel in that name rather than beside it.
#[cfg(any(mobile, test))]
fn staging(dir: &Path, file_name: &str) -> (PathBuf, String) {
    let unique = format!("export-{}", crate::store::migrate::new_entry_id());
    (dir.join(&unique), format!("{unique}/{file_name}"))
}

/// Put the bytes at `file_name` inside `staged`, run the dialog, then take
/// `staged` away again however the dialog ended — the `.csv` export is
/// plaintext, so no copy of it outlives the one dialog it was staged for.
///
/// Split out from [`save_export`] (and compiled on every platform) so the
/// ordering the iOS dialog demands can be tested without one.
#[cfg(any(mobile, test))]
async fn stage_then<F, Fut>(
    staged: &Path,
    file_name: &str,
    bytes: Vec<u8>,
    dialog: F,
) -> Result<Option<PathBuf>>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Option<PathBuf>>>,
{
    let _staged = Staged::write(staged, file_name, bytes)?;
    dialog().await
}

/// A directory that exists only as long as this value does.
///
/// The removal lives in `Drop` rather than after the dialog so that *every* way
/// out of the scope — a dismissed dialog, a failed dialog, a write that ran out
/// of disk halfway, a panic — takes the directory with it. Cleanup that has to
/// be remembered at each exit is cleanup that one of them will forget.
#[cfg(any(mobile, test))]
struct Staged(PathBuf);

#[cfg(any(mobile, test))]
impl Staged {
    /// Claim the directory first, then create and fill it: a create or a write
    /// that fails partway has already left something on disk, and only a guard
    /// that exists by then removes it.
    fn write(dir: &Path, file_name: &str, bytes: Vec<u8>) -> Result<Self> {
        let staged = Self(dir.to_path_buf());
        std::fs::create_dir_all(dir)?;
        write_and_scrub(&dir.join(file_name), bytes)?;
        Ok(staged)
    }
}

#[cfg(any(mobile, test))]
impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// Present the save dialog off the main thread; blocking on it there would
// deadlock the very event loop the dialog needs. An empty `filter` asks for no
// file-type filter at all.
async fn ask(
    app: &AppHandle,
    file_name: &str,
    filter: &str,
    extension: &str,
) -> Result<Option<PathBuf>> {
    let app = app.clone();
    let file_name = file_name.to_string();
    let filter = filter.to_string();
    let extension = extension.to_string();
    let chosen = tauri::async_runtime::spawn_blocking(move || {
        let dialog = app.dialog().file().set_file_name(file_name);
        let dialog = if filter.is_empty() {
            dialog
        } else {
            dialog.add_filter(filter, &[extension.as_str()])
        };
        dialog.blocking_save_file()
    })
    .await
    .map_err(|e| Error::Other(e.to_string()))?;
    // `into_path` also resolves the `file://` URL the mobile plugin hands back.
    Ok(chosen.and_then(|f| f.into_path().ok()))
}

/// A destination given without one keeps the extension the export needs. A
/// file that never had one (an attachment called `id_ed25519`) is left as the
/// user named it.
pub fn with_extension(dest: PathBuf, extension: &str) -> PathBuf {
    match dest.extension() {
        _ if extension.is_empty() => dest,
        Some(e) if e == extension => dest,
        _ => dest.with_extension(extension),
    }
}

/// Put plaintext on disk and scrub the copy in memory on every exit.
///
/// An export is the whole vault in the clear, so on desktop it goes through the
/// shared durable writer that a saved `.env` gets: the owner-only temp sibling
/// (`0600` on Unix, a protected owner-and-SYSTEM DACL on Windows) is complete
/// and fsynced before it atomically replaces the destination, so the bytes are
/// never briefly readable through the folder's permissions and a failed
/// overwrite leaves the old file whole. `atomic_write_private` is desktop-only —
/// on mobile the only thing written here is the staging copy inside the app's
/// own sandbox, which the picker consumes and [`Staged`] then removes.
pub(crate) fn write_and_scrub(dest: &Path, mut bytes: Vec<u8>) -> Result<()> {
    #[cfg(desktop)]
    let result = crate::storage::atomic_write_private(dest, &bytes);
    #[cfg(not(desktop))]
    let result = std::fs::write(dest, &bytes).map_err(Error::from);
    bytes.zeroize();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // A `.env` on disk is a plaintext secret; whatever the umask says, nobody
    // but the owner gets to read it — including when the user picked a file
    // that already existed with looser permissions.
    #[cfg(all(desktop, unix))]
    #[test]
    fn write_private_leaves_the_file_owner_readable_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("rowel-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join(".env.production");

        write_and_scrub(&dest, b"KEY=value\n".to_vec()).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"KEY=value\n");
        assert_eq!(
            std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
            0o600
        );

        // Overwriting a looser file tightens it.
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_and_scrub(&dest, b"KEY=other\n".to_vec()).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"KEY=other\n");
        assert_eq!(
            std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
            0o600
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    // An export is every entry in the clear — a bigger secret than one `.env`,
    // and written through the same writer, so it lands with the same mode.
    #[cfg(all(desktop, unix))]
    #[test]
    fn an_export_lands_owner_readable_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("rowel-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("rowel-export.csv");

        write_and_scrub(&dest, b"title,username,password\n".to_vec()).unwrap();
        assert_eq!(
            std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
            0o600
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    // What iOS requires: the file the picker is about to export must already
    // hold the export by the time the dialog runs, and must be gone after it.
    #[tokio::test]
    async fn staging_writes_before_the_dialog_and_cleans_up_after() {
        let dir = std::env::temp_dir().join(format!("rowel-save-{}", std::process::id()));
        let (staged, _) = staging(&dir, "vault.swftx");

        let chosen = stage_then(
            &staged,
            "vault.swftx",
            b"exported bytes".to_vec(),
            || async {
                let file = staged.join("vault.swftx");
                assert_eq!(std::fs::read(file).unwrap(), b"exported bytes");
                Ok(Some(PathBuf::from("/somewhere/vault.swftx")))
            },
        )
        .await
        .unwrap();

        assert_eq!(chosen, Some(PathBuf::from("/somewhere/vault.swftx")));
        assert!(!staged.exists(), "the staged copy outlived the dialog");
        std::fs::remove_dir_all(&dir).ok();
    }

    // The name the dialog is asked for is the only thing the iOS plugin uses to
    // decide what to export, so it has to resolve — from the same directory the
    // staging is relative to — to the staged file itself, under the plain name
    // the user is shown.
    #[test]
    fn the_asked_name_resolves_to_the_staged_file() {
        let dir = Path::new("/documents");
        let (staged, asked) = staging(dir, "vault.swftx");

        assert_eq!(dir.join(&asked), staged.join("vault.swftx"));
        assert_eq!(Path::new(&asked).file_name().unwrap(), "vault.swftx");
        assert_ne!(staging(dir, "vault.swftx").0, staged);
    }

    // The path the export takes over is one it created, so an export that lands
    // in the directory the app exports *from* leaves the neighbours alone —
    // staging used to truncate `<Documents>/vault.swftx` and then remove it.
    #[tokio::test]
    async fn staging_leaves_the_rest_of_the_directory_alone() {
        let dir = std::env::temp_dir().join(format!("rowel-save-nbr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let kept = dir.join("vault.swftx");
        std::fs::write(&kept, b"an export the user already saved here").unwrap();
        let (staged, _) = staging(&dir, "vault.swftx");

        let chosen = stage_then(
            &staged,
            "vault.swftx",
            b"exported bytes".to_vec(),
            || async { Ok(Some(dir.join("vault.swftx"))) },
        )
        .await
        .unwrap();

        assert!(chosen.is_some());
        assert_eq!(
            std::fs::read(&kept).unwrap(),
            b"an export the user already saved here"
        );
        assert!(!staged.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    // Nor a dialog that failed outright.
    #[tokio::test]
    async fn staging_cleans_up_after_a_failed_dialog() {
        let dir = std::env::temp_dir().join(format!("rowel-save-fail-{}", std::process::id()));
        let (staged, _) = staging(&dir, "rowel-export.csv");

        let result = stage_then(
            &staged,
            "rowel-export.csv",
            b"name,password".to_vec(),
            || async { Err(Error::Other("picker crashed".into())) },
        )
        .await;

        assert!(result.is_err());
        assert!(!staged.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    // A write that fails is the one exit the old "delete after the dialog"
    // ordering missed: the guard exists before the bytes go down, so whatever
    // the failed write left in the directory is removed exactly like a complete
    // export. The refusal comes from a read-only *directory* rather than a
    // read-only file, because the desktop writer replaces its destination by
    // rename — which a read-only file does not stand in the way of. Unix-only
    // because that is the portable way to make the write fail, and Windows then
    // refuses to remove the directory as well.
    #[cfg(unix)]
    #[test]
    fn a_failed_write_leaves_nothing_behind() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("rowel-save-partial-{}", std::process::id()));
        let (staged, _) = staging(&dir, "rowel-export.csv");
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o555)).unwrap();

        let result = Staged::write(&staged, "rowel-export.csv", b"name,password".to_vec());

        assert!(
            result.is_err(),
            "a read-only directory should have refused the write"
        );
        assert!(
            !staged.exists(),
            "a staging directory survived its failed write"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    // A dismissed dialog must not leave the plaintext behind either.
    #[tokio::test]
    async fn staging_cleans_up_after_a_cancelled_dialog() {
        let dir = std::env::temp_dir().join(format!("rowel-save-cancel-{}", std::process::id()));
        let (staged, _) = staging(&dir, "rowel-export.csv");

        let chosen = stage_then(
            &staged,
            "rowel-export.csv",
            b"name,password".to_vec(),
            || async { Ok(None) },
        )
        .await
        .unwrap();

        assert!(chosen.is_none());
        assert!(!staged.exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
