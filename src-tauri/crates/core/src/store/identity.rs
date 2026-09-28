//! The vault id: a stable identity for a vault's *data*.
//!
//! Minted once when a vault is created and kept in the encrypted `meta` table,
//! so it travels inside every pack and backup: a `.rowel` restored on another
//! device is the same vault, and Drive sync finds its pack under the same name
//! (see `sync::layout`). It is not the workspace id — that is a local slot,
//! `default` on every install — and it is not under `SYNC_META_PREFIX`, so a
//! restore keeps it while scrubbing the source device's sync bookkeeping.
//!
//! Vaults created before ids existed have none; sync assigns one the first
//! time it needs it (or adopts the one the remote already carries).
//!
//! The vault's *name* lives here for the same reason: a label kept only in this
//! device's workspace registry is a label the user's other devices never see.
//! It carries a stamp because two devices can rename the same vault, and the
//! later rename has to win (see `sync::engine`).
//!
//! Like `migrate`, this reaches for the app's crypto to mint ids; it is a
//! boundary helper, not part of the pure [`VaultStore`] trait.

use super::{Result, VaultStore};

/// `meta` key the id is stored under.
pub const META_VAULT_ID: &str = "vault_id";

/// The vault's id, or `None` for a vault created before ids existed.
pub fn vault_id(store: &impl VaultStore) -> Result<Option<String>> {
    Ok(store.meta_get(META_VAULT_ID)?.filter(|id| !id.is_empty()))
}

/// Give the vault a fresh id, replacing any it had. For creation, and for the
/// one upgrade case where no remote exists to adopt from.
pub fn assign_vault_id(store: &impl VaultStore) -> Result<String> {
    let id = crate::crypto::random_hex_id();
    store.meta_set(META_VAULT_ID, &id)?;
    Ok(id)
}

/// Take on an id that already names this vault elsewhere — the remote pack an
/// upgraded install found under `Vaults/`.
pub fn adopt_vault_id(store: &impl VaultStore, id: &str) -> Result<()> {
    store.meta_set(META_VAULT_ID, id)
}

/// `meta` keys the vault's name is stored under: the label, and the ms-epoch
/// stamp it was last set at.
pub const META_VAULT_NAME: &str = "vault_name";
pub const META_VAULT_NAME_MS: &str = "vault_name_updated_ms";

/// The stamp a name gets when it was *migrated* into the vault rather than
/// chosen in it — the label this device's workspace registry already held (see
/// `commands::auth::seed_vault_name`).
///
/// Such a label says nothing about when the user picked it, only that it
/// predates the build that put names inside vaults. Stamping it `now` would let
/// an old device joining a staggered rollout outrank a rename another device
/// published yesterday, so it takes the smallest stamp that still counts as
/// named: one past the `0` an unnamed vault reads as. Every real rename beats
/// it, and it still beats a vault nobody has named.
pub const MIGRATED_NAME_MS: i64 = 1;

/// The vault's name and when it was set. A vault nobody has named reads as
/// `(None, 0)`, which loses to every stamped name there is.
pub fn vault_name(store: &impl VaultStore) -> Result<(Option<String>, i64)> {
    let name = store
        .meta_get(META_VAULT_NAME)?
        .filter(|name| !name.is_empty());
    let at_ms = store
        .meta_get(META_VAULT_NAME_MS)?
        .and_then(|ms| ms.parse().ok())
        .unwrap_or(0);
    Ok((name, at_ms))
}

/// Name the vault, stamping when. The stamp travels with the name so another
/// device can tell which of two renames came last — which is why the two go
/// down as one write: a new name left under the stamp of the name before it
/// reads as stale everywhere, and the rename would never leave this device.
///
/// An empty `name` is how a vault is put back to unnamed; [`vault_name`] reads
/// it as `None`.
pub fn set_vault_name(store: &impl VaultStore, name: &str, at_ms: i64) -> Result<()> {
    let at_ms = at_ms.to_string();
    store.meta_set_many(&[(META_VAULT_NAME, name), (META_VAULT_NAME_MS, &at_ms)])
}

/// The stamp to write for a rename the user just made here, over a vault that
/// currently holds a name stamped `held_ms`.
///
/// A rename made here must outrank whatever this vault held, whichever clock
/// wrote it; wall-clock is only a tie-breaker between devices. Taking `now_ms`
/// alone loses the rename whenever the held stamp came from a device running
/// ahead of this one: it reads as older, the next sync hands the old name back,
/// and the push is skipped because both sides then agree — the user's explicit
/// choice gone with no error to show for it. One past the held stamp is the
/// smallest stamp [`name_wins`] still lets through, so a skewed clock costs the
/// rename nothing but its place in wall-clock order.
pub fn rename_stamp(held_ms: i64, now_ms: i64) -> i64 {
    now_ms.max(held_ms.saturating_add(1))
}

/// Whether `candidate` should replace `held` as the vault's name.
///
/// One total order over the pair, used by both halves of a sync run — the
/// adopt and the push decision — so the two agree by construction and two
/// devices comparing the same two names always pick the same winner. The later
/// stamp wins; when the stamps are equal (two renames in the same millisecond,
/// which strict recency cannot separate) the name itself breaks the tie, the
/// way `SqliteStore::merge_records` breaks a timestamp tie on the record hash.
/// Unnamed sorts below every name, so `(None, 0)` loses to all of them.
pub fn name_wins(candidate: (Option<&str>, i64), held: (Option<&str>, i64)) -> bool {
    (candidate.1, candidate.0) > (held.1, held.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SqliteStore;

    fn store() -> SqliteStore {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rowel-identity-{}-{}.db",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        SqliteStore::open(&path, &[7u8; 32]).unwrap()
    }

    #[test]
    fn a_new_store_has_no_id_until_one_is_assigned() {
        let store = store();
        assert_eq!(vault_id(&store).unwrap(), None);
        let id = assign_vault_id(&store).unwrap();
        assert_eq!(id.len(), 32);
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(vault_id(&store).unwrap().as_deref(), Some(id.as_str()));
    }

    #[test]
    fn adopting_takes_the_given_id_verbatim() {
        let store = store();
        adopt_vault_id(&store, "a1b2").unwrap();
        assert_eq!(vault_id(&store).unwrap().as_deref(), Some("a1b2"));
    }

    #[test]
    fn an_empty_stored_id_reads_as_none() {
        let store = store();
        store.meta_set(META_VAULT_ID, "").unwrap();
        assert_eq!(vault_id(&store).unwrap(), None);
    }

    #[test]
    fn a_name_round_trips_with_the_stamp_it_was_set_at() {
        let store = store();
        assert_eq!(vault_name(&store).unwrap(), (None, 0));

        set_vault_name(&store, "Work", 1_700_000_000_000).unwrap();
        assert_eq!(
            vault_name(&store).unwrap(),
            (Some("Work".to_string()), 1_700_000_000_000)
        );
    }

    // A vault named before the stamp existed still reads, and reads as the
    // oldest name there is — so the first device to rename it wins.
    #[test]
    fn a_name_with_no_stamp_reads_as_stamped_zero() {
        let store = store();
        store.meta_set(META_VAULT_NAME, "Work").unwrap();
        assert_eq!(vault_name(&store).unwrap(), (Some("Work".to_string()), 0));
    }

    // How a rename that could not be mirrored is put back.
    #[test]
    fn an_empty_name_reads_as_unnamed_again() {
        let store = store();
        set_vault_name(&store, "Work", 500).unwrap();
        set_vault_name(&store, "", 0).unwrap();
        assert_eq!(vault_name(&store).unwrap(), (None, 0));
    }

    // The ordinary case: this clock is the later one, so the rename is stamped
    // when it happened and keeps its place among the other devices' renames.
    #[test]
    fn a_rename_is_stamped_now_when_now_is_already_the_later_clock() {
        assert_eq!(
            rename_stamp(1_600_000_000_000, 1_700_000_000_000),
            1_700_000_000_000
        );
        assert_eq!(rename_stamp(0, 1_700_000_000_000), 1_700_000_000_000);
    }

    // The held name was written by a device running ahead of this one. Stamped
    // `now` the rename would read as the older of the two and be undone by the
    // next pull, so it takes the smallest stamp that still outranks what it
    // replaces.
    #[test]
    fn a_rename_over_a_stamp_from_a_faster_clock_still_outranks_it() {
        let held = 1_700_000_000_000;
        let stamp = rename_stamp(held, 1_600_000_000_000);
        assert_eq!(stamp, held + 1);
        assert!(name_wins((Some("Home"), stamp), (Some("Work"), held)));
    }

    #[test]
    fn the_later_stamp_wins() {
        assert!(name_wins((Some("New"), 2), (Some("Old"), 1)));
        assert!(!name_wins((Some("Old"), 1), (Some("New"), 2)));
    }

    // Two devices renaming in the same millisecond: the stamp cannot separate
    // them, so the name does. Whichever side runs the comparison, it picks the
    // same winner — which is what makes the two converge instead of pushing at
    // each other forever.
    #[test]
    fn a_tied_stamp_is_broken_by_the_name_itself() {
        assert!(name_wins((Some("Work"), 5), (Some("Home"), 5)));
        assert!(!name_wins((Some("Home"), 5), (Some("Work"), 5)));
        // And a pair never beats itself, so agreement is the end of it.
        assert!(!name_wins((Some("Work"), 5), (Some("Work"), 5)));
    }

    #[test]
    fn any_name_beats_an_unnamed_vault_and_a_real_rename_beats_a_migrated_label() {
        assert!(name_wins((Some("Work"), MIGRATED_NAME_MS), (None, 0)));
        assert!(!name_wins((None, 0), (Some("Work"), MIGRATED_NAME_MS)));
        assert!(name_wins(
            (Some("Work"), 500),
            (Some("Laptop"), MIGRATED_NAME_MS)
        ));
    }
}
