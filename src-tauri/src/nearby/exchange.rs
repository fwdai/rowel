//! Sync between two devices directly: one lockbox, no account in between
//! (design §3, "Exchange").
//!
//! One side initiates ([`sync_with_peer`]); the other answers three calls about
//! the lockbox ([`head`], [`fetch`], [`absorb`]) over whatever carries them, a
//! [`PeerLink`]. The initiator runs [`engine::sync`] with the peer standing in
//! for the account: the peer's pack is the pull, and the push is this device's
//! pack, which the peer merges. A merge where an account would replace is
//! always safe, because the merge is a join: nothing the peer holds is lost to
//! it, whatever it wrote since its pack was taken.
//!
//! Every exchange that finds a difference moves whole packs; [`revision`] is
//! what lets one that finds none move nothing.
//!
//! Tombstones are where a peer differs from an account. The engine reclaims one
//! after 90 days on the grounds that every device has pulled it by then, which
//! a device met only on the local network may not have. So each lockbox records
//! when it last converged with each device ([`LocalVault::note_converged`]),
//! and `engine::reclaim` holds back every tombstone newer than the oldest of
//! those, and every tombstone once rows have come in from elsewhere since,
//! until that device converges again or is forgotten (`engine::forget_peer`).

use std::cell::Cell;

use crate::error::{Error, Result};
use crate::sync::engine::{self, LocalVault, Reclaim, Remote, RemoteFile, Snapshot};

/// How far apart two devices' clocks may be for a run between them. Every
/// merge is last-writer-wins on the stamps each device's own clock wrote; past
/// this, a fast clock rather than the later edit would decide what survives.
const MAX_CLOCK_SKEW_MS: u64 = 5 * 60 * 1000;

/// What the answering side says about its lockbox before anything moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerHead {
    /// See [`revision`].
    pub revision: String,
    /// The answering device's clock, in ms since the Unix epoch.
    pub now_ms: i64,
}

/// The other device, as the initiating side reaches it: three calls about the
/// lockbox both hold, answered on that device by [`head`], [`fetch`] and
/// [`absorb`].
pub trait PeerLink {
    /// The peer's revision and its clock.
    fn head(&self) -> Result<PeerHead>;
    /// The peer's pack, with the revision it was packed at.
    fn fetch(&self) -> Result<RemoteFile>;
    /// Hand the peer a pack to merge; its revision afterwards.
    fn absorb(&self, pack: &[u8]) -> Result<String>;
}

/// What one run with a peer did here, for the caller's events.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PeerOutcome {
    /// Rows the merge wrote here. Non-zero means the entry list changed.
    pub merged: usize,
    /// Whether the peer's pack carried a newer name for the lockbox, which the
    /// caller mirrors into the workspace registry.
    pub renamed: bool,
}

/// The lockbox's state as one comparable string: the state digest, and the
/// name with the stamp it was set at, which the digest does not cover. Two
/// devices whose revisions are equal have nothing to say to each other.
pub fn revision(local: &dyn LocalVault) -> Result<String> {
    let digest = hex::encode(local.digest()?);
    Ok(match local.name()? {
        (Some(name), at_ms) => format!("{digest}.{at_ms}.{name}"),
        // Distinct from every named revision, an empty name's included.
        (None, at_ms) => format!("{digest}.{at_ms}"),
    })
}

// --- the answering side -----------------------------------------------------

/// [`PeerLink::head`], answered.
pub fn head(local: &dyn LocalVault, now_ms: i64) -> Result<PeerHead> {
    Ok(PeerHead {
        revision: revision(local)?,
        now_ms,
    })
}

/// [`PeerLink::fetch`], answered: the lockbox packed as it is for an upload.
/// The revision is read after the pack, because the pack reclaims first and
/// that may have moved it.
pub fn fetch(local: &dyn LocalVault, now_ms: i64) -> Result<RemoteFile> {
    let bytes = local.pack(Reclaim::at(now_ms))?;
    Ok(RemoteFile {
        bytes,
        revision: revision(local)?,
    })
}

/// What [`absorb`] did here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Absorbed {
    /// Rows the merge wrote. Non-zero means the entry list changed.
    pub merged: usize,
    /// Whether the pack's name beat this lockbox's and was adopted.
    pub renamed: bool,
    /// This lockbox's revision afterwards: the answer the initiator gets.
    pub revision: String,
}

/// [`PeerLink::absorb`], answered: the pack taken in exactly as a pull is
/// ([`engine::merge_pack`]). One that does not open here — another lockbox's,
/// or one sealed under a password this device has not taken on yet — is an
/// error before the store is touched, as it is for a pull.
pub fn absorb(local: &dyn LocalVault, pack: &[u8]) -> Result<Absorbed> {
    let taken = engine::merge_pack(local, pack)?;
    Ok(Absorbed {
        merged: taken.merged,
        renamed: taken.renamed,
        revision: revision(local)?,
    })
}

// --- the initiating side ----------------------------------------------------

/// One run with the device `device_id` over `link`, initiated here:
///
/// 1. The peer's head. Clocks more than five minutes apart stop the run with
///    [`Error::ClockSkew`], before anything moves.
/// 2. Equal revisions: nothing moves.
/// 3. Otherwise [`engine::sync`], the peer standing in for the account. Its
///    errors mean what they mean for a cloud run: `PasswordChangedRemotely`,
///    `VaultUnderRetiredPassword`, another lockbox's pack.
/// 4. Record that the lockbox converged with `device_id` at `now_ms`, equal
///    revisions included: that is what lets the tombstones held back for it go.
///
/// An error from the peer's own [`absorb`] is the run's error too. A peer still
/// under a password this device has replaced refuses this device's pack with
/// `PasswordChangedRemotely` after this side has merged the peer's: that error
/// is the peer's state, not this device's.
///
/// What the run did here comes back however it ended: one that merged the
/// peer's pack and then failed has changed this lockbox all the same.
pub fn sync_with_peer<P: PeerLink>(
    link: &P,
    local: &dyn LocalVault,
    device_id: &str,
    now_ms: i64,
) -> (PeerOutcome, Result<()>) {
    let local = PeerLocal {
        vault: local,
        done: Cell::default(),
    };
    let ended = converge(link, &local, device_id, now_ms);
    (local.done.get(), ended)
}

fn converge<P: PeerLink>(link: &P, local: &PeerLocal, device_id: &str, now_ms: i64) -> Result<()> {
    let head = link.head()?;
    let skew_ms = head.now_ms.saturating_sub(now_ms);
    if skew_ms.unsigned_abs() > MAX_CLOCK_SKEW_MS {
        return Err(Error::ClockSkew { skew_ms });
    }
    if head.revision != revision(local)? {
        // Nothing to settle: the lockbox's id is what the two devices proved
        // they share before the link was opened (design §4.3).
        engine::sync(&PeerRemote(link), local, now_ms, || Ok(()))?;
    }
    local.note_converged(device_id, now_ms)
}

/// The peer as the engine's [`Remote`]. It always holds a pack, and its upload
/// is a merge, so there is nothing to make the write conditional on.
struct PeerRemote<'a, P>(&'a P);

impl<P: PeerLink> Remote for PeerRemote<'_, P> {
    fn fetch(&self) -> Result<Option<RemoteFile>> {
        self.0.fetch().map(Some)
    }

    fn head_revision(&self) -> Result<Option<String>> {
        Ok(Some(self.0.head()?.revision))
    }

    fn upload(&self, bytes: &[u8], _base_revision: Option<&str>) -> Result<String> {
        self.0.absorb(bytes)
    }
}

/// This device's lockbox for a run with a peer: as for a cloud run, except
/// that the push is not recorded. `sync_remote_rev` and `sync_last_ms` describe
/// the account, and a peer's revision is none of its business.
struct PeerLocal<'a> {
    vault: &'a dyn LocalVault,
    /// What the run has done to the lockbox so far.
    done: Cell<PeerOutcome>,
}

impl PeerLocal<'_> {
    fn did(&self, change: impl FnOnce(&mut PeerOutcome)) {
        let mut done = self.done.get();
        change(&mut done);
        self.done.set(done);
    }
}

impl LocalVault for PeerLocal<'_> {
    fn decode(&self, pack_bytes: &[u8]) -> Result<Snapshot> {
        self.vault.decode(pack_bytes)
    }

    fn merge(&self, incoming: &Snapshot) -> Result<usize> {
        let merged = self.vault.merge(incoming)?;
        self.did(|done| done.merged += merged);
        Ok(merged)
    }

    fn digest(&self) -> Result<[u8; 32]> {
        self.vault.digest()
    }

    fn pack(&self, reclaim: Reclaim) -> Result<Vec<u8>> {
        self.vault.pack(reclaim)
    }

    fn note_push(&self, _revision: &str, _at_ms: i64) -> Result<()> {
        Ok(())
    }

    fn note_converged(&self, device_id: &str, at_ms: i64) -> Result<()> {
        self.vault.note_converged(device_id, at_ms)
    }

    fn vault_id(&self) -> Result<Option<String>> {
        self.vault.vault_id()
    }

    fn adopt_vault_id(&self, id: &str) -> Result<()> {
        self.vault.adopt_vault_id(id)
    }

    fn name(&self) -> Result<(Option<String>, i64)> {
        self.vault.name()
    }

    fn adopt_name(&self, name: &str, at_ms: i64) -> Result<()> {
        self.vault.adopt_name(name, at_ms)?;
        self.did(|done| done.renamed = true);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::commands::password_sync::two_devices::{derive, params, Device, NEW, NOW, OLD};
    use crate::store::{Attachment, AttachmentMeta, Record, VaultStore};
    use crate::sync::store::fake::FakeStore;

    const MINUTE_MS: i64 = 60 * 1000;
    const DAY_MS: i64 = 24 * 60 * MINUTE_MS;

    /// The other device in process: the answering side's three functions,
    /// called on it at its own clock, counting the packs that move.
    struct InProcess<'a> {
        peer: &'a Device,
        now_ms: i64,
        fetches: Cell<usize>,
        absorbs: Cell<usize>,
    }

    impl<'a> InProcess<'a> {
        fn new(peer: &'a Device, now_ms: i64) -> Self {
            Self {
                peer,
                now_ms,
                fetches: Cell::new(0),
                absorbs: Cell::new(0),
            }
        }

        fn packs_moved(&self) -> usize {
            self.fetches.get() + self.absorbs.get()
        }
    }

    impl PeerLink for InProcess<'_> {
        fn head(&self) -> Result<PeerHead> {
            head(self.peer, self.now_ms)
        }

        fn fetch(&self) -> Result<RemoteFile> {
            self.fetches.set(self.fetches.get() + 1);
            fetch(self.peer, self.now_ms)
        }

        fn absorb(&self, pack: &[u8]) -> Result<String> {
            self.absorbs.set(self.absorbs.get() + 1);
            Ok(absorb(self.peer, pack)?.revision)
        }
    }

    /// One lockbox, `v1` under the old password, on the device named `id`.
    fn device(id: &'static str) -> Device {
        Device::create(id, "v1", OLD, &params(1, 0))
    }

    /// A run `local` initiates over `link`: what it did, or the error it
    /// ended on.
    fn run_over(link: &InProcess, local: &Device, now_ms: i64) -> Result<PeerOutcome> {
        let (outcome, ended) = sync_with_peer(link, local, link.peer.id, now_ms);
        ended.map(|()| outcome)
    }

    /// A run `local` initiates with `peer`, both clocks at `now_ms`.
    fn run_at(local: &Device, peer: &Device, now_ms: i64) -> Result<PeerOutcome> {
        run_over(&InProcess::new(peer, now_ms), local, now_ms)
    }

    fn run(local: &Device, peer: &Device) -> Result<PeerOutcome> {
        run_at(local, peer, NOW)
    }

    /// The row for `id`, tombstones included.
    fn row(device: &Device, id: &str) -> Option<Record> {
        device
            .store()
            .export_for_sync()
            .unwrap()
            .into_iter()
            .find(|r| r.id == id)
    }

    /// `id` as `device` holds it, re-stamped: edited at `at_ms`, or with
    /// `deleted`, deleted then. The stamps a run compares are then the test's,
    /// not the wall clock's.
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

    // The account bookkeeping a cloud run writes, as `device` holds it.
    fn account_marks(device: &Device) -> (Option<String>, Option<String>) {
        let store = device.store();
        (
            store.meta_get("sync_remote_rev").unwrap(),
            store.meta_get("sync_last_ms").unwrap(),
        )
    }

    #[test]
    fn two_devices_converge_in_one_run_and_the_next_moves_no_pack() {
        let (a, b) = (device("a"), device("b"));
        a.write("1", "made on a");
        b.write("2", "made on b");

        let link = InProcess::new(&b, NOW);
        let outcome = run_over(&link, &a, NOW).unwrap();
        assert_eq!(
            outcome,
            PeerOutcome {
                merged: 1,
                renamed: false,
            }
        );
        assert_eq!((link.fetches.get(), link.absorbs.get()), (1, 1));
        for device in [&a, &b] {
            assert_eq!(device.title("1").as_deref(), Some("made on a"));
            assert_eq!(device.title("2").as_deref(), Some("made on b"));
        }
        assert_eq!(a.digest(), b.digest());

        // Nothing left to say, whichever side starts.
        let again = InProcess::new(&b, NOW);
        assert_eq!(run_over(&again, &a, NOW).unwrap(), PeerOutcome::default());
        let back = InProcess::new(&a, NOW);
        assert_eq!(run_over(&back, &b, NOW).unwrap(), PeerOutcome::default());
        assert_eq!(again.packs_moved() + back.packs_moved(), 0);
    }

    #[test]
    fn three_devices_converge_whatever_order_the_pairs_sync_in() {
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
            // Each pair once, the initiator alternating.
            for (step, &pair) in order.iter().enumerate() {
                let (x, y) = pairs[pair];
                let (local, peer) = if step % 2 == 0 { (x, y) } else { (y, x) };
                run(&devices[local], &devices[peer]).unwrap();
            }
            for device in &devices {
                assert_eq!(device.digest(), devices[0].digest(), "order {order:?}");
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
        run(&a, &b).unwrap();

        // 1: deleted on A, then edited on B. 2: edited on B, then deleted on A.
        stamp(&a, "1", NOW + 10, true);
        b.write("1", "edited on b");
        stamp(&b, "1", NOW + 20, false);
        b.write("2", "edited on b");
        stamp(&b, "2", NOW + 10, false);
        stamp(&a, "2", NOW + 20, true);

        run(&b, &a).unwrap();
        for device in [&a, &b] {
            assert_eq!(device.title("1").as_deref(), Some("edited on b"));
            assert_eq!(device.title("2"), None);
            assert_eq!(row(device, "2").unwrap().deleted_at, Some(NOW + 20));
        }
        assert_eq!(a.digest(), b.digest());
    }

    // "Delete forever" beats an edit made after it, whichever device starts.
    #[test]
    fn a_purged_shell_wins_both_ways() {
        for purger_starts in [true, false] {
            let (a, b) = (device("a"), device("b"));
            a.write("1", "first");
            run(&a, &b).unwrap();

            a.store().delete("1").unwrap();
            a.store().purge("1").unwrap();
            let purged_at = row(&a, "1").unwrap().updated_at;
            b.write("1", "edited after the purge");
            stamp(&b, "1", purged_at + DAY_MS, false);

            if purger_starts {
                run(&a, &b).unwrap();
            } else {
                run(&b, &a).unwrap();
            }
            for device in [&a, &b] {
                let shell = row(device, "1").unwrap();
                assert!(shell.deleted_at.is_some() && shell.payload.is_empty());
            }
            assert_eq!(a.digest(), b.digest());
        }
    }

    #[test]
    fn an_attachment_follows_its_entry() {
        let (a, b) = (device("a"), device("b"));
        a.write("1", "with a file");
        run(&a, &b).unwrap();

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
        run(&b, &a).unwrap();
        assert_eq!(files(&b, "1"), ["f1"]);

        // Into the Trash with its entry, on both.
        a.store().delete("1").unwrap();
        run(&a, &b).unwrap();
        assert!(files(&b, "1").is_empty());
        assert!(b.store().get_attachment("f1").unwrap().is_none());
        assert_eq!(a.digest(), b.digest());
    }

    // Merging the peer's pack adds nothing here, so only the digests say the
    // peer is behind; they do, and the push brings it level.
    #[test]
    fn a_peer_this_device_is_a_strict_superset_of_ends_up_equal() {
        let (a, b) = (device("a"), device("b"));
        a.write("1", "on both");
        run(&a, &b).unwrap();
        a.write("2", "only on a");

        let outcome = run(&a, &b).unwrap();
        assert_eq!(
            outcome,
            PeerOutcome {
                merged: 0,
                renamed: false,
            }
        );
        assert_eq!(b.title("2").as_deref(), Some("only on a"));
        assert_eq!(a.digest(), b.digest());
    }

    #[test]
    fn clocks_six_minutes_apart_are_refused_and_nothing_changes() {
        let (a, b) = (device("a"), device("b"));
        a.write("1", "on a");
        b.write("2", "on b");
        let before = (a.digest(), b.digest());

        for skew in [6 * MINUTE_MS, -6 * MINUTE_MS] {
            let link = InProcess::new(&b, NOW + skew);
            match run_over(&link, &a, NOW) {
                Err(Error::ClockSkew { skew_ms }) => assert_eq!(skew_ms, skew),
                other => panic!("expected a clock skew error, got {other:?}"),
            }
            assert_eq!(link.packs_moved(), 0);
        }
        assert_eq!((a.digest(), b.digest()), before);
        assert!(a.store().meta_under("sync_peer:").unwrap().is_empty());

        // Within the bound, the same two converge.
        let link = InProcess::new(&b, NOW + 4 * MINUTE_MS);
        run_over(&link, &a, NOW).unwrap();
        assert_eq!(a.digest(), b.digest());
    }

    // Another lockbox's pack does not open, whichever way it travels: under
    // another password and salt (another descriptor), or another password
    // alone (the same descriptor, another key).
    #[test]
    fn a_pack_from_another_lockbox_is_refused_and_nothing_merges() {
        let a = device("a");
        a.write("1", "mine");
        for other in [
            Device::create("x", "v1", "another password", &params(9, 0)),
            Device::create("y", "v1", "another password", &params(1, 0)),
        ] {
            other.write("2", "theirs");
            let before = (a.digest(), other.digest());

            assert!(run(&a, &other).is_err());
            assert!(run(&other, &a).is_err());
            let theirs = fetch(&other, NOW).unwrap().bytes;
            assert!(absorb(&a, &theirs).is_err());

            assert_eq!((a.digest(), other.digest()), before);
            assert_eq!(a.title("2"), None);
        }
    }

    #[test]
    fn a_password_change_on_one_device_stops_the_other_as_a_cloud_run_does() {
        let gen1 = params(2, 1);
        let mut a = device("a");
        let mut b = device("b");
        a.write("1", "first");
        run(&a, &b).unwrap();

        a.rekey(derive(NEW, &gen1), &gen1);
        b.write("2", "made on b");
        let b_before = b.digest();

        // B, still on the old password, cannot open A's pack: the error a
        // cloud run gets, and nothing merged.
        assert!(matches!(run(&b, &a), Err(Error::PasswordChangedRemotely)));
        assert_eq!(b.digest(), b_before);
        let account = FakeStore::default();
        a.sync(&account).unwrap();
        assert!(matches!(
            b.sync(&account),
            Err(Error::PasswordChangedRemotely)
        ));

        // A opens B's pack under the retired key and merges it; B refuses A's
        // pack in turn, so A's run ends with B's error and B is unchanged. The
        // run still says what it merged on A.
        let (outcome, ended) = sync_with_peer(&InProcess::new(&b, NOW), &a, b.id, NOW);
        assert!(matches!(ended, Err(Error::PasswordChangedRemotely)));
        assert_eq!(outcome.merged, 1);
        assert_eq!(a.title("2").as_deref(), Some("made on b"));
        assert_eq!(b.digest(), b_before);

        // Once B takes the new password, one run brings them level.
        b.rekey(derive(NEW, &gen1), &gen1);
        run(&a, &b).unwrap();
        assert_eq!(a.digest(), b.digest());
    }

    #[test]
    fn a_tombstone_is_held_for_a_device_not_met_since_the_delete() {
        const LATER: i64 = NOW + 100 * DAY_MS;
        let (a, b) = (device("a"), device("b"));
        a.write("1", "deleted on a while b is away");
        stamp(&a, "1", NOW, false);
        run(&a, &b).unwrap();
        stamp(&a, "1", NOW + DAY_MS, true);

        // A hundred days on, past the TTL, A packs (for the account, for
        // another device): the tombstone stays, since B has not been met since.
        a.pack(Reclaim::at(LATER)).unwrap();
        assert!(row(&a, "1").is_some());

        // B, back at last, takes the delete rather than handing the entry back.
        run_at(&b, &a, LATER).unwrap();
        for device in [&a, &b] {
            assert_eq!(device.title("1"), None);
        }
        assert_eq!(a.digest(), b.digest());

        // A answered that run rather than starting it, so its own record of B
        // still predates the delete; forgetting B is what lets the tombstone go.
        a.pack(Reclaim::at(LATER)).unwrap();
        assert!(row(&a, "1").is_some());
        engine::forget_peer(a.store(), b.id).unwrap();
        a.pack(Reclaim::at(LATER)).unwrap();
        assert!(row(&a, "1").is_none());
    }

    // The time two devices last converged says nothing of a delete that
    // arrives afterwards with an older stamp: one brought the long way round,
    // or made on a device whose clock runs behind.
    #[test]
    fn a_tombstone_that_arrives_after_a_device_was_last_met_is_held_for_it() {
        const LATER: i64 = NOW + 100 * DAY_MS;
        let (a, b, c) = (device("a"), device("b"), device("c"));
        a.write("1", "deleted on c, which a meets again after b");
        stamp(&a, "1", NOW - DAY_MS, false);
        run(&a, &c).unwrap();
        stamp(&c, "1", NOW, true);

        // A meets B a day after the delete was stamped, and only then C.
        run_at(&a, &b, NOW + DAY_MS).unwrap();
        assert!(b.title("1").is_some());
        run_at(&a, &c, NOW + 2 * DAY_MS).unwrap();
        assert_eq!(a.title("1"), None);

        // Past the TTL the tombstone stays on A: B has not been met since it
        // came. B, back at last, takes the delete instead of undoing it.
        a.pack(Reclaim::at(LATER)).unwrap();
        assert!(row(&a, "1").is_some());
        run_at(&b, &a, LATER).unwrap();
        assert_eq!(b.title("1"), None);
        assert_eq!(a.digest(), b.digest());
    }

    #[test]
    fn a_lockbox_on_an_account_and_a_peer_converges_and_the_peer_run_leaves_the_account_marks() {
        let account = FakeStore::default();
        let a = device("a");
        a.write("1", "from a");
        assert!(a.sync(&account).unwrap().pushed);
        let b = Device::restore("b", "v1", &account, OLD).unwrap();
        a.write("3", "from a, not yet pushed");
        b.write("2", "from b, met nearby");
        let (a_marks, b_marks) = (account_marks(&a), account_marks(&b));
        assert!(a_marks.0.is_some());

        // Each has a row the other lacks, so the run pulls and pushes.
        let outcome = run(&a, &b).unwrap();
        assert_eq!(outcome.merged, 1);
        assert_eq!(account_marks(&a), a_marks);
        assert_eq!(account_marks(&b), b_marks);

        // A now differs from the account and pushes; B already matches it.
        assert!(a.sync(&account).unwrap().pushed);
        assert_ne!(account_marks(&a), a_marks);
        assert!(!b.sync(&account).unwrap().pushed);
        assert_eq!(a.digest(), b.digest());
        let c = Device::restore("c", "v1", &account, OLD).unwrap();
        assert_eq!(c.title("2").as_deref(), Some("from b, met nearby"));
        assert_eq!(c.title("3").as_deref(), Some("from a, not yet pushed"));
    }
}
