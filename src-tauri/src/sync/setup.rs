//! What a *keyless* install can ask of a connected account.
//!
//! During first-run onboarding there is no vault, so none of the usual entry
//! points apply: [`super::run`] needs a session cryptor to read the token file
//! with, and the token file itself cannot exist until a vault key does. The
//! two questions onboarding actually has — which vaults are up there, give me
//! this one's bytes — are answered here against a [`VaultStore`] directly,
//! built over tokens the caller holds in memory ([`super::Auth::Pending`]).
//!
//! (Not to be confused with [`super::setup`], the desktop consent flow for an
//! install that already has a vault.)

use serde::Serialize;

use super::store::{RemoteEntry, VaultStore};
use super::{layout, pack, Provider};
use crate::error::{Error, Result};

/// A remote vault as the onboarding screen describes it — enough for the user
/// to recognise the account as theirs, and to choose between two, without
/// downloading a byte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackInfo {
    /// The provider's handle for the file (a Drive file id). The frontend
    /// hands it back to name which of several vaults the user chose to restore.
    pub id: String,
    pub name: String,
    /// The vault this pack holds, read out of its file name — which is also the
    /// only thing that makes a file in `Vaults/` a pack at all.
    pub vault_id: String,
    /// `0` when the provider reported no length. Onboarding only renders this,
    /// and a missing size is not worth a second shape in the payload.
    pub size: u64,
    /// RFC 3339 UTC, as the provider gives it; the frontend formats it.
    pub modified_time: String,
    /// Which account the pack is in.
    pub provider: Provider,
}

impl PackInfo {
    /// The pack `entry` is, or `None` when it is not a live one: an archive the
    /// user deliberately set aside, or whatever else they dropped into the
    /// folder. [`layout::vault_id_of`] is what tells them apart, so reading the
    /// vault id and recognising the file are the same act.
    fn of(entry: &RemoteEntry, provider: Provider) -> Option<Self> {
        Some(Self {
            id: entry.handle.clone(),
            name: entry.name.clone(),
            vault_id: layout::vault_id_of(&entry.name)?.to_owned(),
            size: entry.size.unwrap_or(0),
            modified_time: entry.modified.clone(),
            provider,
        })
    }
}

/// Every vault in the account a first run could restore from, newest first.
///
/// An empty list covers "no Rowel folder", "no Vaults/ in it" and "a Vaults/
/// with nothing restorable in it" — to onboarding they are the same answer, and
/// the same fresh start.
///
/// All of them rather than one: two installs syncing their own primary vault to
/// the same account each mint their own vault id, so the account holds a pack
/// per vault. Picking one here would hide the rest, and restore or "start
/// fresh" would then act on a vault the user never chose.
pub async fn find_packs(store: &impl VaultStore) -> Result<Vec<PackInfo>> {
    Ok(restorable_packs(
        store.list_vaults().await?,
        store.provider(),
    ))
}

/// The pack with this handle, re-listed rather than trusted from the probe: the
/// two are minutes apart, and a stale id is a confusing failure where "no vault
/// up there any more" is a clear one.
pub async fn find_pack_by_id(store: &impl VaultStore, file_id: &str) -> Result<PackInfo> {
    find_packs(store)
        .await?
        .into_iter()
        .find(|pack| pack.id == file_id)
        .ok_or(Error::NoRemoteVault)
}

/// Which of `vaults` (the contents of `Rowel/Vaults/`) are vaults to offer, and
/// in what order.
///
/// Live packs only — see [`PackInfo::of`] for what makes one.
///
/// Newest first, because that is the order a picker wants: `modified_time` is
/// RFC 3339 UTC, a fixed-width format whose lexicographic order *is*
/// chronological order, and the file id settles ties so every device shows the
/// same list.
fn restorable_packs(vaults: Vec<RemoteEntry>, provider: Provider) -> Vec<PackInfo> {
    let mut packs: Vec<PackInfo> = vaults
        .iter()
        .filter_map(|entry| PackInfo::of(entry, provider))
        .collect();
    packs.sort_by(|a, b| {
        b.modified_time
            .cmp(&a.modified_time)
            .then_with(|| a.id.cmp(&b.id))
    });
    packs
}

/// Download a pack located by [`find_packs`], under the same size cap the sync
/// engine applies.
pub async fn download_pack(store: &impl VaultStore, id: &str) -> Result<Vec<u8>> {
    store.read(id, pack::MAX_PACK_BYTES).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOGLE: Provider = Provider::GoogleDrive;

    // A `Vaults/` listing row. Nothing here reads the revision, and the listing
    // is ordered by `modified`.
    fn drive_file(id: &str, name: &str, modified: &str) -> RemoteEntry {
        RemoteEntry {
            handle: id.into(),
            name: name.into(),
            size: None,
            modified: modified.into(),
            revision: None,
        }
    }

    fn ids(packs: &[PackInfo]) -> Vec<&str> {
        packs.iter().map(|pack| pack.id.as_str()).collect()
    }

    // One vault in the account is the ordinary case.
    #[test]
    fn the_single_live_pack_is_the_one_to_restore() {
        let pack = drive_file("f1", "a1b2.rowel", "2024-01-01T00:00:00.000Z");
        let packs = restorable_packs(vec![pack], GOOGLE);
        assert_eq!(ids(&packs), ["f1"]);
        assert_eq!(packs[0].vault_id, "a1b2");
    }

    // Anything that is not a pack is not something to restore: the folder is the
    // user's own Drive and may hold whatever else they dropped in it.
    #[test]
    fn only_a_live_pack_is_restorable() {
        let files = vec![
            drive_file("f1", "a1b2-copy.rowel", "2024-01-01T00:00:00.000Z"),
            drive_file("f2", "notes.txt", "2024-01-01T00:00:00.000Z"),
            drive_file("f3", "a1b2.rowelshare", "2024-01-01T00:00:00.000Z"),
        ];
        assert!(restorable_packs(files, GOOGLE).is_empty());
        assert!(restorable_packs(Vec::new(), GOOGLE).is_empty());
    }

    // The marker a "delete everywhere" leaves behind is not a vault to offer:
    // onboarding, auto-join and the "other vaults in this account" list all
    // read this, and a deleted vault must not come back through any of them.
    #[test]
    fn a_deleted_vaults_marker_is_not_offered() {
        let files = vec![
            drive_file(
                "f1",
                &layout::deleted_marker_name("a1b2"),
                "2024-01-01T00:00:00.000Z",
            ),
            drive_file("f2", "cafe.rowel", "2024-01-01T00:00:00.000Z"),
        ];
        assert_eq!(ids(&restorable_packs(files, GOOGLE)), ["f2"]);
    }

    // Every vault the account holds, so the user picks rather than having one
    // picked for them — newest first, whatever order Drive listed them in.
    #[test]
    fn every_live_pack_is_offered_newest_first() {
        let files = vec![
            drive_file("f2", "beef.rowel", "2024-06-01T00:00:00.000Z"),
            drive_file("f1", "a1b2.rowel", "2024-01-01T00:00:00.000Z"),
            drive_file("f3", "cafe.rowel", "2024-03-01T00:00:00.000Z"),
        ];
        assert_eq!(ids(&restorable_packs(files, GOOGLE)), ["f2", "f3", "f1"]);
    }

    // Two packs written in the same second still list in one fixed order, so
    // every device shows the same thing and a chosen id means the same vault.
    #[test]
    fn packs_of_the_same_age_are_ordered_by_file_id() {
        let files = vec![
            drive_file("f2", "beef.rowel", "2024-06-01T00:00:00.000Z"),
            drive_file("f1", "a1b2.rowel", "2024-06-01T00:00:00.000Z"),
        ];
        assert_eq!(ids(&restorable_packs(files, GOOGLE)), ["f1", "f2"]);
    }

    #[test]
    fn a_listed_pack_describes_itself_to_onboarding() {
        let mut file = drive_file("f1", "a1b2.rowel", "2024-05-04T10:11:12.000Z");
        file.size = Some(2048);

        let info = PackInfo::of(&file, GOOGLE).unwrap();
        assert_eq!(info.id, "f1");
        assert_eq!(info.name, "a1b2.rowel");
        assert_eq!(info.vault_id, "a1b2");
        assert_eq!(info.size, 2048);
        assert_eq!(info.modified_time, "2024-05-04T10:11:12.000Z");

        // Payload keys are the frontend's contract.
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["id"], "f1");
        assert_eq!(json["name"], "a1b2.rowel");
        assert_eq!(json["vaultId"], "a1b2");
        assert_eq!(json["size"], 2048);
        assert_eq!(json["modifiedTime"], "2024-05-04T10:11:12.000Z");
        assert_eq!(json["provider"], "googleDrive");
    }

    // A `size` Drive did not report reads as 0 rather than breaking the shape
    // the frontend destructures.
    #[test]
    fn a_pack_of_unknown_size_reports_zero() {
        let file = drive_file("f1", "a1b2.rowel", "");
        assert_eq!(PackInfo::of(&file, GOOGLE).unwrap().size, 0);
    }

    // The probe and the restore read the account through its store: the
    // listing says whose account it is, a stale id is no vault rather than a
    // confusing failure, and the download is capped like a sync pull.
    #[tokio::test]
    async fn the_probe_and_the_download_go_through_the_store() {
        let store = crate::sync::store::fake::FakeStore::default();
        store.put("a1b2.rowel", b"pack");
        store.put(&layout::deleted_marker_name("dead"), &[]);

        let packs = find_packs(&store).await.unwrap();
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].vault_id, "a1b2");
        assert_eq!(packs[0].provider, GOOGLE);

        let found = find_pack_by_id(&store, &packs[0].id).await.unwrap();
        assert_eq!(download_pack(&store, &found.id).await.unwrap(), b"pack");
        assert_eq!(*store.read_caps.lock().unwrap(), [pack::MAX_PACK_BYTES]);

        assert!(matches!(
            find_pack_by_id(&store, "gone").await,
            Err(Error::NoRemoteVault)
        ));
    }
}
