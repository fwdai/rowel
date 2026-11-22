//! One lockbox between two devices, without the channel (design §3).

use crate::error::{Error, Result};
use crate::sync::engine::{self, LocalVault, Reclaim};
use crate::sync::pack::MAX_PACK_BYTES;

// Every merge is last-writer-wins on each device's own clock: past this, a
// fast clock rather than the later edit would decide what survives.
const MAX_CLOCK_SKEW_MS: u64 = 5 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Absorbed {
    pub merged: usize,
    /// The pack's name, when it beat this lockbox's and was taken.
    pub renamed: Option<String>,
    pub revision: String,
}

impl Absorbed {
    pub fn changed(&self) -> bool {
        self.merged > 0 || self.renamed.is_some()
    }
}

pub fn revision(local: &dyn LocalVault) -> Result<String> {
    let digest = hex::encode(local.digest()?);
    Ok(match local.name()? {
        (Some(name), at_ms) => format!("{digest}.{at_ms}.{name}"),
        (None, at_ms) => format!("{digest}.{at_ms}"),
    })
}

pub fn check_clocks(mine_ms: i64, theirs_ms: i64) -> Result<()> {
    let skew_ms = theirs_ms.saturating_sub(mine_ms);
    if skew_ms.unsigned_abs() > MAX_CLOCK_SKEW_MS {
        return Err(Error::ClockSkew { skew_ms });
    }
    Ok(())
}

// The revision is read after the pack, whose reclaim may move it.
pub fn pack(local: &dyn LocalVault, now_ms: i64) -> Result<(Vec<u8>, String)> {
    let bytes = local.pack(Reclaim::at(now_ms))?;
    if bytes.len() > MAX_PACK_BYTES {
        return Err(Error::Other("this lockbox is too large to sync".into()));
    }
    Ok((bytes, revision(local)?))
}

pub fn absorb(local: &dyn LocalVault, pack: &[u8]) -> Result<Absorbed> {
    let taken = engine::merge_pack(local, pack)?;
    let renamed = if taken.renamed { local.name()?.0 } else { None };
    Ok(Absorbed {
        merged: taken.merged,
        renamed,
        revision: revision(local)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::password_sync::two_devices::{derive, params, Device, NEW, NOW, OLD};
    use crate::store::{Attachment, AttachmentMeta, Record, VaultStore};
    use crate::sync::store::fake::FakeStore;

    const MINUTE_MS: i64 = 60 * 1000;
    const DAY_MS: i64 = 24 * 60 * MINUTE_MS;

    fn device(id: &'static str) -> Device {
        Device::create(id, "v1", OLD, &params(1, 0))
    }

    fn rev(device: &Device) -> String {
        revision(device).unwrap()
    }

    fn pack_of(device: &Device) -> Vec<u8> {
        pack(device, NOW).unwrap().0
    }

    // Each takes in the other's pack, the second packed after its merge.
    fn meet(x: &Device, y: &Device) {
        absorb(y, &pack_of(x)).unwrap();
        absorb(x, &pack_of(y)).unwrap();
    }

    fn row(device: &Device, id: &str) -> Option<Record> {
        device
            .store()
            .export_for_sync()
            .unwrap()
            .into_iter()
            .find(|r| r.id == id)
    }

    // Re-stamped at `at_ms`, so the merge compares the test's stamps.
    fn stamp(device: &Device, id: &str, at_ms: i64, deleted: bool) {
        let rec = row(device, id).unwrap();
        device
            .store()
            .import(&[Record {
                updated_at: at_ms,
                deleted_at: deleted.then_some(at_ms),
                ..rec
            }])
            .unwrap();
    }

    fn files(device: &Device, entry_id: &str) -> Vec<String> {
        device
            .store()
            .list_attachments(entry_id)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect()
    }

    #[test]
    fn revisions_are_equal_exactly_when_nothing_differs() {
        let (a, b) = (device("a"), device("b"));
        assert_eq!(rev(&a), rev(&b));
        a.write("1", "made on a");
        assert_ne!(rev(&a), rev(&b));
        meet(&a, &b);
        assert_eq!(rev(&a), rev(&b));
        assert_eq!(pack(&a, NOW).unwrap().1, rev(&b));
    }

    #[test]
    fn a_pack_states_the_revision_after_its_reclaim() {
        let a = device("a");
        a.write("1", "deleted long ago");
        stamp(&a, "1", NOW - 100 * DAY_MS, true);
        let before = rev(&a);
        let (_, packed) = pack(&a, NOW).unwrap();
        assert!(row(&a, "1").is_none());
        assert_ne!(packed, before);
        assert_eq!(packed, rev(&a));
    }

    #[test]
    fn a_newer_name_moves_the_revision_and_is_taken_and_an_older_one_is_not() {
        let (a, b) = (device("a"), device("b"));
        a.adopt_name("Work", NOW).unwrap();
        b.adopt_name("Personal", NOW - 1).unwrap();
        assert_ne!(rev(&a), rev(&b));

        let kept = absorb(&a, &pack_of(&b)).unwrap();
        assert_eq!((kept.merged, kept.renamed), (0, None));
        let taken = absorb(&b, &pack_of(&a)).unwrap();
        assert_eq!(taken.renamed.as_deref(), Some("Work"));
        assert_eq!(taken.revision, rev(&a));
    }

    #[test]
    fn two_devices_end_level_with_one_pack_each_way() {
        let (a, b) = (device("a"), device("b"));
        a.write("1", "made on a");
        b.write("2", "made on b");

        let (to_b, a_at) = pack(&a, NOW).unwrap();
        let taken = absorb(&b, &to_b).unwrap();
        assert!(taken.merged > 0);
        assert_ne!(taken.revision, a_at);
        let (to_a, b_at) = pack(&b, NOW).unwrap();
        let back = absorb(&a, &to_a).unwrap();
        assert!(back.merged > 0);
        assert_eq!(back.revision, b_at);
        for device in [&a, &b] {
            assert_eq!(device.title("1").as_deref(), Some("made on a"));
            assert_eq!(device.title("2").as_deref(), Some("made on b"));
        }
    }

    // Merging the peer's pack adds nothing here, so only the revisions say
    // the peer is behind; this side's pack brings it level.
    #[test]
    fn a_strict_superset_merges_nothing_and_its_pack_brings_the_other_level() {
        let (a, b) = (device("a"), device("b"));
        a.write("1", "on both");
        meet(&a, &b);
        a.write("2", "only on a");

        let kept = absorb(&a, &pack_of(&b)).unwrap();
        assert_eq!(kept.merged, 0);
        assert_ne!(kept.revision, rev(&b));
        assert_eq!(absorb(&b, &pack_of(&a)).unwrap().revision, rev(&a));
        assert_eq!(b.title("2").as_deref(), Some("only on a"));
    }

    #[test]
    fn three_devices_converge_whatever_order_the_pairs_meet_in() {
        let pairs = [(0, 1), (1, 2), (0, 2)];
        let orders = [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ];
        for order in orders {
            let devices = [device("a"), device("b"), device("c")];
            for (n, device) in devices.iter().enumerate() {
                device.write(&n.to_string(), device.id);
            }
            for (step, &pair) in order.iter().enumerate() {
                let (x, y) = pairs[pair];
                let (first, second) = if step % 2 == 0 { (x, y) } else { (y, x) };
                meet(&devices[first], &devices[second]);
            }
            for device in &devices {
                assert_eq!(rev(device), rev(&devices[0]), "order {order:?}");
                for (n, writer) in ["a", "b", "c"].iter().enumerate() {
                    assert_eq!(device.title(&n.to_string()).as_deref(), Some(*writer));
                }
            }
        }
    }

    #[test]
    fn a_delete_and_an_edit_resolve_by_time() {
        let (a, b) = (device("a"), device("b"));
        a.write("1", "first");
        a.write("2", "second");
        stamp(&a, "1", NOW, false);
        stamp(&a, "2", NOW, false);
        meet(&a, &b);

        // 1: deleted on A, then edited on B. 2: edited on B, then deleted on A.
        stamp(&a, "1", NOW + 10, true);
        b.write("1", "edited on b");
        stamp(&b, "1", NOW + 20, false);
        b.write("2", "edited on b");
        stamp(&b, "2", NOW + 10, false);
        stamp(&a, "2", NOW + 20, true);

        meet(&b, &a);
        for device in [&a, &b] {
            assert_eq!(device.title("1").as_deref(), Some("edited on b"));
            assert_eq!(device.title("2"), None);
            assert_eq!(row(device, "2").unwrap().deleted_at, Some(NOW + 20));
        }
        assert_eq!(rev(&a), rev(&b));
    }

    // "Delete forever" beats an edit made after it, whichever device packs first.
    #[test]
    fn a_purged_shell_wins_both_ways() {
        for purger_first in [true, false] {
            let (a, b) = (device("a"), device("b"));
            a.write("1", "first");
            meet(&a, &b);

            a.store().delete("1").unwrap();
            a.store().purge("1").unwrap();
            let purged_at = row(&a, "1").unwrap().updated_at;
            b.write("1", "edited after the purge");
            stamp(&b, "1", purged_at + DAY_MS, false);

            if purger_first {
                meet(&a, &b);
            } else {
                meet(&b, &a);
            }
            for device in [&a, &b] {
                let shell = row(device, "1").unwrap();
                assert!(shell.deleted_at.is_some() && shell.payload.is_empty());
            }
            assert_eq!(rev(&a), rev(&b));
        }
    }

    #[test]
    fn an_attachment_follows_its_entry() {
        let (a, b) = (device("a"), device("b"));
        a.write("1", "with a file");
        meet(&a, &b);

        a.store()
            .insert_attachment(&Attachment {
                meta: AttachmentMeta {
                    id: "f1".into(),
                    entry_id: "1".into(),
                    name: "f1.pdf".into(),
                    mime: None,
                    size: 6,
                    created_at: 0,
                    updated_at: 0,
                    deleted_at: None,
                },
                blob: b"sealed".to_vec(),
            })
            .unwrap();
        meet(&b, &a);
        assert_eq!(files(&b, "1"), ["f1"]);

        a.store().delete("1").unwrap();
        meet(&a, &b);
        assert!(files(&b, "1").is_empty());
        assert!(b.store().get_attachment("f1").unwrap().is_none());
        assert_eq!(rev(&a), rev(&b));
    }

    #[test]
    fn clocks_more_than_five_minutes_apart_are_refused() {
        for skew in [6 * MINUTE_MS, -6 * MINUTE_MS] {
            match check_clocks(NOW, NOW + skew) {
                Err(Error::ClockSkew { skew_ms }) => assert_eq!(skew_ms, skew),
                other => panic!("expected a clock skew error, got {other:?}"),
            }
        }
        for skew in [0, 4 * MINUTE_MS, 5 * MINUTE_MS, -5 * MINUTE_MS] {
            assert!(check_clocks(NOW, NOW + skew).is_ok());
        }
    }

    // Under another password and salt (another descriptor), or another
    // password alone (the same descriptor, another key).
    #[test]
    fn a_pack_from_another_lockbox_does_not_open_and_nothing_merges() {
        let a = device("a");
        a.write("1", "mine");
        for other in [
            Device::create("x", "v1", "another password", &params(9, 0)),
            Device::create("y", "v1", "another password", &params(1, 0)),
        ] {
            other.write("2", "theirs");
            let before = (rev(&a), rev(&other));
            assert!(absorb(&a, &pack_of(&other)).is_err());
            assert!(absorb(&other, &pack_of(&a)).is_err());
            assert_eq!((rev(&a), rev(&other)), before);
            assert_eq!(a.title("2"), None);
        }
    }

    #[test]
    fn after_a_password_change_only_the_side_that_took_it_opens_both_packs() {
        let gen1 = params(2, 1);
        let mut a = device("a");
        let mut b = device("b");
        a.write("1", "first");
        meet(&a, &b);

        a.rekey(derive(NEW, &gen1), &gen1);
        b.write("2", "made on b");
        let b_before = rev(&b);

        // B, still on the old password: the error a cloud run gets.
        assert!(matches!(
            absorb(&b, &pack_of(&a)),
            Err(Error::PasswordChangedRemotely)
        ));
        assert_eq!(rev(&b), b_before);
        let account = FakeStore::default();
        a.sync(&account).unwrap();
        assert!(matches!(
            b.sync(&account),
            Err(Error::PasswordChangedRemotely)
        ));

        // A opens B's pack under the retired key.
        assert!(absorb(&a, &pack_of(&b)).unwrap().merged > 0);
        assert_eq!(a.title("2").as_deref(), Some("made on b"));

        b.rekey(derive(NEW, &gen1), &gen1);
        meet(&a, &b);
        assert_eq!(rev(&a), rev(&b));
    }

    #[test]
    fn a_lockbox_synced_through_an_account_and_nearby_converges_everywhere() {
        let account = FakeStore::default();
        let a = device("a");
        a.write("1", "from a");
        assert!(a.sync(&account).unwrap().pushed);
        let b = Device::restore("b", "v1", &account, OLD).unwrap();
        a.write("3", "from a, not yet pushed");
        b.write("2", "from b, met nearby");

        meet(&a, &b);
        assert!(a.sync(&account).unwrap().pushed);
        assert!(!b.sync(&account).unwrap().pushed);
        assert_eq!(rev(&a), rev(&b));
        let c = Device::restore("c", "v1", &account, OLD).unwrap();
        assert_eq!(c.title("2").as_deref(), Some("from b, met nearby"));
        assert_eq!(c.title("3").as_deref(), Some("from a, not yet pushed"));
    }
}
