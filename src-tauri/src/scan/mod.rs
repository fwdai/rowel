//! Reading a photo of a payment card or an identity document into the fields
//! of an entry, so the user does not retype what is printed on it.
//!
//! # The contract
//!
//! This file is all the rest of the app knows about scanning: [`scan`] and
//! [`scan_bytes`] take a picture and answer with a [`ScanResult`] — which
//! kind of entry it is and that entry's fields, the holder's photograph among
//! them — and [`is_supported`] says whether this build can scan at all.
//!
//! [`Scanner`] is the engine behind them and the one seam. Everything below
//! it can be replaced — a different recognizer, a model that reads the whole
//! picture — by another `Scanner`, and nothing outside this module moves.
//!
//! # The engine there is
//!
//! `local` is fully on-device: the image is only ever read from where the
//! user already keeps it, and nothing touches the network or writes a file.
//! It works in two halves.
//!
//! The platform half turns a picture into lines of text (`ocr_*`, `rows`)
//! and finds the holder's face to cut the photograph around (`portrait*`),
//! both with what the OS ships.
//!
//! The reading half is pure text-in/fields-out and carries the tests:
//!
//! - `mrz` — the machine-readable zone of a travel document, the one reading
//!   with check digits and so the one believed first;
//! - `document` — the printed face of any identity document: its type by its
//!   title, its fields by their captions, its dates by their order;
//! - `card` — a payment card, by its number.
//!
//! `words` is what they share: recognized text as the words print is matched
//! and judged by.
//!
//! Recognized text holds the very numbers the vault exists to protect, so it
//! is zeroized the moment it has been read.

mod card;
mod document;
mod local;
mod mrz;
#[cfg(target_vendor = "apple")]
mod ocr_apple;
#[cfg(target_os = "windows")]
mod ocr_windows;
#[cfg(any(target_vendor = "apple", test))]
mod portrait;
#[cfg(target_vendor = "apple")]
mod portrait_apple;
#[cfg(any(target_vendor = "apple", test))]
mod rows;
#[cfg(test)]
mod tests;
mod words;

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::error::{Error, Result};

// The local engine's parts, for the example that checks them against a real
// picture (`examples/scan.rs`) and for the backends beside this file.
pub use local::{platform_ocr, read, Ocr};

/// What a scan found. `kind` is `"card"` or `"identity"`; `fields` is keyed by
/// the field names of the matching entry kind, so the form can fill itself in
/// without knowing how it was read. An identity's may include `portrait`,
/// which is a field of the entry like the rest (`Entry::portrait`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub kind: String,
    pub fields: BTreeMap<String, String>,
}

/// The picture a scan is handed: a file the user already keeps, or the bytes
/// of a photo that is on no disk.
#[derive(Clone, Copy)]
pub enum Image<'a> {
    File(&'a Path),
    Bytes(&'a [u8]),
}

/// Something that reads a picture of a card or a document.
///
/// The whole of what an engine owes the app: one picture in, and out of it
/// the entry it describes — or `None` for a picture it makes nothing of. How
/// it gets there is its own business.
pub trait Scanner {
    fn scan(&self, image: Image<'_>) -> Result<Option<ScanResult>>;
}

/// The engine this build scans with, where it has one.
pub fn scanner() -> Option<Box<dyn Scanner>> {
    local::scanner()
}

/// Whether this platform can scan at all, so the UI can leave the affordance
/// out rather than offer a drop target that always fails.
pub fn is_supported() -> bool {
    scanner().is_some()
}

/// Scan the image at `path` and return the fields read out of it.
///
/// `path` is what the frontend already has in hand — the drag-drop event's file
/// path, or the one the dialog plugin returned. A file on disk is never taken
/// as bytes: it is the user's own, so passing bytes would only add a copy of
/// a card photo to memory without removing a read of the file. (`scan_bytes`
/// is for the photo that is *not* on disk.)
pub async fn scan(path: String) -> Result<ScanResult> {
    // The path is the webview's word for a file on the user's disk, so it is
    // checked before anything opens it: only the image types the pickers offer
    // are scannable, and a scan of anything else is a read this has no business
    // doing. Refused as "nothing recognized" — which is what it is.
    let file = local_path(&path);
    if !is_image(&file) {
        return Err(Error::Unrecognized);
    }
    off_thread(move || engine()?.scan(Image::File(&file))).await
}

/// Scan a photo the webview holds as bytes — the one the phone's system picker
/// handed its file input, which the backend has no path to. The same engine
/// as `scan`. The bytes are a card in the clear, so this buffer is zeroized
/// once read — best effort, since it is one copy of several: the IPC body the
/// command cloned it from, whatever the engine decodes, and the webview's own
/// buffers are dropped, not scrubbed.
pub async fn scan_bytes(mut image: Vec<u8>) -> Result<ScanResult> {
    off_thread(move || {
        let found = engine().and_then(|engine| engine.scan(Image::Bytes(&image)));
        image.zeroize();
        found
    })
    .await
}

fn engine() -> Result<Box<dyn Scanner>> {
    scanner().ok_or_else(|| Error::Unsupported("scanning is not available on this platform".into()))
}

// Reading a picture is a heavy CPU job, so it runs off the UI thread — and
// the engine is made there too, since nothing says one may cross threads.
async fn off_thread(
    read: impl FnOnce() -> Result<Option<ScanResult>> + Send + 'static,
) -> Result<ScanResult> {
    tauri::async_runtime::spawn_blocking(read)
        .await
        .map_err(|e| Error::Other(e.to_string()))??
        .ok_or(Error::Unrecognized)
}

/// What the frontend calls a path may be a `file://` URL.
///
/// The iOS dialog plugin copies a picked photo into the app sandbox and hands
/// back that copy's URL (`FilePath::Url`, which serializes as the URL string),
/// where every other source of a path — a drop, the desktop dialog — gives a
/// plain one. A Windows path parses as a URL too (`C:` reads as a scheme), so
/// only `file:` is unwrapped.
fn local_path(path: &str) -> std::path::PathBuf {
    url::Url::parse(path)
        .ok()
        .filter(|u| u.scheme() == "file")
        .and_then(|u| u.to_file_path().ok())
        .unwrap_or_else(|| std::path::PathBuf::from(path))
}

/// The largest photo `scan_bytes` is handed — far past any phone camera's
/// JPEG, there only so a body cannot be arbitrarily large.
pub(crate) const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

/// The image types both OCR backends load and both entry points offer — the
/// file dialog's filter (`commands::tools::pick_file`) and the window drop's
/// own test. Kept in step with `IMAGE_EXTENSIONS` in `src/lib/fileTypes.ts`.
pub(crate) const IMAGE_EXTENSIONS: [&str; 10] = [
    "png", "jpg", "jpeg", "heic", "heif", "webp", "tiff", "tif", "bmp", "gif",
];

/// Whether a path names one of those, by extension alone: the file is not
/// opened to find out, which is the point.
pub(crate) fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}
