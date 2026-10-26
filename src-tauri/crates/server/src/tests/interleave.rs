//! Q19's matrix with the server as the third party: device pushes and the
//! server's own writes interleaved, and what every side holds once each has
//! pulled once more. The devices are `packs::Device`, the device's half of
//! the exchange over the core's functions; an agent's write is a row written
//! through the vault as the host writes one, mirrored after.

use std::time::Duration;

use rowel_broker::audit::Event;
use rowel_core::store::{Record, VaultStore};

use super::packs::{agent_writes, assert_chains_verify, deploy, server_copy, Device};
use crate::state::now_ms;

fn record<'a>(records: &'a [Record], id: &str) -> &'a Record {
    records
        .iter()
        .find(|record| record.id == id)
        .unwrap_or_else(|| panic!("no record {id}"))
}

#[tokio::test]
async fn an_agent_write_between_a_devices_head_and_its_put_is_merged_before_the_push_lands() {
    let (api, lockbox) = deploy().await;
    let mut device = Device::join(&api, &lockbox, "pat@acme.example").await;
    device.save("gh", "GitHub");
    device.log(Event::Used);

    // The device's HEAD finds nothing new; an agent's grant writes before
    // its PUT, so the PUT under that revision fails and the retry merges.
    assert!(!device.pull().await);
    agent_writes(&api.state, &lockbox);
    assert_eq!(device.push().await, [412, 200]);

    let server = server_copy(&api.state, &lockbox);
    assert_eq!(device.entries(), server.store.export_for_sync().unwrap());
    let held = device.store.audit_all().unwrap();
    let kept = server.store.audit_all().unwrap();
    assert_eq!(held, kept);
    assert!(kept
        .iter()
        .any(|row| row.event == "granted" && row.writer_id == api.state.writer_id));
    assert!(kept
        .iter()
        .any(|row| row.event == "used" && row.writer_id == device.writer));
    assert_chains_verify(&held);
    assert_chains_verify(&kept);
    assert_eq!(
        device.store.state_digest().unwrap(),
        server.store.state_digest().unwrap()
    );
}

#[tokio::test]
async fn two_devices_and_an_agent_writing_in_turn_converge() {
    let (api, lockbox) = deploy().await;
    let mut ana = Device::join(&api, &lockbox, "ana@acme.example").await;
    let mut ben = Device::join(&api, &lockbox, "ben@acme.example").await;
    assert!(!ana.pull().await);

    for round in 0..4 {
        // Both start the round at one revision.
        ana.pull().await;
        ben.pull().await;
        assert_eq!(ana.etag, ben.etag);
        for (who, device) in [("ana", &ana), ("ben", &ben)] {
            device.save(&format!("{who}-{round}"), who);
            // Both edit one entry too; the merge's total order settles it.
            device.save("shared", &format!("{who} in round {round}"));
            device.log(Event::Used);
        }
        // The first push lands; the second, from the same revision, finds it
        // moved by the first and by the agent, and merges both before it
        // lands. Who goes first alternates.
        let (first, second) = if round % 2 == 0 {
            (&mut ana, &mut ben)
        } else {
            (&mut ben, &mut ana)
        };
        assert_eq!(first.push().await, [200], "round {round}");
        agent_writes(&api.state, &lockbox);
        assert_eq!(second.push().await, [412, 200], "round {round}");
    }
    agent_writes(&api.state, &lockbox);
    assert!(ana.pull().await);
    assert!(ben.pull().await);

    let server = server_copy(&api.state, &lockbox);
    let digest = server.store.state_digest().unwrap();
    assert_eq!(ana.store.state_digest().unwrap(), digest);
    assert_eq!(ben.store.state_digest().unwrap(), digest);
    let entries = server.store.export_for_sync().unwrap();
    for round in 0..4 {
        record(&entries, &format!("ana-{round}"));
        record(&entries, &format!("ben-{round}"));
    }
    let rows = server.store.audit_all().unwrap();
    for writer in [&ana.writer, &ben.writer, &api.state.writer_id] {
        assert!(rows.iter().any(|row| &row.writer_id == writer));
    }
    assert_chains_verify(&rows);
    assert_chains_verify(&ana.store.audit_all().unwrap());
    assert_chains_verify(&ben.store.audit_all().unwrap());
}

/// What decides is `merge_entry_rows`: a row the receiving side does not hold
/// is taken; a purged row (tombstoned and emptied) is absorbing either way;
/// otherwise the higher `(updated_at, derived-column rank, record hash)`
/// wins. A delete stamps `deleted_at` and `updated_at` alike, so a tombstone
/// is a row like any other in that order: a copy last changed before the
/// delete loses to it, and an edit made after the delete beats it. The
/// server never reclaims a tombstone in I7, so a device offline past the TTL
/// meets it however long it was away; on a provider whose pack the devices
/// reclaim, its stale copy would come back as a new entry.
#[tokio::test]
async fn a_device_offline_past_the_tombstone_ttl_meets_the_tombstone_the_server_kept() {
    let (api, lockbox) = deploy().await;
    let mut ana = Device::join(&api, &lockbox, "ana@acme.example").await;
    ana.save("stale", "Old login");
    ana.save("edited", "Edited later");
    assert_eq!(ana.push().await, [200]);
    let mut ben = Device::join(&api, &lockbox, "ben@acme.example").await;
    let held = ben.entries();
    assert_eq!(record(&held, "stale").deleted_at, None);

    // Ben goes offline. Ana deletes both entries and pushes the tombstones;
    // past the TTL her engine reclaims them before packing, and pushes again.
    ana.store.delete("stale").unwrap();
    ana.store.delete("edited").unwrap();
    assert_eq!(ana.push().await, [200]);
    assert_eq!(ana.store.purge_tombstones_before(now_ms() + 1).unwrap(), 2);
    assert_eq!(ana.push().await, [200]);
    let kept = server_copy(&api.state, &lockbox)
        .store
        .export_for_sync()
        .unwrap();
    let tombstone = record(&kept, "stale").clone();
    let deleted_at = tombstone.deleted_at.expect("the server kept the tombstone");
    assert_eq!(tombstone.updated_at, deleted_at);

    // Offline, Ben edits one of them after the delete, and pushes his copy
    // under the current revision — what a device does whose pull carried no
    // tombstone — so the server's merge is what decides.
    tokio::time::sleep(Duration::from_millis(5)).await;
    ben.save("edited", "Edited offline");
    ben.etag = Some(ben.head().await);
    assert_eq!(ben.put().await, 200);

    let merged = server_copy(&api.state, &lockbox)
        .store
        .export_for_sync()
        .unwrap();
    assert!(record(&held, "stale").updated_at < deleted_at);
    assert_eq!(record(&merged, "stale"), &tombstone);
    let edited = record(&merged, "edited");
    assert_eq!(edited.deleted_at, None);
    assert!(edited.updated_at > deleted_at);

    // One more pull each: Ben takes the tombstone, and Ana is handed back the
    // one she reclaimed, since the server still holds it.
    assert!(ben.pull().await);
    assert!(ana.pull().await);
    assert_eq!(record(&ben.entries(), "stale").deleted_at, Some(deleted_at));
    assert_eq!(record(&ana.entries(), "stale").deleted_at, Some(deleted_at));
    let digest = server_copy(&api.state, &lockbox)
        .store
        .state_digest()
        .unwrap();
    assert_eq!(ana.store.state_digest().unwrap(), digest);
    assert_eq!(ben.store.state_digest().unwrap(), digest);
}
