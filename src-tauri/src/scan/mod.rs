//! Reading a dropped photo of a credit card or an identity document into the
//! fields of an entry, so the user does not retype an embossed number.
//!
//! Fully local: the OS text recognizer runs on-device, the image is only ever
//! read from where the user already keeps it, and nothing here touches the
//! network or writes a file. Recognized text holds the very numbers the vault
//! exists to protect, so it is zeroized the moment it has been parsed.
//!
//! The parsers (`mrz`, `card`) are pure text-in/fields-out and carry the tests;
//! the platform backends only turn an image into lines.

mod card;
mod mrz;
#[cfg(target_vendor = "apple")]
mod ocr_apple;
#[cfg(target_os = "windows")]
mod ocr_windows;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::error::{Error, Result};

/// What a scan found. `kind` is `"card"` or `"identity"`; `fields` is keyed by
/// the field names of the matching entry kind (see `mrz`/`card` for the exact
/// sets), so the form can fill itself in without knowing how it was read.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub kind: String,
    pub fields: BTreeMap<String, String>,
}

/// Text recognition over one image, in reading order (top to bottom) — the
/// order the MRZ parser needs to see its lines in.
pub trait Ocr {
    /// The image at a local path.
    fn recognize(&self, image_path: &Path) -> Result<Vec<String>>;

    /// The image as its encoded bytes — a photo the camera just took, which
    /// exists nowhere on disk. Only the phone's camera reaches here, and the
    /// phone is Apple's, so the other backends keep the default: unsupported.
    fn recognize_data(&self, _image: &[u8]) -> Result<Vec<String>> {
        Err(Error::Unsupported(
            "scanning a captured photo is not available on this platform".into(),
        ))
    }
}

/// Apple's Vision framework (same API on macOS and iOS).
#[cfg(target_vendor = "apple")]
pub fn platform_ocr() -> Option<Box<dyn Ocr>> {
    Some(Box::new(ocr_apple::VisionOcr))
}

/// Windows' `Windows.Media.Ocr`, which needs a language pack to be installed.
#[cfg(target_os = "windows")]
pub fn platform_ocr() -> Option<Box<dyn Ocr>> {
    ocr_windows::WindowsOcr::new().map(|ocr| Box::new(ocr) as Box<dyn Ocr>)
}

/// Linux has no system text recognizer to call, and bundling an OCR engine is
/// not worth the binary — the feature is simply absent there.
#[cfg(not(any(target_vendor = "apple", target_os = "windows")))]
pub fn platform_ocr() -> Option<Box<dyn Ocr>> {
    None
}

/// Read whatever the recognized text holds.
///
/// The MRZ is tried first because a hit is unambiguous: its check digits have
/// to agree, so the lines cannot be anything else. A card, by contrast, is
/// recognized from a Luhn-valid digit run — and an identity document can carry
/// one of those by chance.
pub fn scan_lines(lines: &[String]) -> Option<ScanResult> {
    if let Some(fields) = mrz::parse(lines) {
        return Some(ScanResult {
            kind: "identity".into(),
            fields,
        });
    }
    card::parse(lines).map(|fields| ScanResult {
        kind: "card".into(),
        fields,
    })
}

/// OCR the image at `path` and return the fields read out of it. Recognition is
/// a heavy CPU job, so it runs off the UI thread.
///
/// `path` is what the frontend already has in hand — the drag-drop event's file
/// path, or the one the dialog plugin returned. A file on disk is never taken
/// as bytes: both backends load an image by URL/path, and the file is the
/// user's own, so passing bytes would only add a copy of a card photo to
/// memory without removing a read of the file. (`scan_bytes` is for the photo
/// that is *not* on disk.)
pub async fn scan(path: String) -> Result<ScanResult> {
    // The path is the webview's word for a file on the user's disk, so it is
    // checked before anything opens it: only the image types the pickers offer
    // are scannable, and a scan of anything else is a read this has no business
    // doing. Refused as "nothing recognized" — which is what it is.
    let file = local_path(&path);
    if !is_image(&file) {
        return Err(Error::Unrecognized);
    }
    tauri::async_runtime::spawn_blocking(move || {
        let ocr = platform_ocr().ok_or_else(|| {
            Error::Unsupported("scanning is not available on this platform".into())
        })?;
        let mut lines = ocr.recognize(&file)?;
        let found = scan_lines(&lines);
        // The recognized lines are the card number in the clear; the parsed
        // result is all that may outlive this call.
        lines.zeroize();
        found.ok_or(Error::Unrecognized)
    })
    .await
    .map_err(|e| Error::Other(e.to_string()))?
}

/// OCR a photo the webview holds as bytes — the one the phone's camera just
/// took through the capture input, which was never written anywhere — and
/// return the fields read out of it. The same recognizer and parsers as `scan`,
/// off the UI thread; the bytes are a card in the clear and are zeroized with
/// the lines once parsed.
pub async fn scan_bytes(mut image: Vec<u8>) -> Result<ScanResult> {
    tauri::async_runtime::spawn_blocking(move || {
        let ocr = platform_ocr().ok_or_else(|| {
            Error::Unsupported("scanning is not available on this platform".into())
        })?;
        let recognized = ocr.recognize_data(&image);
        image.zeroize();
        let mut lines = recognized?;
        let found = scan_lines(&lines);
        lines.zeroize();
        found.ok_or(Error::Unrecognized)
    })
    .await
    .map_err(|e| Error::Other(e.to_string()))?
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

/// Whether this platform can scan at all, so the UI can leave the affordance
/// out rather than offer a drop target that always fails.
pub fn is_supported() -> bool {
    platform_ocr().is_some()
}
