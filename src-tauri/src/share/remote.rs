//! Where a one-time share lives while it is outstanding, as the verbs in
//! [`super`] see it: two synchronous traits, one for the sender's account and
//! one for the recipient's account-less download. A provider that supports
//! sharing implements both (Drive's are `sync::providers::gdrive::share`);
//! [`AnyShareRemote`] is how the commands reach whichever that is.
//!
//! The account only ever holds ciphertext plus opaque bookkeeping (an entry
//! UUID, a kind, an expiry), so its listing tells the *sender's* UI what is
//! outstanding without telling the provider what was shared.

#[cfg(test)]
use std::sync::Mutex;

#[cfg(test)]
use crate::error::Error;
use crate::error::Result;
use crate::sync::providers::gdrive::share::DriveShareRemote;

/// File properties on a share (Drive's `appProperties`). Values are opaque to
/// the provider.
pub const PROP_ENTRY_ID: &str = "entryId";
pub const PROP_KIND: &str = "kind";
pub const PROP_EXPIRES_AT: &str = "expiresAt";
/// Which of the sender's vaults published the share. Opaque to Google like the
/// rest; it is what lets one account's several vaults each list their own.
pub const PROP_VAULT_ID: &str = "vaultId";
/// The marker every share carries, and the only thing a listing selects on:
/// shares are found by it wherever they sit, so a duplicate `Shares` folder
/// created by a racing device hides nothing from the sweep or the revoke list.
pub const PROP_SHARE: &str = "rowelShare";
pub const PROP_SHARE_VALUE: &str = "1";

/// One outstanding share, as the sender's UI sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareFile {
    pub id: String,
    /// Whether the file carries the `rowelShare` marker — whether, in other
    /// words, it is one of ours at all. Always true of anything a listing
    /// returned, since that is the one thing a listing selects on. It is a field
    /// for [`ShareRemote::get`], which reads whatever file id a caller handed
    /// in and has to be able to say "that is not a share of this app's".
    pub marked: bool,
    /// The sender's local entry id from appProperties (`entryId`), if present.
    /// An opaque UUID: the sender's UI resolves it to a title; Google learns
    /// nothing.
    pub entry_id: Option<String>,
    /// Entry kind from appProperties (`kind`), e.g. "login".
    pub kind: Option<String>,
    /// The vault that published it (`vaultId`), and the only thing that says
    /// whose share this is. Absent on shares created before the property
    /// existed, which are nobody's: no vault lists or revokes them, and the
    /// account-wide sweep clears them when they expire — see
    /// [`crate::share::list`].
    pub vault_id: Option<String>,
    pub created_ms: i64,
    /// Absent when the file predates expiry bookkeeping or carries a malformed
    /// value — treated as "no known expiry" rather than "expired now", so a
    /// sweep never deletes a share it simply failed to read.
    pub expires_ms: Option<i64>,
}

/// The sender-side store of outstanding shares.
pub trait ShareRemote {
    /// Upload sealed bytes as a new share file; returns the Drive file id.
    fn upload(&self, name: &str, bytes: &[u8], properties: &[(&str, &str)]) -> Result<String>;
    /// Make the file readable by anyone holding its id.
    fn make_public(&self, id: &str) -> Result<()>;
    fn delete(&self, id: &str) -> Result<()>;
    /// Every share file currently in the Shares folder.
    fn list(&self) -> Result<Vec<ShareFile>>;
    /// One file by id, or `None` when there is no such file at all.
    ///
    /// Addressed rather than searched for, so it answers about a file the
    /// listing index has not picked up yet — which is what a revoke moments
    /// after an upload depends on. The file may be anything the caller named, so
    /// the answer carries [`ShareFile::marked`] and the caller checks it.
    fn get(&self, id: &str) -> Result<Option<ShareFile>>;
}

/// Recipient side: no account, just the public download.
pub trait PublicFetch {
    fn download(&self, file_id: &str) -> Result<Vec<u8>>;
}

/// Every provider that supports sharing, behind one type: match and forward.
/// A provider without share support has no variant (see
/// [`super::remote_for`]).
pub enum AnyShareRemote {
    GoogleDrive(DriveShareRemote),
}

impl ShareRemote for AnyShareRemote {
    fn upload(&self, name: &str, bytes: &[u8], properties: &[(&str, &str)]) -> Result<String> {
        match self {
            AnyShareRemote::GoogleDrive(remote) => remote.upload(name, bytes, properties),
        }
    }

    fn make_public(&self, id: &str) -> Result<()> {
        match self {
            AnyShareRemote::GoogleDrive(remote) => remote.make_public(id),
        }
    }

    fn delete(&self, id: &str) -> Result<()> {
        match self {
            AnyShareRemote::GoogleDrive(remote) => remote.delete(id),
        }
    }

    fn list(&self) -> Result<Vec<ShareFile>> {
        match self {
            AnyShareRemote::GoogleDrive(remote) => remote.list(),
        }
    }

    fn get(&self, id: &str) -> Result<Option<ShareFile>> {
        match self {
            AnyShareRemote::GoogleDrive(remote) => remote.get(id),
        }
    }
}

/// An in-memory stand-in for the Drive folder, implementing both sides of the
/// exchange so a test can upload, publish and download without a network.
///
/// `download` is deliberately strict about the public flag: a share that was
/// never published is exactly as unreachable to a recipient as one that was
/// deleted, and it fails with the same message the real client gives.
#[cfg(test)]
pub(crate) struct FakeShareRemote {
    files: Mutex<std::collections::BTreeMap<String, FakeFile>>,
    /// Doubles as the id source and the createdTime clock, so uploads are
    /// ordered and ids are stable across a test.
    next: Mutex<i64>,
    /// Fails `make_public`, for the caller that has to clean up after it.
    fail_publishing: std::sync::atomic::AtomicBool,
    /// Held, but not yet findable by a listing — Drive's query index lags the
    /// files it indexes, and a share revoked seconds after it was uploaded is
    /// exactly where that shows.
    unindexed: Mutex<std::collections::BTreeSet<String>>,
}

#[cfg(test)]
pub(crate) struct FakeFile {
    pub bytes: Vec<u8>,
    pub properties: std::collections::BTreeMap<String, String>,
    pub public: bool,
    pub created_ms: i64,
}

#[cfg(test)]
impl FakeShareRemote {
    pub(crate) fn new() -> Self {
        Self {
            files: Mutex::new(Default::default()),
            next: Mutex::new(1),
            fail_publishing: std::sync::atomic::AtomicBool::new(false),
            unindexed: Mutex::new(Default::default()),
        }
    }

    /// Make every later `make_public` fail, leaving the upload behind.
    pub(crate) fn fail_publishing(&self) {
        self.fail_publishing
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Keep a file out of every later listing while `get` goes on answering for
    /// it, the way Drive's index does until it catches up.
    pub(crate) fn unindex(&self, id: &str) {
        self.unindexed.lock().unwrap().insert(id.to_string());
    }

    /// Ids of every file currently held, oldest first.
    pub(crate) fn ids(&self) -> Vec<String> {
        self.files.lock().unwrap().keys().cloned().collect()
    }

    pub(crate) fn is_public(&self, id: &str) -> bool {
        self.files.lock().unwrap().get(id).is_some_and(|f| f.public)
    }

    pub(crate) fn bytes(&self, id: &str) -> Option<Vec<u8>> {
        self.files.lock().unwrap().get(id).map(|f| f.bytes.clone())
    }
}

/// One held file read as a share, the way Drive's `parse_share_file` reads a real
/// listing row — shared by the fake's `list` and `get` so the two cannot come to
/// disagree about the same file.
#[cfg(test)]
fn read_fake(id: &str, file: &FakeFile) -> ShareFile {
    ShareFile {
        id: id.to_string(),
        marked: file.properties.get(PROP_SHARE).map(String::as_str) == Some(PROP_SHARE_VALUE),
        entry_id: file.properties.get(PROP_ENTRY_ID).cloned(),
        kind: file.properties.get(PROP_KIND).cloned(),
        vault_id: file.properties.get(PROP_VAULT_ID).cloned(),
        created_ms: file.created_ms,
        expires_ms: file
            .properties
            .get(PROP_EXPIRES_AT)
            .and_then(|v| v.parse().ok()),
    }
}

#[cfg(test)]
impl ShareRemote for FakeShareRemote {
    fn upload(&self, _name: &str, bytes: &[u8], properties: &[(&str, &str)]) -> Result<String> {
        let mut next = self.next.lock().unwrap();
        let id = format!("file-{next}");
        self.files.lock().unwrap().insert(
            id.clone(),
            FakeFile {
                bytes: bytes.to_vec(),
                properties: properties
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                    .collect(),
                public: false,
                created_ms: *next,
            },
        );
        *next += 1;
        Ok(id)
    }

    fn make_public(&self, id: &str) -> Result<()> {
        if self
            .fail_publishing
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(Error::Other("could not publish the share".into()));
        }
        match self.files.lock().unwrap().get_mut(id) {
            Some(file) => {
                file.public = true;
                Ok(())
            }
            None => Err(Error::NotFound),
        }
    }

    fn delete(&self, id: &str) -> Result<()> {
        self.files.lock().unwrap().remove(id);
        Ok(())
    }

    // Selects on the marker exactly as the real listing does, so a caller that
    // forgets to set it finds out here.
    fn list(&self) -> Result<Vec<ShareFile>> {
        let unindexed = self.unindexed.lock().unwrap();
        Ok(self
            .files
            .lock()
            .unwrap()
            .iter()
            .map(|(id, file)| read_fake(id, file))
            .filter(|file| file.marked && !unindexed.contains(&file.id))
            .collect())
    }

    // Addressed, not searched: every file the fake holds answers here, marker or
    // no marker, because that is what the caller has to be able to tell apart.
    fn get(&self, id: &str) -> Result<Option<ShareFile>> {
        Ok(self
            .files
            .lock()
            .unwrap()
            .get(id)
            .map(|file| read_fake(id, file)))
    }
}

#[cfg(test)]
impl PublicFetch for FakeShareRemote {
    fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        self.files
            .lock()
            .unwrap()
            .get(file_id)
            .filter(|f| f.public)
            .map(|f| f.bytes.clone())
            .ok_or(Error::ShareExpired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_uploaded_share_is_listed_with_its_properties() {
        let remote = FakeShareRemote::new();
        let id = remote
            .upload(
                "share.rowelshare",
                b"sealed",
                &[
                    (PROP_SHARE, PROP_SHARE_VALUE),
                    (PROP_ENTRY_ID, "entry-1"),
                    (PROP_KIND, "login"),
                ],
            )
            .unwrap();
        // Uploaded without the marker: present, but not a share to the listing.
        remote.upload("stray.bin", b"x", &[]).unwrap();

        let listed = remote.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
        assert_eq!(listed[0].entry_id.as_deref(), Some("entry-1"));
        assert_eq!(listed[0].kind.as_deref(), Some("login"));
        // Both files are held; only one is a share.
        assert_eq!(remote.ids().len(), 2);
    }

    // `get` addresses a file instead of searching for it, so it answers about
    // things a listing never shows: the stray below, and — on the real Drive —
    // a share the query index has not caught up with.
    #[test]
    fn getting_by_id_answers_for_any_file_and_says_which_are_shares() {
        let remote = FakeShareRemote::new();
        let share = remote
            .upload(
                "share.rowelshare",
                b"sealed",
                &[(PROP_SHARE, PROP_SHARE_VALUE), (PROP_VAULT_ID, "a1b2")],
            )
            .unwrap();
        let stray = remote.upload("stray.bin", b"x", &[]).unwrap();

        let got = remote.get(&share).unwrap().unwrap();
        assert!(got.marked);
        assert_eq!(got.vault_id.as_deref(), Some("a1b2"));

        let got = remote.get(&stray).unwrap().unwrap();
        assert!(!got.marked);

        assert_eq!(remote.get("never-existed").unwrap(), None);
    }

    #[test]
    fn publishing_is_what_makes_a_share_downloadable() {
        let remote = FakeShareRemote::new();
        let id = remote.upload("share.rowelshare", b"sealed", &[]).unwrap();

        assert!(!remote.is_public(&id));
        assert!(remote.download(&id).is_err());

        remote.make_public(&id).unwrap();
        assert!(remote.is_public(&id));
        assert_eq!(remote.download(&id).unwrap(), b"sealed");
        assert_eq!(remote.bytes(&id).unwrap(), b"sealed");
    }

    #[test]
    fn an_unknown_or_revoked_share_fails_the_way_the_recipient_is_told() {
        let remote = FakeShareRemote::new();
        let id = remote.upload("share.rowelshare", b"sealed", &[]).unwrap();
        remote.make_public(&id).unwrap();
        remote.delete(&id).unwrap();

        for missing in [id.as_str(), "never-existed"] {
            let err = remote.download(missing).unwrap_err().to_string();
            assert_eq!(err, "this share has expired or was revoked");
        }
    }

    #[test]
    fn deleting_removes_the_share_and_deleting_twice_is_fine() {
        let remote = FakeShareRemote::new();
        let id = remote.upload("share.rowelshare", b"sealed", &[]).unwrap();

        remote.delete(&id).unwrap();
        assert!(remote.list().unwrap().is_empty());
        assert!(remote.ids().is_empty());
        // Revoke and the expiry sweep race each other; whoever loses still
        // got what it asked for.
        remote.delete(&id).unwrap();
        remote.delete("never-existed").unwrap();
    }
}
