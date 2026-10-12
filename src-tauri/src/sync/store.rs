//! Where a vault's pack lives, as the core sees every provider: names and
//! bytes in `<root>/Vaults/`, nothing more.
//!
//! [`VaultStore`] is the one thing an adapter provides. Every rule about
//! *which* name to read or write — the pack, the deletion marker, the order a
//! delete takes them in — is the core's, written once here and in
//! [`super::delete_pack`], so no adapter can get one of them subtly wrong.
//!
//! Runtime dispatch is an enum ([`AnyStore`]), not `Box<dyn>`: `async fn`
//! traits are not object-safe, and there will only ever be a handful of
//! providers.
//!
//! What an adapter must keep (the Drive code's comments defend each):
//!
//! 1. Listings are complete: follow pagination to the last page.
//! 2. `stat` and `read` never answer from a cache; folder ids may be cached per
//!    instance.
//! 3. Downloads are capped before buffering (`sync::read_capped`).
//! 4. No lock is held across a network call; a store is driven with
//!    `block_on` only on blocking-pool threads (`commands::detached` /
//!    `blocking`).
//! 5. A revision is "unknown" (`None`) rather than fabricated when the API has
//!    none.

use tauri::async_runtime::block_on;

use super::account::{Auth, Tokens};
use super::engine::{Remote, RemoteFile};
use super::providers::{dropbox, gdrive};
use super::{layout, pack, Provider};
use crate::error::{Error, Result};

/// One object directly inside `Vaults/`, as a listing or a `stat` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEntry {
    /// Opaque, provider-specific: a Drive file id, a Dropbox `id:` value.
    /// Round-trips through the frontend as `PackInfo.id`.
    pub handle: String,
    pub name: String,
    pub size: Option<u64>,
    /// RFC 3339 UTC; the probe orders by it. Empty when the provider gave none.
    pub modified: String,
    /// Drive's `headRevisionId`, Dropbox's `rev`. `None` is unknown.
    pub revision: Option<String>,
}

/// A provider's view of one account's `Vaults/` folder.
///
/// `create` and `update` are two calls rather than one create-or-overwrite by
/// name on purpose. A push decides between them from a `stat` it has just made,
/// and only the create path asks whether the vault was deleted everywhere
/// ([`may_create_pack`]); an overwrite by name would let a delete landing
/// between that `stat` and the write re-create the pack without the question
/// ever being asked. An update by handle fails instead, as it always has.
pub trait VaultStore {
    fn provider(&self) -> Provider;
    /// Everything directly inside `<root>/Vaults/`. A missing folder is `[]`.
    /// Must be complete: follow pagination to the last page.
    async fn list_vaults(&self) -> Result<Vec<RemoteEntry>>;
    /// The entry named `name` in `Vaults/`, or `None`. Never from a cache.
    async fn stat(&self, name: &str) -> Result<Option<RemoteEntry>>;
    /// Contents by handle, refusing anything over `max_bytes` before it is
    /// buffered whole.
    ///
    /// No revision comes back with the bytes: the one a run records is the
    /// `stat`'s, so that it and [`Remote::head_revision`] read the same field
    /// and degrade identically (see [`PackRemote`]).
    async fn read(&self, handle: &str, max_bytes: usize) -> Result<Vec<u8>>;
    /// A new object named `name` in `Vaults/`, creating folders as needed.
    /// Returns its revision.
    async fn create(&self, name: &str, bytes: &[u8]) -> Result<Option<String>>;
    /// Overwrite the object `handle` names. Returns its new revision; an
    /// object that is gone is an error, never a create.
    async fn update(&self, handle: &str, bytes: &[u8]) -> Result<Option<String>>;
    /// Delete by handle. Already gone counts as deleted.
    async fn delete(&self, handle: &str) -> Result<()>;
}

/// Every adapter, behind one type: match and forward.
pub enum AnyStore {
    GoogleDrive(gdrive::Store),
    Dropbox(dropbox::Store),
}

/// The store for `auth`'s account, picked by its provider.
pub fn store_for(auth: Auth) -> AnyStore {
    match auth.provider() {
        Provider::GoogleDrive => AnyStore::GoogleDrive(gdrive::Store::new(auth)),
        Provider::Dropbox => AnyStore::Dropbox(dropbox::Store::new(auth)),
    }
}

impl AnyStore {
    /// The in-memory tokens a [`Auth::Pending`] store was built with, as its
    /// refreshes left them — for the caller to keep, whether or not the work it
    /// did succeeded. `None` for a sealed store, whose refreshes went to the
    /// token file.
    pub fn into_tokens(self) -> Option<Tokens> {
        match self {
            AnyStore::GoogleDrive(store) => store.into_auth().into_tokens(),
            AnyStore::Dropbox(store) => store.into_auth().into_tokens(),
        }
    }
}

impl VaultStore for AnyStore {
    fn provider(&self) -> Provider {
        match self {
            AnyStore::GoogleDrive(store) => store.provider(),
            AnyStore::Dropbox(store) => store.provider(),
        }
    }

    async fn list_vaults(&self) -> Result<Vec<RemoteEntry>> {
        match self {
            AnyStore::GoogleDrive(store) => store.list_vaults().await,
            AnyStore::Dropbox(store) => store.list_vaults().await,
        }
    }

    async fn stat(&self, name: &str) -> Result<Option<RemoteEntry>> {
        match self {
            AnyStore::GoogleDrive(store) => store.stat(name).await,
            AnyStore::Dropbox(store) => store.stat(name).await,
        }
    }

    async fn read(&self, handle: &str, max_bytes: usize) -> Result<Vec<u8>> {
        match self {
            AnyStore::GoogleDrive(store) => store.read(handle, max_bytes).await,
            AnyStore::Dropbox(store) => store.read(handle, max_bytes).await,
        }
    }

    async fn create(&self, name: &str, bytes: &[u8]) -> Result<Option<String>> {
        match self {
            AnyStore::GoogleDrive(store) => store.create(name, bytes).await,
            AnyStore::Dropbox(store) => store.create(name, bytes).await,
        }
    }

    async fn update(&self, handle: &str, bytes: &[u8]) -> Result<Option<String>> {
        match self {
            AnyStore::GoogleDrive(store) => store.update(handle, bytes).await,
            AnyStore::Dropbox(store) => store.update(handle, bytes).await,
        }
    }

    async fn delete(&self, handle: &str) -> Result<()> {
        match self {
            AnyStore::GoogleDrive(store) => store.delete(handle).await,
            AnyStore::Dropbox(store) => store.delete(handle).await,
        }
    }
}

/// [`Remote`] over one vault's pack, `Vaults/<vault-id>.rowel`, in whichever
/// account `store` reaches.
///
/// The pack is re-`stat`ed on every call: `head_revision` exists precisely to
/// observe a change another device made, so it must never answer from a cache.
/// The adapter's async calls are driven with `block_on` here, which is safe
/// because a run always executes on its own thread (see `commands::sync`) —
/// never on a command thread, and never on a runtime worker.
pub struct PackRemote<S> {
    store: S,
    /// Settled by `resolve_vault_id` before the run starts, so every call
    /// below knows the file name it is after without asking again.
    vault_id: String,
}

impl<S: VaultStore> PackRemote<S> {
    pub fn new(store: S, vault_id: String) -> Self {
        Self { store, vault_id }
    }

    async fn locate(&self) -> Result<Option<RemoteEntry>> {
        self.store
            .stat(&layout::vault_file_name(&self.vault_id))
            .await
    }

    // Is there a marker where this vault's pack would be? Asked only when the
    // pack is missing, so an ordinary run never pays for it.
    async fn marker(&self) -> Result<bool> {
        let name = layout::deleted_marker_name(&self.vault_id);
        Ok(self.store.stat(&name).await?.is_some())
    }
}

/// What a run makes of finding no pack for its vault.
///
/// With no marker beside where the pack would be, this is simply a vault whose
/// first push has not happened yet, and the engine treats the account as empty.
/// With one, another device deleted this vault everywhere
/// ([`super::delete_pack`]) — and "the account is empty" would have this run
/// upload the pack again and undo that delete on every device, so the run
/// stops instead.
fn pack_missing(marker: bool) -> Result<Option<RemoteFile>> {
    if marker {
        return Err(Error::VaultDeletedRemotely);
    }
    Ok(None)
}

/// Whether a push may *create* this vault's pack, given whether a deletion
/// marker sits where it would go.
///
/// A run that pulled before a [`super::delete_pack`] and pushes after it would
/// otherwise put the pack back — this run read an account that still held the
/// vault, so nothing earlier in it has any reason to stop. The marker is the
/// one thing that outlives the pack, so the create path asks for it.
///
/// Only the create path. Finding the pack there means the delete has not
/// reached it (or never will), and an update is the ordinary push every run
/// makes — it pays no extra listing for this.
fn may_create_pack(marker: bool) -> Result<()> {
    if marker {
        return Err(Error::VaultDeletedRemotely);
    }
    Ok(())
}

impl<S: VaultStore> Remote for PackRemote<S> {
    fn fetch(&self) -> Result<Option<RemoteFile>> {
        block_on(async {
            let Some(entry) = self.locate().await? else {
                return pack_missing(self.marker().await?);
            };
            let bytes = self.store.read(&entry.handle, pack::MAX_PACK_BYTES).await?;
            Ok(Some(RemoteFile {
                bytes,
                revision: entry.revision.unwrap_or_default(),
            }))
        })
    }

    fn head_revision(&self) -> Result<Option<String>> {
        block_on(async {
            let Some(entry) = self.locate().await? else {
                return Ok(None);
            };
            // Read from the `stat`, exactly like `fetch` — the pre-flight
            // compares this against `fetch`'s value, so the two must degrade
            // identically. A `files.get` fallback here once made a missing
            // listing field compare `Some(real)` against `fetch`'s `Some("")`,
            // which would burn every retry and fail the run.
            Ok(Some(entry.revision.unwrap_or_default()))
        })
    }

    fn upload(&self, bytes: &[u8]) -> Result<String> {
        block_on(async {
            let revision = match self.locate().await? {
                Some(entry) => self.store.update(&entry.handle, bytes).await?,
                None => {
                    may_create_pack(self.marker().await?)?;
                    let name = layout::vault_file_name(&self.vault_id);
                    self.store.create(&name, bytes).await?
                }
            };
            Ok(revision.unwrap_or_default())
        })
    }
}

/// An in-memory `Vaults/` folder: a name-keyed map whose handles and revisions
/// are counters. What `PackRemote` and the delete are checked against here, and
/// what a later adapter's contract tests can run beside its own.
#[cfg(test)]
pub(crate) mod fake {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::{RemoteEntry, VaultStore};
    use crate::error::{Error, Result};
    use crate::sync::Provider;

    struct Object {
        handle: String,
        bytes: Vec<u8>,
        revision: u64,
    }

    #[derive(Default)]
    pub(crate) struct FakeStore {
        objects: Mutex<BTreeMap<String, Object>>,
        next: Mutex<u64>,
        /// Every `max_bytes` a `read` was asked with, in order.
        pub(crate) read_caps: Mutex<Vec<usize>>,
        /// Every write and delete, in order, as "create <name>", "update
        /// <name>" or "delete <name>" — for the tests that pin an ordering.
        pub(crate) log: Mutex<Vec<String>>,
    }

    impl FakeStore {
        /// Put `name` there directly, as another device's write would.
        pub(crate) fn put(&self, name: &str, bytes: &[u8]) {
            let id = self.tick();
            self.objects.lock().unwrap().insert(
                name.to_string(),
                Object {
                    handle: format!("h{id}"),
                    bytes: bytes.to_vec(),
                    revision: id,
                },
            );
        }

        pub(crate) fn has(&self, name: &str) -> bool {
            self.objects.lock().unwrap().contains_key(name)
        }

        pub(crate) fn bytes(&self, name: &str) -> Option<Vec<u8>> {
            self.objects
                .lock()
                .unwrap()
                .get(name)
                .map(|o| o.bytes.clone())
        }

        fn tick(&self) -> u64 {
            let mut next = self.next.lock().unwrap();
            *next += 1;
            *next
        }

        fn name_of(&self, handle: &str) -> Option<String> {
            self.objects
                .lock()
                .unwrap()
                .iter()
                .find(|(_, o)| o.handle == handle)
                .map(|(name, _)| name.clone())
        }

        fn entry(name: &str, object: &Object) -> RemoteEntry {
            RemoteEntry {
                handle: object.handle.clone(),
                name: name.to_string(),
                size: Some(object.bytes.len() as u64),
                modified: String::new(),
                revision: Some(object.revision.to_string()),
            }
        }
    }

    impl VaultStore for FakeStore {
        fn provider(&self) -> Provider {
            Provider::GoogleDrive
        }

        async fn list_vaults(&self) -> Result<Vec<RemoteEntry>> {
            Ok(self
                .objects
                .lock()
                .unwrap()
                .iter()
                .map(|(name, o)| Self::entry(name, o))
                .collect())
        }

        async fn stat(&self, name: &str) -> Result<Option<RemoteEntry>> {
            Ok(self
                .objects
                .lock()
                .unwrap()
                .get(name)
                .map(|o| Self::entry(name, o)))
        }

        async fn read(&self, handle: &str, max_bytes: usize) -> Result<Vec<u8>> {
            self.read_caps.lock().unwrap().push(max_bytes);
            let name = self.name_of(handle).ok_or(Error::NotFound)?;
            let bytes = self.bytes(&name).ok_or(Error::NotFound)?;
            if bytes.len() > max_bytes {
                return Err(Error::FileTooLarge);
            }
            Ok(bytes)
        }

        // Strict where a real provider might not be: creating over an existing
        // name is a bug in the caller, and the fake says so.
        async fn create(&self, name: &str, bytes: &[u8]) -> Result<Option<String>> {
            if self.has(name) {
                return Err(Error::Other(format!("{name} already exists")));
            }
            self.put(name, bytes);
            self.log.lock().unwrap().push(format!("create {name}"));
            Ok(self
                .objects
                .lock()
                .unwrap()
                .get(name)
                .map(|o| o.revision.to_string()))
        }

        async fn update(&self, handle: &str, bytes: &[u8]) -> Result<Option<String>> {
            let name = self.name_of(handle).ok_or(Error::NotFound)?;
            let revision = self.tick();
            let mut objects = self.objects.lock().unwrap();
            let object = objects.get_mut(&name).ok_or(Error::NotFound)?;
            object.bytes = bytes.to_vec();
            object.revision = revision;
            self.log.lock().unwrap().push(format!("update {name}"));
            Ok(Some(revision.to_string()))
        }

        async fn delete(&self, handle: &str) -> Result<()> {
            if let Some(name) = self.name_of(handle) {
                self.objects.lock().unwrap().remove(&name);
                self.log.lock().unwrap().push(format!("delete {name}"));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeStore;
    use super::*;

    const ID: &str = "a1b2c3";

    fn remote() -> PackRemote<FakeStore> {
        PackRemote::new(FakeStore::default(), ID.into())
    }

    fn pack_name() -> String {
        layout::vault_file_name(ID)
    }

    fn marker_name() -> String {
        layout::deleted_marker_name(ID)
    }

    // No pack is the first push not having happened yet — unless the marker
    // says the vault was deleted from the account, in which case pushing is
    // exactly what must not happen.
    #[test]
    fn a_missing_pack_is_a_delete_only_when_the_marker_is_there() {
        assert!(matches!(pack_missing(false), Ok(None)));
        assert!(matches!(
            pack_missing(true),
            Err(Error::VaultDeletedRemotely)
        ));
    }

    // The other half of the same marker, on the way out: a run that pulled
    // before a "delete everywhere" and pushes after it finds no pack to update,
    // and creating one there would undo the delete on every device. The run
    // that simply has not pushed yet still may.
    #[test]
    fn a_push_may_not_create_a_pack_the_marker_says_was_deleted() {
        assert!(may_create_pack(false).is_ok());
        assert!(matches!(
            may_create_pack(true),
            Err(Error::VaultDeletedRemotely)
        ));
    }

    // An account with nothing for this vault is an account to push to.
    #[test]
    fn a_missing_pack_without_a_marker_reads_as_nothing_pushed_yet() {
        let remote = remote();
        assert!(matches!(remote.fetch(), Ok(None)));
        assert_eq!(remote.head_revision().unwrap(), None);
    }

    // The same absence with a marker beside it is another device's delete.
    #[test]
    fn a_missing_pack_with_a_marker_is_a_vault_deleted_everywhere() {
        let remote = remote();
        remote.store.put(&marker_name(), &[]);
        assert!(matches!(remote.fetch(), Err(Error::VaultDeletedRemotely)));
    }

    // A run that pulled before the delete must not put the pack back after it.
    #[test]
    fn a_push_does_not_create_a_pack_where_a_marker_is() {
        let remote = remote();
        remote.store.put(&marker_name(), &[]);
        assert!(matches!(
            remote.upload(b"pack"),
            Err(Error::VaultDeletedRemotely)
        ));
        assert!(!remote.store.has(&pack_name()));
    }

    // The marker only guards the create: a pack still there is updated as any
    // push updates it — the delete has not reached it, and the half-state is the
    // harmless one.
    #[test]
    fn a_push_updates_a_pack_that_is_still_there_whatever_the_marker_says() {
        let remote = remote();
        remote.store.put(&pack_name(), b"old");
        remote.store.put(&marker_name(), &[]);
        remote.upload(b"new").unwrap();
        assert_eq!(remote.store.bytes(&pack_name()).unwrap(), b"new");

        // And a fetch reads it rather than consulting the marker.
        assert_eq!(remote.fetch().unwrap().unwrap().bytes, b"new");
    }

    // The first push creates the pack; every later one updates it.
    #[test]
    fn the_first_push_creates_and_the_next_updates() {
        let remote = remote();
        remote.upload(b"one").unwrap();
        remote.upload(b"two").unwrap();
        assert_eq!(
            *remote.store.log.lock().unwrap(),
            [
                format!("create {}", pack_name()),
                format!("update {}", pack_name())
            ]
        );
    }

    // The revision a push returns, the one a fetch reports and the head all
    // come from the provider as given — and agree, which is what the engine's
    // pre-flight compares.
    #[test]
    fn revisions_pass_through_from_the_store() {
        let remote = remote();
        let pushed = remote.upload(b"pack").unwrap();
        let head = remote.head_revision().unwrap().unwrap();
        let fetched = remote.fetch().unwrap().unwrap().revision;
        assert_eq!(pushed, head);
        assert_eq!(fetched, head);

        // Another device's write moves the head.
        remote.store.put(&pack_name(), b"theirs");
        assert_ne!(remote.head_revision().unwrap().unwrap(), pushed);
    }

    // Every pull is capped at the pack limit before a byte is buffered.
    #[test]
    fn a_fetch_reads_under_the_pack_size_cap() {
        let remote = remote();
        remote.store.put(&pack_name(), b"pack");
        remote.fetch().unwrap();
        assert_eq!(
            *remote.store.read_caps.lock().unwrap(),
            [pack::MAX_PACK_BYTES]
        );
    }
}
