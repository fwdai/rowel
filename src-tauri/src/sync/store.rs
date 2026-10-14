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

use std::future::Future;

use tauri::async_runtime::block_on;

use super::account::{Auth, Tokens};
use super::engine::{Remote, RemoteFile};
use super::providers::{dropbox, gdrive, icloud};
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
/// "`Vaults/`" here and below is the packs folder, whichever of
/// [`layout::PACKS_FOLDERS`] the account has: an adapter resolves it through
/// [`find_packs_folder`] (or [`ensure_packs_folder`] when creating), never by
/// looking for one name.
///
/// `create` and `update` are two calls rather than one create-or-overwrite by
/// name on purpose. A push decides between them from a `stat` it has just made,
/// and only the create path asks whether the vault was deleted everywhere
/// ([`may_create_pack`]); an overwrite by name would let a delete landing
/// between that `stat` and the write re-create the pack without the question
/// ever being asked. An update of the entry that `stat` returned fails instead.
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
    /// Overwrite `entry`, as the core's `stat` just located it but carrying
    /// the revision the run pulled. Returns its new revision.
    ///
    /// An object that is gone is an error, never a create. An adapter whose
    /// API can make the write conditional on `entry.revision` should (Dropbox
    /// does): then an object another device rewrote since the pull is an
    /// error too, and the run fails rather than overwrite a write it never
    /// merged. One that cannot ignores the revision (Drive).
    async fn update(&self, entry: &RemoteEntry, bytes: &[u8]) -> Result<Option<String>>;
    /// Delete by handle. Already gone counts as deleted.
    async fn delete(&self, handle: &str) -> Result<()>;
}

/// Every adapter, behind one type: match and forward.
pub enum AnyStore {
    GoogleDrive(gdrive::Store),
    Dropbox(dropbox::Store),
    ICloud(icloud::Store),
}

/// The store for `auth`'s account, picked by its provider.
pub fn store_for(auth: Auth) -> AnyStore {
    match auth.provider() {
        Provider::GoogleDrive => AnyStore::GoogleDrive(gdrive::Store::new(auth)),
        Provider::Dropbox => AnyStore::Dropbox(dropbox::Store::new(auth)),
        Provider::ICloud => AnyStore::ICloud(icloud::Store::new(auth)),
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
            AnyStore::ICloud(store) => store.into_auth().into_tokens(),
        }
    }
}

impl VaultStore for AnyStore {
    fn provider(&self) -> Provider {
        match self {
            AnyStore::GoogleDrive(store) => store.provider(),
            AnyStore::Dropbox(store) => store.provider(),
            AnyStore::ICloud(store) => store.provider(),
        }
    }

    async fn list_vaults(&self) -> Result<Vec<RemoteEntry>> {
        match self {
            AnyStore::GoogleDrive(store) => store.list_vaults().await,
            AnyStore::Dropbox(store) => store.list_vaults().await,
            AnyStore::ICloud(store) => store.list_vaults().await,
        }
    }

    async fn stat(&self, name: &str) -> Result<Option<RemoteEntry>> {
        match self {
            AnyStore::GoogleDrive(store) => store.stat(name).await,
            AnyStore::Dropbox(store) => store.stat(name).await,
            AnyStore::ICloud(store) => store.stat(name).await,
        }
    }

    async fn read(&self, handle: &str, max_bytes: usize) -> Result<Vec<u8>> {
        match self {
            AnyStore::GoogleDrive(store) => store.read(handle, max_bytes).await,
            AnyStore::Dropbox(store) => store.read(handle, max_bytes).await,
            AnyStore::ICloud(store) => store.read(handle, max_bytes).await,
        }
    }

    async fn create(&self, name: &str, bytes: &[u8]) -> Result<Option<String>> {
        match self {
            AnyStore::GoogleDrive(store) => store.create(name, bytes).await,
            AnyStore::Dropbox(store) => store.create(name, bytes).await,
            AnyStore::ICloud(store) => store.create(name, bytes).await,
        }
    }

    async fn update(&self, entry: &RemoteEntry, bytes: &[u8]) -> Result<Option<String>> {
        match self {
            AnyStore::GoogleDrive(store) => store.update(entry, bytes).await,
            AnyStore::Dropbox(store) => store.update(entry, bytes).await,
            AnyStore::ICloud(store) => store.update(entry, bytes).await,
        }
    }

    async fn delete(&self, handle: &str) -> Result<()> {
        match self {
            AnyStore::GoogleDrive(store) => store.delete(handle).await,
            AnyStore::Dropbox(store) => store.delete(handle).await,
            AnyStore::ICloud(store) => store.delete(handle).await,
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

    // Is there a marker where this vault's pack would be? The account's last
    // word on this vault, so every fetch asks — one `stat` per run — and so
    // does the create path, before and after its write.
    async fn marker(&self) -> Result<bool> {
        let name = layout::deleted_marker_name(&self.vault_id);
        Ok(self.store.stat(&name).await?.is_some())
    }

    /// Every pack under this vault's name, gone. By the listing rather than a
    /// `stat`: Drive may hold two files of one name (two first pushes that
    /// raced each other as well as the delete), and its `stat` reports only
    /// the oldest — two cleanups going by `stat` would both delete that one
    /// and leave the newer. A listing is complete (invariant 1), so this takes
    /// them all. Already gone counts as deleted.
    async fn remove_packs(&self) -> Result<()> {
        let name = layout::vault_file_name(&self.vault_id);
        for entry in self.store.list_vaults().await? {
            if entry.name == name {
                self.store.delete(&entry.handle).await?;
            }
        }
        Ok(())
    }

    /// A pack found beside the marker is a push that landed in a delete's
    /// window and was never taken back out: the delete's own `stat` missed
    /// it, or the creator's cleanup failed. Whichever run sees the two
    /// together finishes the job, so the half-state cannot outlive the next
    /// run on any device. Best effort: the verdict is the marker's either
    /// way, and a pack this run could not remove is the next run's to try.
    async fn finish_delete(&self) {
        if let Err(e) = self.remove_packs().await {
            log::warn!(
                "the vault was deleted from the account, but the pack left beside its marker \
                 could not be removed yet: {e}"
            );
        }
    }

    /// The first push: create the pack, unless the vault was deleted
    /// everywhere — checked before the write, and again after it.
    ///
    /// The check before ([`may_create_pack`]) is the cheap one, and it settles
    /// the common case without an upload. It cannot settle the race on its
    /// own: this is two calls to the account, and so is
    /// [`super::delete_pack`] (marker, then pack). A delete whose marker lands
    /// after this check and whose `stat` of the pack runs before this create
    /// finds nothing to remove, and the account is left holding both — which
    /// `fetch` reads as the pack, on every device, undoing the delete.
    ///
    /// The check after closes that window. Either the marker was written
    /// before this re-check, and this run sees it and removes the pack it just
    /// wrote; or it was written after, which puts the delete's own `stat` of
    /// the pack after the create too, so the delete finds the pack and removes
    /// it. One side or the other always takes the pack back out.
    ///
    /// A cleanup that fails is still an error — the run fails and is retried
    /// — but a different one, so the log says the pack may have been left
    /// behind. It is not left for good: the next run, here or on any other
    /// device, finds the pack beside the marker and removes it
    /// ([`PackRemote::finish_delete`]).
    async fn create_pack(&self, bytes: &[u8]) -> Result<Option<String>> {
        may_create_pack(self.marker().await?)?;
        let name = layout::vault_file_name(&self.vault_id);
        let revision = self.store.create(&name, bytes).await?;
        if !self.marker().await? {
            return Ok(revision);
        }
        self.remove_packs().await.map_err(|e| {
            Error::Other(format!(
                "this vault was deleted from the account on another device while this one \
                 was pushing it, and removing the pack this push had just written failed: \
                 {e}; sync will retry"
            ))
        })?;
        Err(Error::VaultDeletedRemotely)
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
/// Only the create path. The run's fetch already stopped on a marker beside
/// a pack, so an update is a push over a pack that was live when the run
/// began; a delete landing mid-run removes it after the update, or makes the
/// update fail (the handle is gone), and either way nothing is re-created.
///
/// This is the check *before* the create. It is not enough by itself, since a
/// marker can land between it and the write; `PackRemote::create_pack` asks
/// again afterwards and takes its own pack back out if one did.
fn may_create_pack(marker: bool) -> Result<()> {
    if marker {
        return Err(Error::VaultDeletedRemotely);
    }
    Ok(())
}

impl<S: VaultStore> Remote for PackRemote<S> {
    // The marker is asked for whether or not the pack is there. Beside a
    // pack it is still the account's verdict — the pack is a push that landed
    // in the delete's window and was never removed — and a run that read the
    // pack instead would carry the vault on, on every device, for as long as
    // the two sat together. So the run finishes the delete and stops.
    fn fetch(&self) -> Result<Option<RemoteFile>> {
        block_on(async {
            let located = self.locate().await?;
            let marker = self.marker().await?;
            let Some(entry) = located else {
                return pack_missing(marker);
            };
            if marker {
                self.finish_delete().await;
                return Err(Error::VaultDeletedRemotely);
            }
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

    // The `stat` here only finds the handle and decides update-or-create. The
    // revision a write is conditional on is the engine's `base_revision`, the
    // one the run *pulled*, never this `stat`'s: a push landing between the
    // engine's head check and this call would otherwise be the revision
    // compared against, and be overwritten without ever being merged.
    //
    // The create side is the one that can collide with a "delete everywhere"
    // in flight, and it re-checks the marker after writing (`create_pack`).
    fn upload(&self, bytes: &[u8], base_revision: Option<&str>) -> Result<String> {
        block_on(async {
            let revision = match (self.locate().await?, base_revision) {
                // Over the pulled revision, so an adapter that can make the
                // write conditional on it (Dropbox) fails on a mismatch. Drive
                // ignores it. An empty base is the engine's "unknown", which an
                // entry spells `None`.
                (Some(entry), Some(base)) => {
                    let entry = RemoteEntry {
                        revision: Some(base.to_string()).filter(|rev| !rev.is_empty()),
                        ..entry
                    };
                    self.store.update(&entry, bytes).await?
                }
                // The run pulled nothing, yet a pack is there now: another
                // device created it since. Writing over it would drop that
                // device's first push, so the run fails instead and the next
                // one pulls and merges it. This holds for every provider,
                // Drive included, since no revision is needed to see it.
                (Some(_), None) => {
                    return Err(Error::Other(
                        "another device created the vault file while this one was syncing; \
                         sync will retry"
                            .into(),
                    ));
                }
                (None, _) => self.create_pack(bytes).await?,
            };
            Ok(revision.unwrap_or_default())
        })
    }
}

// --- which folder is `Vaults/` ----------------------------------------------

/// The one of [`layout::PACKS_FOLDERS`] an account has, out of `found` (each
/// packs-folder name the adapter found under the root, with its handle): its
/// handle, or `None` when it has neither.
///
/// `Vaults/` today, `Lockboxes/` once a later release has renamed it. Reading
/// both is what lets a client of this release keep syncing across that rename.
/// Finding *both* is refused rather than resolved: it means two generations of
/// client have filed packs apart (one of them, older than the rename, recreated
/// `Vaults/` beside `Lockboxes/`), and picking either side would hide the
/// other's packs and deletion markers and let the two histories drift on in
/// silence. Stopping is what gets the user to update the straggler and fold the
/// folders together.
///
/// Every adapter's packs-folder lookup ends here, so every device and every
/// provider settles the same way; `provider` only names the account in the
/// error.
pub(crate) fn pick_packs_folder(
    provider: Provider,
    found: Vec<(&'static str, String)>,
) -> Result<Option<String>> {
    if found.len() > 1 {
        return Err(Error::Other(both_packs_folders_error(provider)));
    }
    Ok(found.into_iter().next().map(|(_, handle)| handle))
}

// Surfaced verbatim by the sync indicator, so it has to read as a sentence.
// The folders are named as the user finds them in the account
// (`Provider::packs_root_label`); the app to update is always Rowel.
fn both_packs_folders_error(provider: Provider) -> String {
    let [first, second] = layout::PACKS_FOLDERS;
    let root = provider.packs_root_label();
    format!(
        "this {} holds both {root}/{first} and {root}/{second}; update {app} on every device, \
         then move the packs into one of them and delete the other",
        provider.display_name(),
        app = layout::ROOT_FOLDER,
    )
}

/// Look each of [`layout::PACKS_FOLDERS`] up with `lookup` (the adapter's
/// "folder of this name under the root" call, injected so the rule is testable
/// without one) and settle on one by [`pick_packs_folder`].
pub(crate) async fn find_packs_folder<F, Fut>(
    provider: Provider,
    mut lookup: F,
) -> Result<Option<String>>
where
    F: FnMut(&'static str) -> Fut,
    Fut: Future<Output = Result<Option<String>>>,
{
    let mut found = Vec::new();
    for name in layout::PACKS_FOLDERS {
        if let Some(handle) = lookup(name).await? {
            found.push((name, handle));
        }
    }
    pick_packs_folder(provider, found)
}

/// [`find_packs_folder`], creating [`layout::VAULTS_FOLDER`] with `create` when
/// the account has none. Always that name, never [`layout::LOCKBOXES_FOLDER`]:
/// a fresh account filed under the new name forks the first time a client from
/// before the rename syncs into it (it finds no `Vaults/`, makes one, uploads
/// there). Creating the new name is the cutover release's move, once no
/// supported client is older than the rename.
pub(crate) async fn ensure_packs_folder<F, Fut, C, CFut>(
    provider: Provider,
    lookup: F,
    create: C,
) -> Result<String>
where
    F: FnMut(&'static str) -> Fut,
    Fut: Future<Output = Result<Option<String>>>,
    C: FnOnce(&'static str) -> CFut,
    CFut: Future<Output = Result<String>>,
{
    match find_packs_folder(provider, lookup).await? {
        Some(handle) => Ok(handle),
        None => create(layout::VAULTS_FOLDER).await,
    }
}

/// An in-memory `Vaults/` folder: objects in the order they landed, whose
/// handles and revisions are counters. What `PackRemote` and the delete are
/// checked against here, and what a later adapter's contract tests can run
/// beside its own. Drive-shaped where it matters: two objects may share a
/// name ([`FakeStore::put_sibling`]), and `stat` then reports the oldest.
#[cfg(test)]
pub(crate) mod fake {
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
        objects: Mutex<Vec<(String, Object)>>,
        next: Mutex<u64>,
        /// Every `max_bytes` a `read` was asked with, in order.
        pub(crate) read_caps: Mutex<Vec<usize>>,
        /// Every write and delete, in order, as "create <name>", "update
        /// <name>" or "delete <name>" — for the tests that pin an ordering.
        pub(crate) log: Mutex<Vec<String>>,
        /// The revision every `update` was asked to write over, in order.
        pub(crate) update_revisions: Mutex<Vec<Option<String>>>,
        /// Objects to land, empty, just before the next `create` goes through
        /// — other devices' writes arriving between a caller's `stat` and its
        /// create. Set by [`Self::interlope_at_create`]; a name the caller is
        /// creating lands as a same-name sibling, as a racing first push on
        /// Drive would.
        create_interlopers: Mutex<Vec<String>>,
        /// Make the next `delete` fail, once: the network going away between
        /// a cleanup's listing and its remove.
        fail_next_delete: Mutex<bool>,
    }

    impl FakeStore {
        /// Land `name` on the next `create` — after the caller's checks,
        /// before its write. Call again to land several.
        pub(crate) fn interlope_at_create(&self, name: &str) {
            self.create_interlopers
                .lock()
                .unwrap()
                .push(name.to_string());
        }

        pub(crate) fn fail_next_delete(&self) {
            *self.fail_next_delete.lock().unwrap() = true;
        }

        /// Put `name` there directly, as another device's write would: a new
        /// revision, under the handle it already had if it was there.
        pub(crate) fn put(&self, name: &str, bytes: &[u8]) {
            let id = self.tick();
            let mut objects = self.objects.lock().unwrap();
            match objects.iter_mut().find(|(n, _)| n == name) {
                Some((_, object)) => {
                    object.bytes = bytes.to_vec();
                    object.revision = id;
                }
                None => objects.push((
                    name.to_string(),
                    Object {
                        handle: format!("h{id}"),
                        bytes: bytes.to_vec(),
                        revision: id,
                    },
                )),
            }
        }

        /// A second object under a name already taken, as Drive ends up with
        /// when two first pushes race: its own handle, and younger than the
        /// one `stat` reports.
        pub(crate) fn put_sibling(&self, name: &str, bytes: &[u8]) {
            let id = self.tick();
            self.objects.lock().unwrap().push((
                name.to_string(),
                Object {
                    handle: format!("h{id}"),
                    bytes: bytes.to_vec(),
                    revision: id,
                },
            ));
        }

        pub(crate) fn has(&self, name: &str) -> bool {
            self.count(name) > 0
        }

        /// How many objects carry `name`: one, ordinarily.
        pub(crate) fn count(&self, name: &str) -> usize {
            self.objects
                .lock()
                .unwrap()
                .iter()
                .filter(|(n, _)| n == name)
                .count()
        }

        pub(crate) fn bytes(&self, name: &str) -> Option<Vec<u8>> {
            self.objects
                .lock()
                .unwrap()
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, o)| o.bytes.clone())
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

        // Oldest wins, as on Drive: the first object that landed under the
        // name is the one reported.
        async fn stat(&self, name: &str) -> Result<Option<RemoteEntry>> {
            Ok(self
                .objects
                .lock()
                .unwrap()
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, o)| Self::entry(name, o)))
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
            // Not logged: they are other devices' writes, not this caller's,
            // so the order the log pins stays the caller's own. A write under
            // the name being created is another device's first push, which
            // Drive keeps as a sibling.
            let interlopers = std::mem::take(&mut *self.create_interlopers.lock().unwrap());
            for other in interlopers {
                if other == name {
                    self.put_sibling(&other, &[]);
                } else {
                    self.put(&other, &[]);
                }
            }
            self.put_sibling(name, bytes);
            self.log.lock().unwrap().push(format!("create {name}"));
            Ok(self
                .objects
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|(n, _)| n == name)
                .map(|(_, o)| o.revision.to_string()))
        }

        // A compare-and-swap, as Dropbox's is: an object deleted since the
        // `stat` that found it fails (and is not re-created), and so does one
        // rewritten since, whenever the entry carries the revision it had.
        async fn update(&self, entry: &RemoteEntry, bytes: &[u8]) -> Result<Option<String>> {
            self.update_revisions
                .lock()
                .unwrap()
                .push(entry.revision.clone());
            let name = self.name_of(&entry.handle).ok_or(Error::NotFound)?;
            let revision = self.tick();
            let mut objects = self.objects.lock().unwrap();
            let (_, object) = objects
                .iter_mut()
                .find(|(_, o)| o.handle == entry.handle)
                .ok_or(Error::NotFound)?;
            if let Some(expected) = &entry.revision {
                if *expected != object.revision.to_string() {
                    return Err(Error::Other(format!("{name} changed since it was read")));
                }
            }
            object.bytes = bytes.to_vec();
            object.revision = revision;
            self.log.lock().unwrap().push(format!("update {name}"));
            Ok(Some(revision.to_string()))
        }

        async fn delete(&self, handle: &str) -> Result<()> {
            if std::mem::take(&mut *self.fail_next_delete.lock().unwrap()) {
                return Err(Error::Other("the network went away".into()));
            }
            if let Some(name) = self.name_of(handle) {
                self.objects
                    .lock()
                    .unwrap()
                    .retain(|(_, o)| o.handle != handle);
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
            remote.upload(b"pack", None),
            Err(Error::VaultDeletedRemotely)
        ));
        assert!(!remote.store.has(&pack_name()));
    }

    // A "delete everywhere" whose marker lands after the create path's first
    // look and whose own `stat` of the pack ran before the create: the delete
    // found nothing to remove, so the creator must. It re-checks the marker
    // after writing, takes its own pack back out, and fails as the first look
    // would have — leaving the account as the delete meant it.
    #[test]
    fn a_marker_landing_between_the_check_and_the_create_takes_the_new_pack_back_out() {
        let remote = remote();
        remote.store.interlope_at_create(&marker_name());

        assert!(matches!(
            remote.upload(b"pack", None),
            Err(Error::VaultDeletedRemotely)
        ));
        assert!(!remote.store.has(&pack_name()));
        assert!(remote.store.has(&marker_name()));
        assert_eq!(
            *remote.store.log.lock().unwrap(),
            [
                format!("create {}", pack_name()),
                format!("delete {}", pack_name())
            ]
        );
    }

    // With no delete in flight the re-check is one more `stat` and nothing
    // else: the pack is created once and nothing is deleted.
    #[test]
    fn an_ordinary_first_push_creates_once_and_deletes_nothing() {
        let remote = remote();
        remote.upload(b"pack", None).unwrap();
        assert_eq!(remote.store.bytes(&pack_name()).unwrap(), b"pack");
        assert_eq!(
            *remote.store.log.lock().unwrap(),
            [format!("create {}", pack_name())]
        );
    }

    // A pack beside the marker is a push that landed in a delete's window and
    // was never taken back out. The marker is still the verdict: a fetch
    // finishes the delete — removes the pack, keeps the marker — and stops,
    // rather than carrying the deleted vault on.
    #[test]
    fn a_fetch_that_finds_a_pack_beside_the_marker_finishes_the_delete() {
        let remote = remote();
        remote.store.put(&pack_name(), b"left behind");
        remote.store.put(&marker_name(), &[]);
        assert!(matches!(remote.fetch(), Err(Error::VaultDeletedRemotely)));
        assert!(!remote.store.has(&pack_name()));
        assert!(remote.store.has(&marker_name()));
        assert_eq!(
            *remote.store.log.lock().unwrap(),
            [format!("delete {}", pack_name())]
        );
    }

    // The creator's cleanup can fail after its create went through, leaving
    // the pair behind with an error that names it. That is not the end of
    // it: the next run — this device's or any other's — reads the marker,
    // removes the pack and stops, so a retry never reports the deleted vault
    // as synced.
    #[test]
    fn a_failed_cleanup_is_finished_by_the_next_run() {
        let remote = remote();
        remote.store.interlope_at_create(&marker_name());
        remote.store.fail_next_delete();
        match remote.upload(b"pack", None) {
            Err(Error::Other(why)) => assert!(why.contains("removing the pack"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert!(remote.store.has(&pack_name()));
        assert!(remote.store.has(&marker_name()));

        assert!(matches!(remote.fetch(), Err(Error::VaultDeletedRemotely)));
        assert!(!remote.store.has(&pack_name()));
        assert!(remote.store.has(&marker_name()));
    }

    // Two devices' first pushes and a delete, all in one window: both passed
    // the pre-check, the delete's marker landed, both creates went through,
    // and Drive now holds two packs under one name. Each creator's cleanup
    // takes every one of them out — by the listing, since `stat` would show
    // both creators the same, oldest pack and leave the newer behind.
    #[test]
    fn a_cleanup_removes_every_same_name_pack_not_just_the_oldest() {
        let remote = remote();
        remote.store.interlope_at_create(&marker_name());
        remote.store.interlope_at_create(&pack_name());
        assert!(matches!(
            remote.upload(b"pack", None),
            Err(Error::VaultDeletedRemotely)
        ));
        assert_eq!(remote.store.count(&pack_name()), 0);
        assert!(remote.store.has(&marker_name()));
        assert_eq!(
            *remote.store.log.lock().unwrap(),
            [
                format!("create {}", pack_name()),
                format!("delete {}", pack_name()),
                format!("delete {}", pack_name()),
            ]
        );

        // The same from the fetch side, for a pair the creators left behind.
        let later = self::remote();
        later.store.put(&pack_name(), b"one");
        later.store.put_sibling(&pack_name(), b"two");
        later.store.put(&marker_name(), &[]);
        assert!(matches!(later.fetch(), Err(Error::VaultDeletedRemotely)));
        assert_eq!(later.store.count(&pack_name()), 0);
    }

    // The marker guards fetch and create, not update: a run whose fetch found
    // the pack live is pushing over a pack a delete has not reached, and that
    // delete's own `stat` removes it afterwards — or the update fails on a
    // handle that is gone. Nothing is re-created either way.
    #[test]
    fn a_push_updates_a_pack_that_is_still_there_whatever_the_marker_says() {
        let remote = remote();
        remote.store.put(&pack_name(), b"old");
        let base = remote.head_revision().unwrap();
        remote.store.put(&marker_name(), &[]);
        remote.upload(b"new", base.as_deref()).unwrap();
        assert_eq!(remote.store.bytes(&pack_name()).unwrap(), b"new");
    }

    // The first push creates the pack; every later one updates it.
    #[test]
    fn the_first_push_creates_and_the_next_updates() {
        let remote = remote();
        let first = remote.upload(b"one", None).unwrap();
        remote.upload(b"two", Some(&first)).unwrap();
        assert_eq!(
            *remote.store.log.lock().unwrap(),
            [
                format!("create {}", pack_name()),
                format!("update {}", pack_name())
            ]
        );
    }

    // A push over an existing pack hands the store the revision the run
    // pulled, so an adapter that can compare-and-swap on it does.
    #[test]
    fn a_push_updates_with_the_revision_the_run_pulled() {
        let remote = remote();
        remote.store.put(&pack_name(), b"old");
        let pulled = remote.fetch().unwrap().unwrap().revision;

        remote.upload(b"new", Some(&pulled)).unwrap();
        assert_eq!(
            *remote.store.update_revisions.lock().unwrap(),
            [Some(pulled)]
        );
    }

    // The engine's "unknown" revision is an empty string; the store is handed
    // `None`, which every adapter already reads as "nothing to compare".
    #[test]
    fn an_unknown_pulled_revision_reaches_the_store_as_none() {
        let remote = remote();
        remote.store.put(&pack_name(), b"old");
        remote.upload(b"new", Some("")).unwrap();
        assert_eq!(*remote.store.update_revisions.lock().unwrap(), [None]);
        assert_eq!(remote.store.bytes(&pack_name()).unwrap(), b"new");
    }

    // Another device's push landing after the engine's head check, before
    // this upload: the `stat` in `upload` sees the new revision, but the write
    // is conditional on the pulled one, so a compare-and-swap store (Dropbox,
    // the fake) refuses it and the other write stands.
    #[test]
    fn a_push_landing_after_the_head_check_fails_the_upload_rather_than_being_overwritten() {
        let remote = remote();
        remote.store.put(&pack_name(), b"old");
        let pulled = remote.fetch().unwrap().unwrap().revision;
        assert_eq!(remote.head_revision().unwrap().as_ref(), Some(&pulled));

        remote.store.put(&pack_name(), b"theirs");
        assert!(remote.upload(b"mine", Some(&pulled)).is_err());
        assert_eq!(remote.store.bytes(&pack_name()).unwrap(), b"theirs");
        assert!(remote.store.log.lock().unwrap().is_empty());
    }

    // The run pulled nothing, and by the time it pushes another device has
    // created the pack. Overwriting it would drop that device's first push, on
    // any provider, so the upload fails and the next run merges it.
    #[test]
    fn a_pack_that_appeared_after_an_empty_pull_fails_the_upload() {
        let remote = remote();
        assert!(remote.fetch().unwrap().is_none());

        remote.store.put(&pack_name(), b"theirs");
        match remote.upload(b"mine", None) {
            Err(Error::Other(msg)) => assert!(msg.contains("another device created")),
            other => panic!("expected the readable race error, got {other:?}"),
        }
        assert_eq!(remote.store.bytes(&pack_name()).unwrap(), b"theirs");
        assert!(remote.store.log.lock().unwrap().is_empty());
    }

    // The fake holds adapters to the contract: an update of an entry rewritten
    // since its `stat` fails and leaves the other write in place, and one of an
    // entry deleted since fails without putting it back.
    #[test]
    fn an_update_over_a_stale_or_deleted_entry_fails_rather_than_writes() {
        let store = FakeStore::default();
        store.put(&pack_name(), b"ours");
        let entry = block_on(store.stat(&pack_name())).unwrap().unwrap();

        store.put(&pack_name(), b"theirs");
        assert!(block_on(store.update(&entry, b"mine")).is_err());
        assert_eq!(store.bytes(&pack_name()).unwrap(), b"theirs");

        let entry = block_on(store.stat(&pack_name())).unwrap().unwrap();
        block_on(store.delete(&entry.handle)).unwrap();
        assert!(block_on(store.update(&entry, b"mine")).is_err());
        assert!(!store.has(&pack_name()));

        // Neither failure was logged as a write.
        assert_eq!(
            *store.log.lock().unwrap(),
            [format!("delete {}", pack_name())]
        );
    }

    // The revision a push returns, the one a fetch reports and the head all
    // come from the provider as given — and agree, which is what the engine's
    // pre-flight compares.
    #[test]
    fn revisions_pass_through_from_the_store() {
        let remote = remote();
        let pushed = remote.upload(b"pack", None).unwrap();
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

    // --- which folder is `Vaults/` ---

    const DRIVE: Provider = Provider::GoogleDrive;

    // What an adapter found under the root: the folder's own name doubles as
    // its handle.
    fn found(folders: &[&'static str]) -> Vec<(&'static str, String)> {
        folders
            .iter()
            .map(|&name| (name, name.to_string()))
            .collect()
    }

    // An account's folders under `Rowel/`, as an adapter's lookup would find
    // them.
    fn lookup_in(
        folders: &'static [&'static str],
    ) -> impl FnMut(&'static str) -> std::future::Ready<Result<Option<String>>> {
        move |name| std::future::ready(Ok(folders.contains(&name).then(|| name.to_string())))
    }

    // Every account today: syncing goes where it always has, and where a
    // client from before the rename on another device still looks.
    #[test]
    fn an_account_with_vaults_uses_it() {
        let picked = pick_packs_folder(DRIVE, found(&[layout::VAULTS_FOLDER]));
        assert_eq!(picked.unwrap().as_deref(), Some("Vaults"));
    }

    // After the cutover release has renamed the folder, this release keeps
    // reading it rather than taking the account for empty.
    #[test]
    fn an_account_renamed_by_the_cutover_uses_lockboxes() {
        let picked = pick_packs_folder(DRIVE, found(&[layout::LOCKBOXES_FOLDER]));
        assert_eq!(picked.unwrap().as_deref(), Some("Lockboxes"));
    }

    #[test]
    fn an_account_with_neither_has_no_packs_folder() {
        assert_eq!(pick_packs_folder(DRIVE, Vec::new()).unwrap(), None);
    }

    // Both means two generations of client have filed packs apart. Picking a
    // side would hide the other's packs and markers and let the histories
    // drift on; the run stops instead, whichever order the folders turn up in,
    // with a sentence naming both.
    #[test]
    fn with_both_folders_the_run_refuses_to_pick_a_side() {
        for both in [
            [layout::VAULTS_FOLDER, layout::LOCKBOXES_FOLDER],
            [layout::LOCKBOXES_FOLDER, layout::VAULTS_FOLDER],
        ] {
            let err = pick_packs_folder(DRIVE, found(&both)).unwrap_err();
            assert!(
                matches!(&err, Error::Other(message) if message.contains("Rowel/Vaults") && message.contains("Rowel/Lockboxes")),
                "{err:?}"
            );
        }
    }

    // The Drive copy, word for word as it shipped before the core existed.
    #[test]
    fn the_both_folders_error_reads_as_it_always_did_for_google() {
        assert_eq!(
            both_packs_folders_error(DRIVE),
            "this Google Drive holds both Rowel/Vaults and Rowel/Lockboxes; update Rowel on \
             every device, then move the packs into one of them and delete the other"
        );
    }

    // In Dropbox the folders sit in the app folder, which the user finds
    // under `Apps/`; the app to update is still Rowel.
    #[test]
    fn the_both_folders_error_names_dropboxs_app_folder() {
        assert_eq!(
            both_packs_folders_error(Provider::Dropbox),
            "this Dropbox holds both Apps/Rowel/Vaults and Apps/Rowel/Lockboxes; update Rowel \
             on every device, then move the packs into one of them and delete the other"
        );
    }

    // The lookup asks for every name, so both being there is always noticed.
    #[tokio::test]
    async fn the_lookup_settles_by_the_same_rule() {
        let only = find_packs_folder(DRIVE, lookup_in(&[layout::LOCKBOXES_FOLDER])).await;
        assert_eq!(only.unwrap().as_deref(), Some("Lockboxes"));
        assert_eq!(
            find_packs_folder(DRIVE, lookup_in(&[])).await.unwrap(),
            None
        );
        let both = &[layout::LOCKBOXES_FOLDER, layout::VAULTS_FOLDER];
        assert!(find_packs_folder(DRIVE, lookup_in(both)).await.is_err());
    }

    // A fresh account gets `Vaults/`, the one name every supported client
    // knows; an account that has a folder, under either name, keeps it and
    // nothing is created beside it.
    #[tokio::test]
    async fn only_an_account_with_no_folder_gets_one_created_and_it_is_vaults() {
        let created = std::cell::RefCell::new(Vec::new());
        let create = |name: &'static str| {
            created.borrow_mut().push(name);
            std::future::ready(Ok(format!("new-{name}")))
        };
        let id = ensure_packs_folder(DRIVE, lookup_in(&[]), create)
            .await
            .unwrap();
        assert_eq!(id, "new-Vaults");
        assert_eq!(*created.borrow(), ["Vaults"]);

        let existing: [&'static [&'static str]; 2] =
            [&[layout::VAULTS_FOLDER], &[layout::LOCKBOXES_FOLDER]];
        for folders in existing {
            let create = |name: &'static str| {
                created.borrow_mut().push(name);
                std::future::ready(Ok(format!("new-{name}")))
            };
            let id = ensure_packs_folder(DRIVE, lookup_in(folders), create)
                .await
                .unwrap();
            assert_eq!(id, folders[0]);
            assert_eq!(created.borrow().len(), 1);
        }
    }
}
