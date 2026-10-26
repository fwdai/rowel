//! The `.rowel` pack — the single-file sync exchange artifact.
//!
//! Byte layout:
//!
//! ```text
//! "SWSY"  | format | kdf_len  | kdf params JSON | snapshot
//! 4 bytes | u8     | u32 (LE) | kdf_len bytes   | rest of file
//! ```
//!
//! The header is plaintext **by design**. KDF parameters are not secret (the
//! `.kdbx` header carries the same thing in the clear), and they are precisely
//! what a fresh install must read *before* it can derive a key — a chicken/egg
//! the header exists to break. Everything after it is the SQLCipher snapshot,
//! ciphertext end to end, so nothing else leaks: no titles, no hosts, not even
//! the entry count.
//!
//! There is no compression layer. The body is ciphertext, which does not
//! compress, so a codec would only add a failure mode and a decode cost.
//!
//! This module is deliberately pure — paths and bytes, no `AppHandle`, no Tauri
//! types — so the Drive provider can reuse it and the tests can drive it
//! directly.

use std::fs;
use std::path::Path;

use super::Scratch;
use crate::crypto::KdfParams;
use crate::store::SqliteStore;

/// The most a pack may be when pulled from Drive. A vault is metadata plus
/// sealed payloads whose biggest members are 1 MiB env files, so a real pack is
/// megabytes; the cap is there so a file that is not a real pack cannot be
/// buffered whole before `unpack` gets to refuse it.
pub const MAX_PACK_BYTES: usize = 256 * 1024 * 1024;

// The format tag, not the product name: every pack ever written carries it, so
// changing it would make this build refuse files it wrote itself.
const MAGIC: &[u8; 4] = b"SWSY";
const FORMAT_V1: u8 = 1;
const HEADER_LEN: usize = MAGIC.len() + 1 + 4;

// A KDF descriptor is a ~150-byte JSON object. The cap turns a corrupt or
// hostile length field into an error instead of a multi-gigabyte allocation.
const MAX_KDF_LEN: usize = 64 * 1024;

// The snapshot has no length field — it is "the rest of the file" — so a
// half-finished upload still parses structurally. This is the one cheap check
// that catches it: a SQLite database is always a whole number of pages, and 512
// is the smallest page size SQLite supports, so any real database body is a
// multiple of it. Catching truncation *here* matters because at open time every
// failure looks like a wrong key, and telling a user their master password is
// wrong when the download was short is the misdiagnosis this app cannot afford.
const SQLITE_MIN_PAGE: usize = 512;

/// Why a `.rowel` pack could not be read. Messages are user-safe: they can be
/// surfaced verbatim without naming offsets, paths, or key material.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("not a {} sync file", crate::app::APP_NAME)]
    BadMagic,

    // Kept apart from the corruption cases so a future format v2 reads as
    // "update the app", never as "your remote vault is damaged".
    #[error("sync file was written by a newer version of the app")]
    UnknownVersion(u8),

    #[error("sync file is truncated")]
    Truncated,

    #[error("sync file header is implausibly large")]
    KdfTooLarge(usize),

    #[error("sync file has an unreadable key descriptor")]
    BadKdf,

    #[error("sync file contains no vault data")]
    EmptySnapshot,

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Store(String),
}

pub type Result<T> = std::result::Result<T, PackError>;

impl From<PackError> for crate::error::Error {
    fn from(e: PackError) -> Self {
        crate::error::Error::Other(e.to_string())
    }
}

/// A parsed `.rowel` pack: its plaintext KDF descriptor and the encrypted
/// database snapshot that follows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unpacked {
    pub kdf_params_json: String,
    pub snapshot: Vec<u8>,
}

/// Wrap a SQLCipher snapshot in the pack header: magic, the format byte, the
/// length of the KDF descriptor, the descriptor, and then the snapshot.
pub fn pack(kdf_params_json: &str, snapshot: &[u8]) -> Vec<u8> {
    let kdf = kdf_params_json.as_bytes();
    let mut out = Vec::with_capacity(HEADER_LEN + kdf.len() + snapshot.len());
    out.extend_from_slice(MAGIC);
    out.push(FORMAT_V1);
    out.extend_from_slice(&(kdf.len() as u32).to_le_bytes());
    out.extend_from_slice(kdf);
    out.extend_from_slice(snapshot);
    out
}

/// Parse a `.rowel` pack.
///
/// Every field is validated before it is trusted, because the input is a file
/// pulled from a remote drive: it can be truncated by a failed upload, replaced
/// by an unrelated file, or written by a build that does not exist yet. Any of
/// those must produce an error, never a panic and never a partial read.
pub fn unpack(bytes: &[u8]) -> Result<Unpacked> {
    if bytes.get(..MAGIC.len()).is_some_and(|m| m != MAGIC) {
        return Err(PackError::BadMagic);
    }
    let header = bytes.get(..HEADER_LEN).ok_or(PackError::Truncated)?;

    let format = header[MAGIC.len()];
    if format != FORMAT_V1 {
        return Err(PackError::UnknownVersion(format));
    }

    let kdf_len = u32::from_le_bytes(header[MAGIC.len() + 1..HEADER_LEN].try_into().unwrap());
    let kdf_len = kdf_len as usize;
    if kdf_len > MAX_KDF_LEN {
        return Err(PackError::KdfTooLarge(kdf_len));
    }
    // Capped above, so this addition cannot overflow.
    let kdf_end = HEADER_LEN + kdf_len;

    let kdf = bytes.get(HEADER_LEN..kdf_end).ok_or(PackError::Truncated)?;
    let kdf_params_json = std::str::from_utf8(kdf)
        .map_err(|_| PackError::BadKdf)?
        .to_owned();
    // Parsed, not just stored: a descriptor this build cannot derive from is
    // worthless later, and failing here keeps the bad file out of the restore
    // path entirely.
    KdfParams::from_json(&kdf_params_json).map_err(|_| PackError::BadKdf)?;

    let snapshot = &bytes[kdf_end..];
    if snapshot.is_empty() {
        return Err(PackError::EmptySnapshot);
    }
    if !snapshot.len().is_multiple_of(SQLITE_MIN_PAGE) {
        return Err(PackError::Truncated);
    }

    Ok(Unpacked {
        kdf_params_json,
        snapshot: snapshot.to_vec(),
    })
}

/// Pack a live store: take a consistent snapshot through the online-backup API
/// into `scratch_dir`, then wrap it.
///
/// The snapshot has to land on disk first — SQLite's backup API writes to a
/// file, not a buffer — but it is SQLCipher ciphertext from the first byte, so
/// an interrupted cleanup leaks nothing beyond a stray encrypted blob. It is
/// still always removed, on the error paths too, via the [`Scratch`] guard.
pub fn pack_store(
    store: &SqliteStore,
    key: &[u8],
    kdf_params_json: &str,
    scratch_dir: &Path,
) -> Result<Vec<u8>> {
    let scratch = Scratch::new(scratch_dir, "pack")?;

    store
        .snapshot_to(scratch.path(), key)
        .map_err(|e| PackError::Store(e.to_string()))?;

    let snapshot = fs::read(scratch.path())?;
    if snapshot.is_empty() {
        return Err(PackError::EmptySnapshot);
    }
    Ok(pack(kdf_params_json, &snapshot))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::VaultStore;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    const KEY: &[u8] = &[0x33; 32];

    fn kdf_json() -> String {
        KdfParams::argon2id(b"salt-0123456789012345", 256, 1, 1)
            .to_json()
            .unwrap()
    }

    fn tmp_dir() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rowel-pack-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn record(id: &str, payload: &[u8]) -> crate::store::Record {
        crate::store::Record {
            id: id.into(),
            kind: "login".into(),
            title: "Example".into(),
            tags: "[]".into(),
            url_host: "example.com".into(),
            created_at: 100,
            updated_at: 200,
            deleted_at: None,
            payload: payload.to_vec(),
            card_brand: None,
            favorite: false,
            has_passkey: false,
            file_name: None,
            var_count: None,
            username: None,
            doc_type: None,
            doc_country: None,
        }
    }

    fn seeded_store(dir: &Path) -> SqliteStore {
        let store = SqliteStore::open(&dir.join("vault.db"), KEY).unwrap();
        store
            .import(&[record("1", b"one"), record("2", b"two")])
            .unwrap();
        store
    }

    // A stand-in database body: page-aligned, as any real snapshot is.
    fn body() -> Vec<u8> {
        (0..2 * SQLITE_MIN_PAGE).map(|i| (i * 31) as u8).collect()
    }

    #[test]
    fn header_round_trips() {
        let json = kdf_json();
        let snapshot = body();

        let got = unpack(&pack(&json, &snapshot)).unwrap();
        assert_eq!(got.kdf_params_json, json);
        assert_eq!(got.snapshot, snapshot);
    }

    #[test]
    fn pack_store_round_trips_through_a_reopened_snapshot() {
        let dir = tmp_dir();
        let source = seeded_store(&dir);
        let scratch = dir.join("scratch");

        let bytes = pack_store(&source, KEY, &kdf_json(), &scratch).unwrap();
        let unpacked = unpack(&bytes).unwrap();
        assert_eq!(unpacked.kdf_params_json, kdf_json());

        let restored_path = tmp_dir().join("restored.db");
        fs::write(&restored_path, &unpacked.snapshot).unwrap();
        let restored = SqliteStore::open(&restored_path, KEY).unwrap();

        assert_eq!(
            restored.state_digest().unwrap(),
            source.state_digest().unwrap()
        );
        assert_eq!(restored.get("1").unwrap().unwrap().payload, b"one");

        // The scratch snapshot is cleaned up, siblings included.
        assert!(fs::read_dir(&scratch).unwrap().next().is_none());
    }

    #[test]
    fn pack_store_errors_cleanly_when_the_scratch_dir_cannot_be_created() {
        let dir = tmp_dir();
        let store = seeded_store(&dir);
        // A file where the scratch directory should be.
        let scratch = dir.join("occupied");
        fs::write(&scratch, b"not a directory").unwrap();

        assert!(matches!(
            pack_store(&store, KEY, &kdf_json(), &scratch),
            Err(PackError::Io(_))
        ));
    }

    // Every prefix of a valid file, byte by byte. Anything cut inside the header
    // or the descriptor is structurally impossible and must be refused; a cut
    // inside the body is caught by the page-alignment check unless it happens to
    // land exactly on a page boundary, which is the one case the format cannot
    // see (there is no body length field) and the SQLCipher open rejects instead.
    #[test]
    fn truncation_at_every_length_errors_without_panicking() {
        let full = pack(&kdf_json(), &body());
        let body_start = full.len() - body().len();

        for len in 0..full.len() {
            let result = unpack(&full[..len]);
            let page_aligned_body =
                len > body_start && (len - body_start).is_multiple_of(SQLITE_MIN_PAGE);
            assert_eq!(
                result.is_err(),
                !page_aligned_body,
                "a {len}-byte prefix was classified wrongly"
            );
        }
        assert!(unpack(&full).is_ok());
    }

    #[test]
    fn a_body_that_is_not_a_whole_number_of_pages_is_refused() {
        let mut short = pack(&kdf_json(), &body());
        short.truncate(short.len() - 1);
        assert!(matches!(unpack(&short), Err(PackError::Truncated)));
    }

    #[test]
    fn garbage_of_every_length_errors_without_panicking() {
        let garbage: Vec<u8> = (0u16..=1024).map(|b| b.wrapping_mul(37) as u8).collect();
        for len in 0..garbage.len() {
            assert!(unpack(&garbage[..len]).is_err());
        }
    }

    #[test]
    fn wrong_magic_is_not_reported_as_corruption() {
        let mut bytes = pack(&kdf_json(), b"snapshot");
        bytes[0] = b'X';
        assert!(matches!(unpack(&bytes), Err(PackError::BadMagic)));
    }

    #[test]
    fn unknown_format_version_is_its_own_error() {
        let mut bytes = pack(&kdf_json(), b"snapshot");
        bytes[MAGIC.len()] = 2;
        assert!(matches!(unpack(&bytes), Err(PackError::UnknownVersion(2))));
    }

    #[test]
    fn kdf_length_beyond_the_cap_is_refused_before_allocating() {
        let mut bytes = pack(&kdf_json(), b"snapshot");
        bytes[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(unpack(&bytes), Err(PackError::KdfTooLarge(_))));
    }

    #[test]
    fn kdf_length_past_the_buffer_reads_as_truncated() {
        let mut bytes = pack(&kdf_json(), b"snapshot");
        // Under the cap, but longer than what the file actually holds.
        bytes[5..9].copy_from_slice(&(MAX_KDF_LEN as u32 - 1).to_le_bytes());
        assert!(matches!(unpack(&bytes), Err(PackError::Truncated)));
    }

    #[test]
    fn a_header_with_no_snapshot_is_refused() {
        assert!(matches!(
            unpack(&pack(&kdf_json(), b"")),
            Err(PackError::EmptySnapshot)
        ));
    }

    #[test]
    fn an_unparseable_kdf_descriptor_is_refused() {
        assert!(matches!(
            unpack(&pack("{\"algo\":\"rot13\"}", b"snapshot")),
            Err(PackError::BadKdf)
        ));
        assert!(matches!(
            unpack(&pack("not json at all", b"snapshot")),
            Err(PackError::BadKdf)
        ));
    }
}
