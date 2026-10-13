//! What a *keyless* install can ask of a connected Drive account.
//!
//! During first-run onboarding there is no vault, so none of the usual entry
//! points apply: [`super::run`] needs a session cryptor to read the token file
//! with, and the token file itself cannot exist until a vault key does. The
//! two questions onboarding actually has — which vaults are up there, give me
//! this one's bytes — are answered here against [`drive`] directly, with an
//! access token the caller holds in memory.
//!
//! (Not to be confused with [`super::setup`], the desktop consent flow for an
//! install that already has a vault.)

use reqwest::Client;
use serde::Serialize;

use super::{drive, layout, pack};
use crate::error::{Error, Result};

/// A remote vault as the onboarding screen describes it — enough for the user
/// to recognise the account as theirs, and to choose between two, without
/// downloading a byte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackInfo {
    /// The Drive file id. The frontend hands it back to name which of several
    /// vaults the user chose to restore.
    pub id: String,
    pub name: String,
    /// The vault this pack holds, read out of its file name — which is also the
    /// only thing that makes a file in the packs folder a pack at all.
    pub vault_id: String,
    /// `0` when Drive reported no length. Onboarding only renders this, and a
    /// missing size is not worth a second shape in the payload.
    pub size: u64,
    /// RFC 3339 UTC, as Drive gives it; the frontend formats it.
    pub modified_time: String,
}

impl PackInfo {
    /// The pack `file` is, or `None` when it is not a live one: an archive the
    /// user deliberately set aside, or whatever else they dropped into the
    /// folder. [`layout::vault_id_of`] is what tells them apart, so reading the
    /// vault id and recognising the file are the same act.
    fn of(file: &drive::DriveFile) -> Option<Self> {
        Some(Self {
            id: file.id.clone(),
            name: file.name.clone(),
            vault_id: layout::vault_id_of(&file.name)?.to_owned(),
            size: file.size.unwrap_or(0),
            modified_time: file.modified_time.clone(),
        })
    }
}

/// Every vault in the account a first run could restore from, newest first.
///
/// An empty list covers "no Rowel folder", "no packs folder in it" (neither
/// `Vaults/` nor, after the cutover, `Lockboxes/`) and "a packs folder with
/// nothing restorable in it" — to onboarding they are the same answer, and the
/// same fresh start.
///
/// All of them rather than one: two installs syncing their own primary vault to
/// the same Google account each mint their own vault id, so the account holds a
/// pack per vault. Picking one here would hide the rest, and restore or "start
/// fresh" would then act on a vault the user never chose.
pub async fn find_packs(client: &Client, token: &str) -> Result<Vec<PackInfo>> {
    let Some(root) = drive::folder_id(client, token, layout::ROOT_FOLDER).await? else {
        return Ok(Vec::new());
    };
    let vaults = match super::packs_folder_in(client, token, &root).await? {
        Some(vaults) => drive::list_folder(client, token, &vaults).await?,
        None => Vec::new(),
    };
    Ok(restorable_packs(vaults))
}

/// The pack with this Drive file id, re-listed rather than trusted from the
/// probe: the two are minutes apart, and a stale id is a confusing failure
/// where "no vault up there any more" is a clear one.
pub async fn find_pack_by_id(client: &Client, token: &str, file_id: &str) -> Result<PackInfo> {
    find_packs(client, token)
        .await?
        .into_iter()
        .find(|pack| pack.id == file_id)
        .ok_or(Error::NoRemoteVault)
}

/// Which of `vaults` (the contents of `Rowel/Vaults/`, or of
/// `Rowel/Lockboxes/` after the cutover) are vaults to offer, and
/// in what order.
///
/// Live packs only — see [`PackInfo::of`] for what makes one.
///
/// Newest first, because that is the order a picker wants: `modified_time` is
/// RFC 3339 UTC, a fixed-width format whose lexicographic order *is*
/// chronological order, and the file id settles ties so every device shows the
/// same list.
fn restorable_packs(vaults: Vec<drive::DriveFile>) -> Vec<PackInfo> {
    let mut packs: Vec<PackInfo> = vaults.iter().filter_map(PackInfo::of).collect();
    packs.sort_by(|a, b| {
        b.modified_time
            .cmp(&a.modified_time)
            .then_with(|| a.id.cmp(&b.id))
    });
    packs
}

/// Download a pack located by [`find_packs`], under the same size cap the sync
/// engine applies.
pub async fn download_pack(client: &Client, token: &str, id: &str) -> Result<Vec<u8>> {
    drive::read_file(client, token, id, pack::MAX_PACK_BYTES).await
}

#[cfg(test)]
mod tests {
    use super::*;

    // `modified_time` doubles as the created stamp: nothing here reads the
    // latter, and the listing is ordered by the former.
    fn drive_file(id: &str, name: &str, modified: &str) -> drive::DriveFile {
        drive::DriveFile {
            id: id.into(),
            name: name.into(),
            created_time: modified.into(),
            modified_time: modified.into(),
            size: None,
            head_revision: None,
            app_properties: Default::default(),
        }
    }

    fn ids(packs: &[PackInfo]) -> Vec<&str> {
        packs.iter().map(|pack| pack.id.as_str()).collect()
    }

    // One vault in the account is the ordinary case.
    #[test]
    fn the_single_live_pack_is_the_one_to_restore() {
        let pack = drive_file("f1", "a1b2.rowel", "2024-01-01T00:00:00.000Z");
        let packs = restorable_packs(vec![pack]);
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
        assert!(restorable_packs(files).is_empty());
        assert!(restorable_packs(Vec::new()).is_empty());
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
        assert_eq!(ids(&restorable_packs(files)), ["f2"]);
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
        assert_eq!(ids(&restorable_packs(files)), ["f2", "f3", "f1"]);
    }

    // Two packs written in the same second still list in one fixed order, so
    // every device shows the same thing and a chosen id means the same vault.
    #[test]
    fn packs_of_the_same_age_are_ordered_by_file_id() {
        let files = vec![
            drive_file("f2", "beef.rowel", "2024-06-01T00:00:00.000Z"),
            drive_file("f1", "a1b2.rowel", "2024-06-01T00:00:00.000Z"),
        ];
        assert_eq!(ids(&restorable_packs(files)), ["f1", "f2"]);
    }

    #[test]
    fn a_listed_pack_describes_itself_to_onboarding() {
        let mut file = drive_file("f1", "a1b2.rowel", "2024-05-04T10:11:12.000Z");
        file.size = Some(2048);

        let info = PackInfo::of(&file).unwrap();
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
    }

    // A `size` Drive did not report reads as 0 rather than breaking the shape
    // the frontend destructures.
    #[test]
    fn a_pack_of_unknown_size_reports_zero() {
        let file = drive_file("f1", "a1b2.rowel", "");
        assert_eq!(PackInfo::of(&file).unwrap().size, 0);
    }
}
