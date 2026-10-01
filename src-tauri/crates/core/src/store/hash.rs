//! Content hashing for sync: a canonical fingerprint per record and per
//! attachment row, and a digest over everything the two tables hold.
//!
//! Both are consumed by the sync engine as *comparison* primitives, so the only
//! property that matters is that two stores agree bit-for-bit on the same
//! content — never that the bytes are secret. The encoding is therefore fixed
//! and injective, not merely "hash the fields": every variable-length field is
//! length-prefixed and every `Option` carries a presence tag, so no two
//! different records can share a preimage. Naive concatenation would let
//! ("ab", "c") and ("a", "bc") collide, which in the merge tie-break would mean
//! two genuinely different records comparing equal and one silently winning.

use sha2::{Digest, Sha256};

use super::{AttachmentMeta, Record};

/// `u64` LE length, then the bytes. The prefix is what makes the concatenation
/// of fields unambiguous.
fn field(h: &mut Sha256, bytes: &[u8]) {
    h.update((bytes.len() as u64).to_le_bytes());
    h.update(bytes);
}

/// Presence tag (0/1) then, when present, the 8-byte LE value. `None` and
/// `Some(0)` must not encode alike.
fn opt_i64(h: &mut Sha256, v: Option<i64>) {
    match v {
        None => h.update([0u8]),
        Some(n) => {
            h.update([1u8]);
            h.update(n.to_le_bytes());
        }
    }
}

fn opt_field(h: &mut Sha256, v: Option<&str>) {
    match v {
        None => h.update([0u8]),
        Some(s) => {
            h.update([1u8]);
            field(h, s.as_bytes());
        }
    }
}

/// SHA-256 over a record's content: everything that makes two rows the same
/// row, in a fixed field order.
///
/// `id` is deliberately excluded — it is the key the comparison is made *under*
/// (records are only ever hashed against another record of the same id), so
/// including it would add nothing. [`state_digest`] adds it back, because there
/// the identity of each row is part of what is being fingerprinted.
///
/// Timestamps are included: a record whose only change is `updated_at` is a
/// different record for merge purposes.
pub fn record_hash(r: &Record) -> [u8; 32] {
    let mut h = Sha256::new();
    field(&mut h, r.kind.as_bytes());
    field(&mut h, r.title.as_bytes());
    field(&mut h, r.tags.as_bytes());
    field(&mut h, r.url_host.as_bytes());
    h.update(r.created_at.to_le_bytes());
    h.update(r.updated_at.to_le_bytes());
    opt_i64(&mut h, r.deleted_at);
    field(&mut h, &r.payload);
    opt_field(&mut h, r.card_brand.as_deref());
    h.update([r.favorite as u8]);
    // Included for the same reason `card_brand` is: it is a column, so two
    // stores can hold the same payload under a different flag (one has run the
    // backfill, the other has not), and a merge blind to it would leave the
    // stale flag beside the winner's payload. Appended last so the field order
    // keeps tracking the column order.
    h.update([r.has_passkey as u8]);
    // Columns too, for the same reason again: a peer that has not run the env
    // backfill holds the same payload under NULLs, and the merge must see that.
    opt_field(&mut h, r.file_name.as_deref());
    opt_i64(&mut h, r.var_count);
    opt_field(&mut h, r.username.as_deref());
    h.finalize().into()
}

/// How many of a record's derived columns have been stamped. The merge prefers
/// the higher rank on a timestamp tie, so a row backfilled on one device is not
/// undone by a peer's copy of the same row that predates the column — the two
/// have equal `updated_at` (backfills do not stamp it), and the hash tie-break
/// alone can pick the NULL side, permanently.
///
/// Only the NULL-marker columns count. Every write stamps them for its kind, so
/// two genuine edits always tie here and fall through to the hash; the rank
/// only ever separates a stamped row from one that predates the column.
/// `has_passkey` is left out because `false` is also what a stamped login with
/// no passkey holds — counting it would bias real edit conflicts.
pub fn derived_rank(r: &Record) -> u8 {
    r.card_brand.is_some() as u8 + r.var_count.is_some() as u8 + r.username.is_some() as u8
}

/// SHA-256 over an entire entry set — tombstones included — as
/// (length-prefixed id, [`record_hash`]) pairs in ascending id byte order.
///
/// Equal digests mean identical entry state, which is what lets the sync engine
/// decide a push by digest inequality alone. Sorting happens here rather than
/// being inherited from a caller's `ORDER BY` so the ordering contract stays
/// with the definition of the digest.
pub fn state_digest(recs: &[Record]) -> [u8; 32] {
    let mut order: Vec<&Record> = recs.iter().collect();
    order.sort_unstable_by(|a, b| a.id.as_bytes().cmp(b.id.as_bytes()));

    let mut h = Sha256::new();
    for r in order {
        field(&mut h, r.id.as_bytes());
        h.update(record_hash(r));
    }
    h.finalize().into()
}

/// [`record_hash`]'s counterpart for an attachment row, under the same rules:
/// the id left out, every field in a fixed order, length-prefixed or tagged.
///
/// The sealed file stands in by its length alone. A row's blob is written once
/// and only ever changes by being emptied, which the length (and the tombstone
/// beside it) already says — so hashing megabytes of ciphertext, and reading
/// them out of the database under the session lock to do it, would buy nothing.
pub fn attachment_hash(m: &AttachmentMeta, blob_len: usize) -> [u8; 32] {
    let mut h = Sha256::new();
    field(&mut h, m.entry_id.as_bytes());
    field(&mut h, m.name.as_bytes());
    opt_field(&mut h, m.mime.as_deref());
    h.update(m.size.to_le_bytes());
    h.update(m.created_at.to_le_bytes());
    h.update(m.updated_at.to_le_bytes());
    opt_i64(&mut h, m.deleted_at);
    h.update((blob_len as u64).to_le_bytes());
    h.finalize().into()
}

/// The whole synced state: [`state_digest`] over the entries, extended by the
/// attachment rows — each as its metadata and blob length — when there are
/// any. A vault without attachments digests exactly as it did before they
/// existed, so nothing about it looks changed.
pub fn vault_digest<'a>(
    recs: &[Record],
    atts: impl IntoIterator<Item = (&'a AttachmentMeta, usize)>,
) -> [u8; 32] {
    let entries = state_digest(recs);
    let mut order: Vec<(&AttachmentMeta, usize)> = atts.into_iter().collect();
    if order.is_empty() {
        return entries;
    }
    order.sort_unstable_by(|a, b| a.0.id.as_bytes().cmp(b.0.id.as_bytes()));

    let mut h = Sha256::new();
    h.update(entries);
    for (meta, blob_len) in order {
        field(&mut h, meta.id.as_bytes());
        h.update(attachment_hash(meta, blob_len));
    }
    h.finalize().into()
}
