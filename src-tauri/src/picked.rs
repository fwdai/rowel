//! Opening a file the user chose, wherever the choice points.
//!
//! What the file dialog (`commands::tools::pick_file`) or a drop on the window
//! hands the webview, and the webview hands back to the reader that spends its
//! grant (`grants`), is a path on disk everywhere but Android. There the system
//! picker answers with a `content://` URI: a document some provider serves,
//! which `std::fs` cannot open and whose last segment is an id rather than its
//! name (`tauri-plugin-dialog` 2.8.4, `DialogPlugin.kt`: `filePickerResult`
//! resolves with the intent's URIs as strings). Every reader of a chosen file
//! opens it through [`open`], and the name a pick goes by comes from [`name`],
//! so that difference lives here and nowhere else.
//!
//! The address is what a grant is held under and what a reader opens; the name
//! is what anything about the file is told from — its kind by its extension, a
//! label shown to the user. On a path the name is its last component, so the
//! two only part on Android, which is why a pick hands the webview both
//! ([`Pick`]) rather than leaving it to read the name off the address.
//!
//! What a reader is handed is [`Contents`]: bytes read front to back, and
//! nothing else. A file on disk is a regular file, settled on its handle when
//! it is opened; a document is whatever descriptor its provider chose, and a
//! provider may serve one through a pipe, which cannot seek and has no length
//! or metadata worth asking. So no reader is offered any of that: each reads
//! what it needs under its own cap, counted on the bytes that arrive, and the
//! one that needs to move about in a file (an archive) asks
//! [`Contents::seekable`], which takes a file on disk as it is and reads
//! anything else into memory first, under the cap it is given.

use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;

use serde::Serialize;
use tauri::AppHandle;
use zeroize::Zeroizing;

use crate::error::Result;
use crate::storage;

/// Whether this platform's picker answers with documents rather than paths.
pub(crate) const DOCUMENTS: bool = cfg!(target_os = "android");

/// A chosen file, open for reading, and the name it goes by: what a reader
/// tells its kind by, and what an attachment is stored under.
pub struct Chosen {
    pub contents: Contents,
    pub name: String,
}

/// A chosen file's bytes, read front to back. The handle is held rather than
/// exposed so that nothing a pipe cannot do — seek, ask a length, clone —
/// compiles against it (see the module docs).
pub struct Contents(File);

impl Read for Contents {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Contents {
    /// The bytes as something read in any order, for a reader that needs that
    /// and asks before it has read any: a file on disk as it is, since it can
    /// seek, and anything else read whole into memory first — refused past
    /// `cap`, counted on the bytes read, and scrubbed when it is dropped.
    pub fn seekable(mut self, cap: u64) -> Result<Seekable> {
        if self.0.metadata()?.file_type().is_file() {
            return Ok(Seekable::File(self.0));
        }
        let bytes = Zeroizing::new(storage::read_capped(&mut self, cap)?);
        Ok(Seekable::Memory(Cursor::new(bytes)))
    }
}

/// What [`Contents::seekable`] gives back.
pub enum Seekable {
    File(File),
    Memory(Cursor<Zeroizing<Vec<u8>>>),
}

impl Read for Seekable {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Seekable::File(file) => file.read(buf),
            Seekable::Memory(bytes) => bytes.read(buf),
        }
    }
}

impl Seek for Seekable {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match self {
            Seekable::File(file) => file.seek(pos),
            Seekable::Memory(bytes) => bytes.seek(pos),
        }
    }
}

/// A pick as the webview is handed it: the address to read it by and the name
/// it goes by.
#[derive(Serialize, Debug, PartialEq)]
pub struct Pick {
    pub path: String,
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
        let file = open_document(app, path)?;
        return Ok(Chosen {
            contents: Contents(file),
            name: name(app, path),
        });
    }
    #[cfg(not(target_os = "android"))]
    let _ = app;
    open_local(path)
}

/// The name the chosen file at `path` goes by: its last component, or a
/// document's display name as its provider gives it (`PathResolver::file_name`,
/// which asks `PathPlugin.getFileNameFromUri`), empty if the provider will not
/// say. A document's name is a round trip to its provider, so this belongs off
/// the async runtime, as an open does.
pub fn name(app: &AppHandle, path: &Path) -> String {
    #[cfg(target_os = "android")]
    if is_document(path, DOCUMENTS) {
        use tauri::Manager;
        return app
            .path()
            .file_name(&path.to_string_lossy())
            .unwrap_or_default();
    }
    #[cfg(not(target_os = "android"))]
    let _ = app;
    local_name(path)
}

/// A file on disk: one open, settled on that handle to be a regular file
/// (`storage::open_regular_file`), named by its last component.
pub fn open_local(path: &Path) -> Result<Chosen> {
    let file = storage::open_regular_file(path)?;
    Ok(Chosen {
        contents: Contents(file),
        name: local_name(path),
    })
}

fn local_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A document, opened read-only through `tauri-plugin-fs` 2.7.1
/// (`src/android.rs`: `Fs::open` asks `FsPlugin.getFileDescriptor`, which
/// opens the URI with the content resolver).
///
/// The descriptor is not held to being a regular file, as one on disk is: a
/// provider may serve a document through a pipe, and the URI names exactly the
/// document the user picked, so there is no FIFO or device behind a path to
/// guard against. What bounds the read is the reader's own cap, which every
/// reader applies to the bytes it reads (`storage::read_capped`), and what
/// keeps a reader from seeking it is [`Contents`].
#[cfg(target_os = "android")]
fn open_document(app: &AppHandle, path: &Path) -> Result<File> {
    use tauri_plugin_fs::{FilePath, FsExt, OpenOptions};

    let uri = path.to_string_lossy();
    let url = url::Url::parse(&uri).map_err(|e| crate::error::Error::Other(e.to_string()))?;
    let mut options = OpenOptions::new();
    options.read(true);
    Ok(app.fs().open(FilePath::Url(url), options)?)
}

/// A chosen file whose contents arrive through a pipe, as a provider may
/// serve a document: a real descriptor that cannot seek, written from another
/// thread so that a file past the pipe's buffer streams as it would on a
/// device. A reader that stops at its cap closes its end early, and the write
/// then fails, which is the writer's business only.
#[cfg(all(test, unix))]
pub(crate) fn piped(name: &str, bytes: Vec<u8>) -> Chosen {
    use std::io::Write;

    let (reader, mut writer) = std::io::pipe().unwrap();
    std::thread::spawn(move || {
        let _ = writer.write_all(&bytes);
    });
    Chosen {
        contents: Contents(File::from(std::os::fd::OwnedFd::from(reader))),
        name: name.into(),
    }
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
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("export.csv");
        std::fs::write(&path, b"name,password\n").unwrap();

        let mut chosen = open_local(&path).unwrap();
        let mut text = String::new();
        chosen.contents.read_to_string(&mut text).unwrap();
        assert_eq!(chosen.name, "export.csv");
        assert_eq!(text, "name,password\n");
    }

    // What `PickedFile` in `src/api/tools.ts` reads.
    #[test]
    fn a_pick_reaches_the_webview_as_its_path_and_name() {
        let pick = Pick {
            path: "content://com.android.providers.downloads.documents/document/42".into(),
            name: "Rowel backup.rowel".into(),
        };
        assert_eq!(
            serde_json::to_value(&pick).unwrap(),
            serde_json::json!({
                "path": "content://com.android.providers.downloads.documents/document/42",
                "name": "Rowel backup.rowel",
            })
        );
    }

    #[test]
    fn a_directory_on_disk_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert!(open_local(dir.path()).is_err());
    }

    fn read_all(mut source: impl Read) -> Vec<u8> {
        let mut bytes = Vec::new();
        source.read_to_end(&mut bytes).unwrap();
        bytes
    }

    // The helper the tests feed readers with is what it claims: a descriptor
    // that will not seek.
    #[cfg(unix)]
    #[test]
    fn a_piped_document_cannot_seek() {
        let Contents(mut file) = piped("export.csv", b"name,password\n".to_vec()).contents;
        assert!(file.seek(SeekFrom::Start(0)).is_err());
    }

    // A file on disk is moved about where it lies, not copied.
    #[test]
    fn a_file_on_disk_is_seekable_where_it_lies() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("export.1pux");
        std::fs::write(&path, b"PK\x03\x04 the archive").unwrap();

        let mut source = open_local(&path).unwrap().contents.seekable(4).unwrap();
        assert!(matches!(source, Seekable::File(_)), "read into memory");
        source.seek(SeekFrom::Start(4)).unwrap();
        assert_eq!(read_all(&mut source), b" the archive");
    }

    // A pipe is read into memory, whole, and can then be moved about.
    #[cfg(unix)]
    #[test]
    fn a_piped_document_is_seekable_once_read() {
        let mut source = piped("export.1pux", b"PK\x03\x04 the archive".to_vec())
            .contents
            .seekable(64)
            .unwrap();
        assert!(matches!(source, Seekable::Memory(_)));
        assert_eq!(read_all(&mut source), b"PK\x03\x04 the archive");
        source.seek(SeekFrom::Start(4)).unwrap();
        assert_eq!(read_all(&mut source), b" the archive");
    }

    // The cap is on the bytes that arrive: a pipe reports no length to trust.
    #[cfg(unix)]
    #[test]
    fn a_piped_document_past_the_cap_is_refused() {
        let result = piped("export.1pux", vec![0; 65]).contents.seekable(64);
        assert!(matches!(result, Err(crate::error::Error::FileTooLarge)));
    }
}
