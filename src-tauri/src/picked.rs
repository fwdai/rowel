//! A file picked on Android, made a path on disk as it is everywhere else.
//!
//! Every reader of a chosen file, the grants (`grants`) and the webview's
//! checks by extension work on a path. The desktop dialog answers with one, and
//! the iOS dialog with a `file://` URL to a copy its picker has already made in
//! the app's own temp folder. Android's answers with a `content://` URI: a
//! document some provider serves, which `std::fs` cannot open and whose last
//! segment is an id rather than its name (`tauri-plugin-dialog` 2.8.4,
//! `DialogPlugin.kt`: `filePickerResult` resolves with the intent's URIs as
//! strings). So there the document is copied once, as iOS does, into
//! `picked/<unique id>/<its name>` under the app's cache directory, and the pick
//! goes on as that path ([`into_path`]). The directory is the internal one,
//! `Context.getCacheDir`, which no other app can read (tauri 2.12.5:
//! `PathResolver::app_cache_dir` asks `PathPlugin.getCacheDir`).
//!
//! A copy is whatever the user picked, an export in the clear or a photo of a
//! card, so the copies are not kept: the whole folder is removed at startup and
//! whenever a session ends ([`wipe`]).

use std::path::PathBuf;
#[cfg(any(target_os = "android", test))]
use std::{
    fs::{self, File},
    io::{self, Read},
    path::Path,
};

use tauri::AppHandle;
use tauri_plugin_dialog::FilePath;

use crate::error::{Error, Result};

/// The folder under the app's cache directory the copies are made in.
#[cfg(any(target_os = "android", test))]
const DIR: &str = "picked";

/// The dialog's answer as a path to read: on Android a document is copied in
/// first (see the module docs); anything else is the path the answer names,
/// a `file://` URL resolved to its path.
pub fn into_path(app: &AppHandle, answer: FilePath) -> Result<PathBuf> {
    match answer {
        #[cfg(target_os = "android")]
        FilePath::Url(uri) if uri.scheme() == "content" => copy_document(app, uri),
        answer => {
            #[cfg(not(target_os = "android"))]
            let _ = app;
            answer
                .into_path()
                .map_err(|_| Error::Unsupported("the picked file has no local path".into()))
        }
    }
}

/// Copy the document at `uri` into the app's cache, under the name its provider
/// gives it (`PathPlugin.getFileNameFromUri`; empty if the provider will not
/// say). Opened read-only through the content resolver (`tauri-plugin-fs`
/// 2.7.1, `src/android.rs`: `Fs::open` asks `FsPlugin.getFileDescriptor`).
/// Both are round trips to the provider, so this runs off the async runtime.
#[cfg(target_os = "android")]
fn copy_document(app: &AppHandle, uri: url::Url) -> Result<PathBuf> {
    use tauri::Manager;
    use tauri_plugin_fs::{FsExt, OpenOptions};

    let name = app.path().file_name(uri.as_str()).unwrap_or_default();
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|e| Error::Other(e.to_string()))?;
    let mut options = OpenOptions::new();
    options.read(true);
    let source = app.fs().open(FilePath::Url(uri), options)?;
    copy_in(&cache, &name, source, crate::storage::MAX_BACKUP_BYTES)
}

/// Copy `source` into a folder of its own under `<cache>/picked`, as `name`,
/// and hand back where it went. `name` comes from another app, so only what
/// [`file_name`] keeps of it is used. Refused past `cap`, counted on the bytes
/// read: a provider may serve a pipe, which has no length to trust. On any
/// failure the folder goes, with whatever had been written into it.
#[cfg(any(target_os = "android", test))]
fn copy_in(cache: &Path, name: &str, source: impl Read, cap: u64) -> Result<PathBuf> {
    let name = file_name(name)
        .ok_or_else(|| Error::Unsupported("the picked file has no name to keep it under".into()))?;
    let root = cache.join(DIR);
    fs::create_dir_all(&root)?;
    // Removed when dropped, so every way out before `keep` takes it along.
    let dir = tempfile::tempdir_in(&root)?;
    let mut file = File::create_new(dir.path().join(name))?;
    let copied = io::copy(&mut source.take(cap.saturating_add(1)), &mut file)?;
    if copied > cap {
        return Err(Error::FileTooLarge);
    }
    Ok(dir.keep().join(name))
}

/// What a copy may be named out of the `name` a provider gave: its last path
/// component, so the copy cannot land outside its folder, and nothing when
/// that is empty or only dots (`.`, `..`), which name no file of their own.
#[cfg(any(target_os = "android", test))]
fn file_name(name: &str) -> Option<&str> {
    let last = name.rsplit(['/', '\\']).next()?;
    (!last.trim_start_matches('.').is_empty()).then_some(last)
}

/// Remove every copy. Android only, the one platform that makes them. The
/// removal runs off the calling thread, so this may be called under the
/// session guard (`session::announce_ended`). Best effort: a folder that will
/// not go now goes at the next session end or startup.
pub fn wipe(app: &AppHandle) {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;

        let app = app.clone();
        crate::commands::detached(move || {
            if let Ok(cache) = app.path().app_cache_dir() {
                wipe_in(&cache);
            }
        });
    }
    #[cfg(not(target_os = "android"))]
    let _ = app;
}

/// Remove the copies made under `cache`, all of them at once.
#[cfg(any(target_os = "android", test))]
fn wipe_in(cache: &Path) {
    let _ = fs::remove_dir_all(cache.join(DIR));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_keeps_only_its_last_component() {
        assert_eq!(file_name("export.csv"), Some("export.csv"));
        assert_eq!(file_name(".env.production"), Some(".env.production"));
        assert_eq!(file_name("../../databases/vault.db"), Some("vault.db"));
        assert_eq!(file_name("/data/data/app/x.rowel"), Some("x.rowel"));
        assert_eq!(file_name("..\\..\\x.rowel"), Some("x.rowel"));
    }

    #[test]
    fn a_name_that_leaves_nothing_is_refused() {
        for name in ["", ".", "..", "...", "a/..", "a/", "/"] {
            assert_eq!(file_name(name), None, "{name:?}");
        }
    }

    #[test]
    fn a_document_is_copied_into_a_folder_of_its_own() {
        let cache = tempfile::tempdir().unwrap();

        let first = copy_in(cache.path(), "../export.csv", &b"name,password\n"[..], 64).unwrap();
        let second = copy_in(cache.path(), "export.csv", &b"other"[..], 64).unwrap();

        assert_eq!(first.file_name().unwrap(), "export.csv");
        assert_eq!(
            first.parent().unwrap().parent().unwrap(),
            cache.path().join(DIR)
        );
        assert_ne!(first, second);
        assert_eq!(fs::read(&first).unwrap(), b"name,password\n");
        assert_eq!(fs::read(&second).unwrap(), b"other");
    }

    #[test]
    fn a_document_without_a_name_is_not_copied() {
        let cache = tempfile::tempdir().unwrap();
        assert!(matches!(
            copy_in(cache.path(), "..", &b"x"[..], 64),
            Err(Error::Unsupported(_))
        ));
        assert!(!cache.path().join(DIR).exists());
    }

    // Counted on the bytes read, and nothing of a refused copy stays behind.
    #[test]
    fn a_document_past_the_cap_is_refused_and_removed() {
        let cache = tempfile::tempdir().unwrap();

        assert!(copy_in(cache.path(), "exact.rowel", &[0u8; 64][..], 64).is_ok());
        let refused = copy_in(cache.path(), "huge.rowel", &[0u8; 65][..], 64);

        assert!(matches!(refused, Err(Error::FileTooLarge)));
        let left: Vec<_> = fs::read_dir(cache.path().join(DIR)).unwrap().collect();
        assert_eq!(left.len(), 1, "only the copy within the cap is left");
    }

    #[test]
    fn a_wipe_takes_every_copy_and_nothing_else() {
        let cache = tempfile::tempdir().unwrap();
        let kept = cache.path().join("favicons.tmp");
        fs::write(&kept, b"x").unwrap();
        copy_in(cache.path(), "export.csv", &b"a"[..], 64).unwrap();
        copy_in(cache.path(), "card.jpg", &b"b"[..], 64).unwrap();

        wipe_in(cache.path());

        assert!(!cache.path().join(DIR).exists());
        assert!(kept.exists());
        // Nothing to wipe is nothing to do.
        wipe_in(cache.path());
    }
}
