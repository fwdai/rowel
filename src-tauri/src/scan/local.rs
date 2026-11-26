//! The on-device engine: the OS's recognizers for what is in the picture, and
//! the readers in this module for what it means.

use std::collections::BTreeMap;
use std::path::Path;

use zeroize::Zeroize;

use super::{card, document, mrz, Image, ScanResult, Scanner};
use crate::error::{Error, Result};

/// Text recognition over one image, in reading order (top to bottom) — the
/// order the MRZ parser needs to see its lines in.
pub trait Ocr {
    /// The image at a local path.
    fn recognize(&self, image_path: &Path) -> Result<Vec<String>>;

    /// The image as its encoded bytes — a photo the phone's picker handed the
    /// webview, which the backend has no path to. Only the phone reaches here,
    /// and the phone is Apple's, so the other backends keep the default:
    /// unsupported.
    fn recognize_data(&self, _image: &[u8]) -> Result<Vec<String>> {
        Err(Error::Unsupported(
            "scanning a captured photo is not available on this platform".into(),
        ))
    }
}

/// Apple's Vision framework (same API on macOS and iOS).
#[cfg(target_vendor = "apple")]
pub fn platform_ocr() -> Option<Box<dyn Ocr>> {
    Some(Box::new(super::ocr_apple::VisionOcr))
}

/// Windows' `Windows.Media.Ocr`, which needs a language pack to be installed.
#[cfg(target_os = "windows")]
pub fn platform_ocr() -> Option<Box<dyn Ocr>> {
    super::ocr_windows::WindowsOcr::new().map(|ocr| Box::new(ocr) as Box<dyn Ocr>)
}

/// Linux has no system text recognizer to call, and bundling an OCR engine is
/// not worth the binary — the feature is simply absent there.
#[cfg(not(any(target_vendor = "apple", target_os = "windows")))]
pub fn platform_ocr() -> Option<Box<dyn Ocr>> {
    None
}

/// The holder's photograph in `image`, as `Entry::portrait` keeps it, where
/// the platform can find a face: Apple's Vision.
#[cfg(target_vendor = "apple")]
fn platform_portrait(image: Image<'_>) -> Option<String> {
    super::portrait_apple::find(image)
}

/// No face detector is wired up anywhere else yet; the document's face keeps
/// its silhouette there.
#[cfg(not(target_vendor = "apple"))]
fn platform_portrait(_image: Image<'_>) -> Option<String> {
    None
}

/// The engine, where the platform has a text recognizer to give it.
pub(super) fn scanner() -> Option<Box<dyn Scanner>> {
    platform_ocr().map(|ocr| Box::new(Local { ocr }) as Box<dyn Scanner>)
}

struct Local {
    ocr: Box<dyn Ocr>,
}

impl Scanner for Local {
    fn scan(&self, image: Image<'_>) -> Result<Option<ScanResult>> {
        let mut lines = match image {
            Image::File(path) => self.ocr.recognize(path)?,
            Image::Bytes(bytes) => self.ocr.recognize_data(bytes)?,
        };
        let found = read(&lines, image);
        // The recognized lines are the card number in the clear; the parsed
        // result is all that may outlive this call.
        lines.zeroize();
        Ok(found)
    }
}

fn identity(fields: BTreeMap<String, String>) -> ScanResult {
    ScanResult {
        kind: "identity".into(),
        fields,
    }
}

/// Read whatever the recognized text holds.
///
/// The MRZ is tried first because a hit is unambiguous: its check digits have
/// to agree, so the lines cannot be anything else. What it gives is believed
/// over the print beside it. The print adds the two things an MRZ does not
/// say: which kind of card this is — its document code tells a passport from
/// a card but not one card from another — and when it was issued.
///
/// An MRZ whose digits did not verify is still an MRZ: a worse photo of one,
/// not a card. Its data line is a run of digits long enough to hold a
/// Luhn-valid window by chance, so nothing else is asked about it, and the
/// user is asked for a better photo instead.
///
/// A document without an MRZ is known by its title and read off its print
/// (`document`), and that is asked before the card parser for the same
/// reason the MRZ is: a card is recognized from a Luhn-valid digit run, and
/// a licence carries enough digits to hold one by chance. "Passport" alone
/// is the exception, asked last — it is the one title a payment card might
/// print.
pub(super) fn scan_lines(lines: &[String]) -> Option<ScanResult> {
    let printed = document::doc_type(lines);
    if let Some(mut fields) = mrz::parse(lines) {
        if let Some(named) = printed.filter(|named| *named != "passport") {
            if fields.get("doc_type").is_some_and(|code| code == "id_card") {
                fields.insert("doc_type".into(), named.into());
            }
        }
        if let Some(issued) = issued_between(lines, &fields) {
            fields.insert("issue_date".into(), issued);
        }
        return Some(identity(fields));
    }
    if mrz::present(lines) {
        return None;
    }
    let document = |doc_type: &str| {
        let mut fields = document::fields(lines, Some(doc_type));
        fields.insert("doc_type".into(), doc_type.to_owned());
        identity(fields)
    };
    if let Some(named) = printed.filter(|named| *named != "passport") {
        return Some(document(named));
    }
    card::parse(lines)
        .map(|fields| ScanResult {
            kind: "card".into(),
            fields,
        })
        .or_else(|| printed.map(document))
}

/// The issue date printed on a document whose MRZ was read, if it falls where
/// one has to: after the birth the MRZ gives and before its expiry. ISO dates
/// order as text.
fn issued_between(lines: &[String], mrz: &BTreeMap<String, String>) -> Option<String> {
    let doc_type = mrz.get("doc_type").map(String::as_str);
    let issued = document::fields(lines, doc_type).remove("issue_date")?;
    let known = |key: &str| mrz.get(key).filter(|date| !date.is_empty());
    let after_birth = known("birth_date").is_none_or(|birth| *birth < issued);
    let before_expiry = known("expiry_date").is_none_or(|expiry| issued < *expiry);
    (after_birth && before_expiry).then_some(issued)
}

/// Everything one picture gives: what its text says, and the holder's
/// photograph where it is an identity document.
///
/// Text that names nothing is not the end of it. A picture with print on it
/// and a face in it is an identity document whose title this did not know —
/// one in a script the recognizer was not reading, or with none — so it opens
/// as a document of no stated type, with its photograph and whatever its
/// print still gives. Not where a damaged MRZ is in view: that picture is
/// asked for again.
pub fn read(lines: &[String], image: Image<'_>) -> Option<ScanResult> {
    if let Some(mut found) = scan_lines(lines) {
        // A card has no holder to show; and a document that reads is a scan
        // with or without its picture.
        if found.kind == "identity" {
            if let Some(portrait) = platform_portrait(image) {
                found.fields.insert("portrait".into(), portrait);
            }
        }
        return Some(found);
    }
    if lines.is_empty() || mrz::present(lines) {
        return None;
    }
    let portrait = platform_portrait(image)?;
    let mut fields = document::fields(lines, None);
    fields.insert("doc_type".into(), "other".into());
    fields.insert("portrait".into(), portrait);
    Some(identity(fields))
}
