//! Writing an export to wherever the user says, on three platforms that each
//! mean something else by "save dialog".
//!
//! Desktop hands back a destination to write to. iOS has no such API: its only
//! export mechanism is "move this file to a place the user picks", so the
//! plugin fakes the desktop shape by creating an *empty* file in the app's
//! Documents directory and exporting that (`tauri-plugin-dialog` 2.8.4,
//! `ios/Sources/DialogPlugin.swift`: `saveFileDialog` at line 138 writes `""`
//! to `<Documents>/<fileName>` at lines 148-154, but only if nothing is there
//! yet, then hands that file to
//! `UIDocumentPickerViewController(url:in:.exportToService)` at line 168, which
//! copies it at pick time). The copy has therefore already happened by the time
//! the path comes back, so writing to it achieves nothing: on iOS the content
//! has to be in place *before* the dialog opens.
//!
//! Android asks first, as desktop does, but what it hands back is not a path.
//! The system picker creates the document and answers with its `content://`
//! URI (`android/src/main/java/DialogPlugin.kt` of the same plugin:
//! `saveFileDialog` starts `ACTION_CREATE_DOCUMENT`, `saveFileDialogResult`
//! resolves with `intent.data` as a string), which `std::fs` cannot open and
//! `FilePath::into_path` refuses, being no `file://` URL. The bytes go in
//! through a descriptor the content resolver opens for that URI
//! (`tauri-plugin-fs` 2.7.1, `src/android.rs`: `Fs::open` asks
//! `FsPlugin.getFileDescriptor`). Nothing is staged: the export goes from
//! memory into the document, and the app's own storage never holds it.
//!
//! Every caller (`vault::export_vault`, `import::export_entries`,
//! `attachments::attachment_save`) goes through [`save_export`], so each
//! platform's ordering lives in exactly one place.

use std::path::{Path, PathBuf};

use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, FilePath};
use zeroize::Zeroize;

use crate::error::{Error, Result};

/// Ask the user where `file_name` should go and put `bytes` there.
///
/// `filter` labels the file-type filter the desktop dialog shows; the extension
/// it filters on is `file_name`'s. Returns where the export went, or `None` if
/// the dialog was dismissed: the chosen path on desktop and iOS, and on
/// Android, where the destination is a URI that names nothing a person reads,
/// the name the document has there — which is all the webview shows of it.
pub async fn save_export(
    app: &AppHandle,
    file_name: &str,
    filter: &str,
    bytes: Vec<u8>,
) -> Result<Option<PathBuf>> {
    #[cfg(not(target_os = "android"))]
    let extension = Path::new(file_name)
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();

    #[cfg(target_os = "ios")]
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
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        use tauri_plugin_fs::FsExt;

        let handle = app.clone();
        let Some(dest) = ask_then_write(
            bytes,
            pick(app, file_name, Some((filter, ANY_TYPE))),
            move |dest: &FilePath| handle.fs().open(dest.clone(), document_options()),
        )
        .await?
        else {
            return Ok(None);
        };
        // The name is asked of the document rather than assumed: the picker
        // lets the user retype the one it was offered, and the provider alters
        // one that is taken. A provider that will not say leaves the name that
        // was offered. Off the runtime's workers, as the write is: the question
        // goes through the Android main thread to the provider.
        let (app, offered) = (app.clone(), file_name.to_string());
        crate::commands::blocking(move || {
            let name = app.path().file_name(&dest.to_string());
            Ok(Some(PathBuf::from(name.unwrap_or(offered))))
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
/// its owner alone (0600 on unix). Desktop only: neither phone's picker can be
/// told what mode to give the file — iOS copies it itself, Android has a
/// content provider create it.
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
#[cfg(any(target_os = "ios", test))]
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
#[cfg(any(target_os = "ios", test))]
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
#[cfg(any(target_os = "ios", test))]
struct Staged(PathBuf);

#[cfg(any(target_os = "ios", test))]
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

#[cfg(any(target_os = "ios", test))]
impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The one file type the Android dialog is asked for, whatever is being saved.
///
/// The plugin turns a filter's extensions into MIME types — an "extension"
/// with a `/` in it is taken for one already — and builds its
/// `ACTION_CREATE_DOCUMENT` intent from however many it is left with
/// (`DialogPlugin.kt`: `parseFiltersOption`, `saveFileDialog`). Left with
/// none — no filter, as an attachment has, or an extension Android knows no
/// type for, as `.rowel` — it sends an empty `EXTRA_MIME_TYPES`, and the system
/// picker then offers nowhere to save: it lists a storage root only when the
/// root's types overlap the ones asked for (AOSP DocumentsUI,
/// `ProvidersAccess.getMatchingRoots`). Exactly one type becomes the intent's
/// own and that extra is not sent. This is the one every root takes, and the
/// one under which the provider leaves the offered name as it is, extension or
/// none, where any other type has it check the extension against the type
/// (AOSP `FileUtils.splitFileName`).
#[cfg(target_os = "android")]
const ANY_TYPE: &str = "application/octet-stream";

/// How an export opens the document the Android dialog answered with: for
/// writing, and cut to nothing first — the picker also lets the user choose a
/// document that is already there, and one longer than the export would keep
/// its tail. The one constructor of them, which the tests open a file with too.
#[cfg(any(target_os = "android", test))]
fn document_options() -> tauri_plugin_fs::OpenOptions {
    let mut options = tauri_plugin_fs::OpenOptions::new();
    options.write(true).truncate(true);
    options
}

/// Run the dialog and, if it answered, open what it answered with and write
/// the bytes there: Android's order, where the answer is a document to be
/// opened rather than a path. Split out from [`save_export`] (and compiled for
/// the tests) so the order can be exercised without a picker.
///
/// The bytes are put under a guard before the dialog runs, so they are
/// scrubbed however this ends — a dialog dismissed or failed, a destination
/// that would not open, a write that stopped halfway.
///
/// Nothing here touches the app's own storage, so there is nothing of the
/// app's to take away again. The document is another matter: the picker has
/// created it by the time it answers, so a write that fails leaves it where
/// the user put it, empty or cut short, and the error is what says so —
/// removing a document is not something its descriptor can do.
#[cfg(any(target_os = "android", test))]
async fn ask_then_write<D, W>(
    bytes: Vec<u8>,
    dialog: impl std::future::Future<Output = Result<Option<D>>>,
    open: impl FnOnce(&D) -> std::io::Result<W> + Send + 'static,
) -> Result<Option<D>>
where
    D: Send + 'static,
    W: std::io::Write,
{
    let bytes = zeroize::Zeroizing::new(bytes);
    let Some(dest) = dialog.await? else {
        return Ok(None);
    };
    // Off the runtime's workers: the open is a round trip through the Android
    // main thread, and the write waits on whichever provider owns the document.
    crate::commands::blocking(move || {
        open(&dest)?.write_all(&bytes)?;
        Ok(Some(dest))
    })
    .await
}

// Present the save dialog off the main thread; blocking on it there would
// deadlock the very event loop the dialog needs. `filter` is the file-type
// filter's label and the one type it lets through, `None` for no filter at
// all. The answer is the plugin's own: a path on desktop, a `file://` URL on
// iOS, a `content://` URI on Android.
async fn pick(
    app: &AppHandle,
    file_name: &str,
    filter: Option<(&str, &str)>,
) -> Result<Option<FilePath>> {
    let app = app.clone();
    let file_name = file_name.to_string();
    let filter = filter.map(|(label, kind)| (label.to_string(), kind.to_string()));
    tauri::async_runtime::spawn_blocking(move || {
        let dialog = app.dialog().file().set_file_name(file_name);
        let dialog = match filter {
            Some((label, kind)) => dialog.add_filter(label, &[kind.as_str()]),
            None => dialog,
        };
        dialog.blocking_save_file()
    })
    .await
    .map_err(|e| Error::Other(e.to_string()))
}

// [`pick`] where the dialog answers with somewhere `std::fs` can write. An
// empty `filter` asks for no file-type filter at all.
#[cfg(not(target_os = "android"))]
async fn ask(
    app: &AppHandle,
    file_name: &str,
    filter: &str,
    extension: &str,
) -> Result<Option<PathBuf>> {
    let filter = (!filter.is_empty()).then_some((filter, extension));
    let chosen = pick(app, file_name, filter).await?;
    // `into_path` also resolves the `file://` URL the iOS plugin hands back.
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
/// overwrite leaves the old file whole. `atomic_write_private` is desktop-only.
/// On iOS the only thing written here is the staging copy inside the app's own
/// sandbox, which the picker consumes and [`Staged`] then removes; on Android
/// an export is not written through here at all (see `ask_then_write`).
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

    // What Android requires: the destination is whatever the dialog answered
    // with, opened only then, and the export is what it holds afterwards. The
    // picker has created the document by the time it answers, hence the file
    // already there — with something in it, as one the user chose to replace
    // has. Opened with the options the Android path opens it with, so an export
    // that stopped truncating would keep the older export's tail here.
    #[tokio::test]
    async fn the_bytes_go_into_what_the_dialog_answered_with() {
        let dir = std::env::temp_dir().join(format!("rowel-save-doc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("rowel-export.csv");
        std::fs::write(&dest, b"an older and longer export").unwrap();

        let chosen = ask_then_write(
            b"name,password".to_vec(),
            async { Ok(Some(dest.clone())) },
            |dest: &PathBuf| std::fs::OpenOptions::from(document_options()).open(dest),
        )
        .await
        .unwrap();

        assert_eq!(chosen, Some(dest.clone()));
        assert_eq!(std::fs::read(&dest).unwrap(), b"name,password");
        std::fs::remove_dir_all(&dir).ok();
    }

    // Whether the destination was ever opened, for the dialogs that must not
    // get that far.
    fn never_opened() -> (
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        impl FnOnce(&PathBuf) -> std::io::Result<Vec<u8>> + Send + 'static,
    ) {
        use std::sync::atomic::{AtomicBool, Ordering};

        let opened = std::sync::Arc::new(AtomicBool::new(false));
        let seen = opened.clone();
        (opened, move |_: &PathBuf| {
            seen.store(true, Ordering::SeqCst);
            Ok(Vec::new())
        })
    }

    // A dismissed dialog chose nowhere, so there is nothing to open: it is the
    // `None` every caller reads as "saved nothing".
    #[tokio::test]
    async fn a_dismissed_dialog_opens_nothing() {
        let (opened, open) = never_opened();

        let chosen = ask_then_write(b"name,password".to_vec(), async { Ok(None) }, open)
            .await
            .unwrap();

        assert!(chosen.is_none());
        assert!(!opened.load(std::sync::atomic::Ordering::SeqCst));
    }

    // Nor a dialog that failed outright.
    #[tokio::test]
    async fn a_failed_dialog_opens_nothing() {
        let (opened, open) = never_opened();

        let result = ask_then_write(
            b"name,password".to_vec(),
            async { Err(Error::Other("picker crashed".into())) },
            open,
        )
        .await;

        assert!(result.is_err());
        assert!(!opened.load(std::sync::atomic::Ordering::SeqCst));
    }

    // The picker answered, so the user believes the export is saved: a
    // document the provider then will not open has to come back as an error,
    // not as the destination.
    #[tokio::test]
    async fn a_destination_that_will_not_open_is_an_error() {
        let result = ask_then_write(
            b"name,password".to_vec(),
            async { Ok(Some(PathBuf::from("rowel-export.csv"))) },
            |_: &PathBuf| -> std::io::Result<Vec<u8>> {
                Err(std::io::Error::other("the provider refused"))
            },
        )
        .await;

        assert!(result.is_err());
    }
}
