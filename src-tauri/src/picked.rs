//! Opening a file the user chose, wherever the choice points.
//!
//! What the file dialog (`commands::tools::pick_file`) or a drop on the window
//! hands the webview, and the webview hands back to the reader that spends its
//! grant (`grants`), is a path on disk everywhere but Android. There the system
//! picker answers with a `content://` URI: a document some provider serves,
//! which `std::fs` cannot open and whose last segment is an id rather than its
//! name (`tauri-plugin-dialog` 2.8.4, `DialogPlugin.kt`: `filePickerResult`
//! resolves with the intent's URIs as strings). Every reader of a chosen file
//! opens it through [`open`], so that difference lives here and nowhere else.

use std::fs::File;
use std::path::Path;

use tauri::AppHandle;

use crate::error::Result;
use crate::storage;

/// Whether this platform's picker answers with documents rather than paths.
pub(crate) const DOCUMENTS: bool = cfg!(target_os = "android");

/// A chosen file, open for reading, and the name it goes by: what a reader
/// tells its kind by, and what an attachment is stored under.
pub struct Chosen {
    pub file: File,
    pub name: String,
}

/// Whether `path` names a document rather than a place on disk: a `content://`
/// URI, where the picker hands those out (`documents`, which is [`DOCUMENTS`]
/// outside the tests). Anywhere else a string that merely starts so is a path
/// like any other, and is read as one.
pub(crate) fn is_document(path: &Path, documents: bool) -> bool {
    documents && path.to_str().is_some_and(|p| p.starts_with("content://"))
}

/// Open the chosen file at `path` for reading. The caller has spent, or holds,
/// the grant for it (`grants`); this decides nothing about whether it may.
///
/// A path on disk is opened by [`open_local`]. A document is opened through the
/// content resolver, which hands back a descriptor of the provider's choosing,
/// and named by what the provider calls it.
pub fn open(app: &AppHandle, path: &Path) -> Result<Chosen> {
    #[cfg(target_os = "android")]
    if is_document(path, DOCUMENTS) {
        return open_document(app, path);
    }
    #[cfg(not(target_os = "android"))]
    let _ = app;
    open_local(path)
}

/// A file on disk: one open, settled on that handle to be a regular file
/// (`storage::open_regular_file`), named by its last component.
pub fn open_local(path: &Path) -> Result<Chosen> {
    let file = storage::open_regular_file(path)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(Chosen { file, name })
}

/// A document, opened read-only through `tauri-plugin-fs` 2.7.1
/// (`src/android.rs`: `Fs::open` asks `FsPlugin.getFileDescriptor`, which
/// opens the URI with the content resolver) and named by the provider's display
/// name (`PathResolver::file_name`, which asks `PathPlugin.getFileNameFromUri`).
///
/// The descriptor is not held to being a regular file, as one on disk is: a
/// provider may serve a document through a pipe, and the URI names exactly the
/// document the user picked, so there is no FIFO or device behind a path to
/// guard against. What bounds the read is the reader's own cap, which every
/// reader applies to whatever it was handed (`storage::read_capped`).
#[cfg(target_os = "android")]
fn open_document(app: &AppHandle, path: &Path) -> Result<Chosen> {
    use tauri::Manager;
    use tauri_plugin_fs::{FilePath, FsExt, OpenOptions};

    let uri = path.to_string_lossy();
    let url = url::Url::parse(&uri).map_err(|e| crate::error::Error::Other(e.to_string()))?;
    let mut options = OpenOptions::new();
    options.read(true);
    let file = app.fs().open(FilePath::Url(url), options)?;
    let name = app.path().file_name(&uri).unwrap_or_default();
    Ok(Chosen { file, name })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_content_uri_is_a_document_only_where_the_picker_hands_them_out() {
        let uri = Path::new("content://com.android.providers.downloads.documents/document/42");
        assert!(is_document(uri, true));
        assert!(!is_document(uri, false));
    }

    #[test]
    fn a_path_is_never_a_document() {
        for path in [
            "/storage/emulated/0/Download/export.csv",
            "content",
            "file:///var/mobile/Documents/vault.rowel",
            "C:\\Users\\me\\export.csv",
        ] {
            assert!(!is_document(Path::new(path), true), "{path}");
        }
    }

    #[test]
    fn a_file_on_disk_opens_under_its_own_name() {
        use std::io::Read;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("export.csv");
        std::fs::write(&path, b"name,password\n").unwrap();

        let mut chosen = open_local(&path).unwrap();
        let mut text = String::new();
        chosen.file.read_to_string(&mut text).unwrap();
        assert_eq!(chosen.name, "export.csv");
        assert_eq!(text, "name,password\n");
    }

    #[test]
    fn a_directory_on_disk_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert!(open_local(dir.path()).is_err());
    }
}
