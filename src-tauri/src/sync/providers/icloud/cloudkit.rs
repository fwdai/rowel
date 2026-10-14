//! The CloudKit calls, and nothing else: every Objective-C object lives in this
//! file, and what leaves it is a plain value ([`super::Fields`], a `Failure`, a
//! change tag, the bytes).
//!
//! Every call hands CloudKit a block and awaits a [`Completion`] the block
//! resolves. The blocks run on CloudKit's own serial queue, so what they
//! capture is `Send` (`Retained<CKDatabase>` is; a `CKRecord` is not, and is
//! never captured — it is read, or changed and saved, inside the block it was
//! handed to).
//!
//! Each call is a synchronous `start_*` that builds the blocks, hands CloudKit
//! the work and returns only the completion, which the `async fn` then awaits.
//! Split that way on purpose: the futures reach Tauri commands, which must be
//! `Send`, and a block or record alive across the await would not be. CloudKit
//! retains what it was handed, so nothing is lost when the starter's locals go.
//!
//! Reads that must not download the pack (`stat`, the listing) go through the
//! operations that take `desiredKeys`; the rest use the database's convenience
//! calls, which save with the default `ifServerRecordUnchanged` policy — the
//! compare-and-swap the store's `create` and `update` are built on.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::AllocAnyThread;
use objc2_cloud_kit::{
    CKAccountStatus, CKAsset, CKContainer, CKDatabase, CKErrorDomain, CKFetchRecordsOperation,
    CKOperation, CKQuery, CKQueryCursor, CKQueryOperation, CKRecord, CKRecordID,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSNumber, NSPredicate, NSQualityOfService, NSString, NSURL,
};

use super::completion::Completion;
use super::{
    changed_since_pull, create_raced, entry, stale, within_cap, Failure, Fields, CONTAINER_ID,
    PACK_FIELD, RECORD_TYPE, SERVER_RECORD_CHANGED, SIZE_FIELD, UNKNOWN_ITEM,
};
use crate::error::{Error, Result};
use crate::sync::store::RemoteEntry;

// --- the gate ---

pub(super) fn supported() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(entitled)
}

/// iOS: the entitlements are in the committed project, and every build that
/// runs on a device is signed with them.
#[cfg(target_os = "ios")]
fn entitled() -> bool {
    true
}

/// macOS: ask the kernel what this very process was signed with. The
/// `com.apple.developer.icloud-services` entitlement lists the iCloud services
/// the signature grants; the one this adapter needs is `CloudKit`. A process
/// with none (`tauri dev`, an ad-hoc build) gets `NULL`, and must never reach
/// a container.
#[cfg(not(target_os = "ios"))]
fn entitled() -> bool {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};

    #[link(name = "Security", kind = "framework")]
    extern "C" {
        fn SecTaskCreateFromSelf(allocator: *const c_void) -> *mut c_void;
        fn SecTaskCopyValueForEntitlement(
            task: *mut c_void,
            entitlement: *const c_void,
            error: *mut *mut c_void,
        ) -> *mut c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    // SAFETY: plain C calls on objects this function creates and releases
    // itself. `NSString` is toll-free bridged to `CFString`, so its pointer is
    // the key the call wants; the value comes back at +1 and is released here.
    unsafe {
        let task = SecTaskCreateFromSelf(null());
        if task.is_null() {
            return false;
        }
        let key = NSString::from_str("com.apple.developer.icloud-services");
        let value = SecTaskCopyValueForEntitlement(
            task,
            (&*key as *const NSString).cast::<c_void>(),
            null_mut(),
        );
        let entitled = !value.is_null() && names_cloudkit(&*value.cast::<AnyObject>());
        if !value.is_null() {
            CFRelease(value);
        }
        CFRelease(task);
        entitled
    }
}

/// Whether an entitlement value grants CloudKit: the value is an array of
/// service names, and `CloudKit` is among them. A value of any other shape is
/// taken at its word — the entitlement is there.
#[cfg(not(target_os = "ios"))]
fn names_cloudkit(value: &AnyObject) -> bool {
    let Some(services) = value.downcast_ref::<NSArray<AnyObject>>() else {
        return true;
    };
    services.iter().any(|service| {
        service
            .downcast_ref::<NSString>()
            .is_some_and(|name| name.to_string() == "CloudKit")
    })
}

// --- plumbing ---

fn container() -> Retained<CKContainer> {
    unsafe { CKContainer::containerWithIdentifier(&NSString::from_str(CONTAINER_ID)) }
}

fn database() -> Retained<CKDatabase> {
    unsafe { container().privateCloudDatabase() }
}

/// A record id in the default zone.
fn record_id(name: &str) -> Retained<CKRecordID> {
    unsafe { CKRecordID::initWithRecordName(CKRecordID::alloc(), &NSString::from_str(name)) }
}

fn ns(text: &str) -> Retained<NSString> {
    NSString::from_str(text)
}

fn keys(names: &[&str]) -> Retained<NSArray<NSString>> {
    let names: Vec<Retained<NSString>> = names.iter().map(|name| ns(name)).collect();
    NSArray::from_retained_slice(&names)
}

/// Operations default to a quality of service under which a network failure
/// waits (up to a week, discretionary when the app is in the background).
/// A sync run wants the convenience calls' behaviour: fail, and be retried.
fn user_initiated(op: &CKOperation) {
    unsafe {
        let configuration = op.configuration();
        configuration.setQualityOfService(NSQualityOfService::UserInitiated);
        op.setConfiguration(Some(&configuration));
    }
}

/// The error a block was handed, as a plain value, read before the block
/// returns: the pointer is only good for the call.
///
/// # Safety
/// `error` is null or points to a live `NSError`, as CloudKit's handlers
/// guarantee for their error argument.
unsafe fn error_at(error: *mut NSError) -> Option<Failure> {
    error.as_ref().map(failure)
}

fn failure(error: &NSError) -> Failure {
    let cloudkit = unsafe { &*error.domain() == CKErrorDomain };
    Failure {
        code: cloudkit.then(|| error.code()),
        description: error.localizedDescription().to_string(),
    }
}

/// A handler that reported neither a result nor an error. CloudKit does not do
/// this; the mapping exists so no call can hang on a `None` it cannot explain.
fn no_answer() -> Failure {
    Failure {
        code: None,
        description: "iCloud gave no answer".into(),
    }
}

fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the store needs from a record, read while the record is alive.
fn fields(record: &CKRecord) -> Fields {
    let size = unsafe { record.objectForKey(&ns(SIZE_FIELD)) }.and_then(|value| {
        AsRef::<AnyObject>::as_ref(&*value)
            .downcast_ref::<NSNumber>()
            .map(|number| number.longLongValue())
    });
    Fields {
        name: unsafe { record.recordID().recordName() }.to_string(),
        size,
        modified: unsafe { record.modificationDate() }.map(|date| date.timeIntervalSince1970()),
        change_tag: unsafe { record.recordChangeTag() }.map(|tag| tag.to_string()),
    }
}

/// Where CloudKit put the asset's file: its cache, which it empties once the
/// record is released — so the file is read inside the block that holds it.
fn asset_path(record: &CKRecord) -> Option<PathBuf> {
    let value = unsafe { record.objectForKey(&ns(PACK_FIELD)) }?;
    let asset = AsRef::<AnyObject>::as_ref(&*value).downcast_ref::<CKAsset>()?;
    let url = unsafe { asset.fileURL() }?;
    Some(PathBuf::from(url.path()?.to_string()))
}

/// Put the bytes at `path` on the record as its asset, and their count as its
/// size. The asset is read from the file when the save runs, so the file has
/// to outlive the save (the callers hold the tempfile across the await).
fn set_pack(record: &CKRecord, path: &Path, size: usize) {
    let url = NSURL::fileURLWithPath(&ns(&path.to_string_lossy()));
    let asset = unsafe { CKAsset::initWithFileURL(CKAsset::alloc(), &url) };
    let size = NSNumber::numberWithLongLong(size as i64);
    unsafe {
        record.setObject_forKey(Some(ProtocolObject::from_ref(&*asset)), &ns(PACK_FIELD));
        record.setObject_forKey(Some(ProtocolObject::from_ref(&*size)), &ns(SIZE_FIELD));
    }
}

/// The bytes on disk for CloudKit to upload. Owner-only from creation, and
/// gone when dropped — after the save, never before.
fn stage(bytes: &[u8]) -> Result<tempfile::NamedTempFile> {
    let mut file = tempfile::NamedTempFile::new()?;
    file.write_all(bytes)?;
    file.flush()?;
    Ok(file)
}

/// The block a save answers: the saved record's change tag, or the failure
/// — with a `ServerRecordChanged` read as whatever the caller's compare-and-
/// swap losing means (`create`: someone made it first; `update`: it moved).
fn saved_block(
    done: Completion<Result<Option<String>>>,
    changed: fn() -> Error,
) -> RcBlock<dyn Fn(*mut CKRecord, *mut NSError)> {
    RcBlock::new(move |record: *mut CKRecord, error: *mut NSError| {
        let outcome = match unsafe { record.as_ref() } {
            Some(record) => Ok(unsafe { record.recordChangeTag() }.map(|tag| tag.to_string())),
            None => Err(match unsafe { error_at(error) } {
                Some(failure) if failure.is(SERVER_RECORD_CHANGED) => changed(),
                Some(failure) => failure.into_error(),
                None => no_answer().into_error(),
            }),
        };
        done.resolve(outcome);
    })
}

// --- the calls ---

/// `CKContainer.accountStatus`, as the raw status and the error's text if the
/// status came with one.
pub(super) async fn account_status() -> Result<(isize, Option<String>)> {
    Ok(start_account_status().await)
}

fn start_account_status() -> Completion<(isize, Option<String>)> {
    let done = Completion::new();
    let reply = {
        let done = done.clone();
        RcBlock::new(move |status: CKAccountStatus, error: *mut NSError| {
            let detail = unsafe { error_at(error) }.map(|failure| failure.description);
            done.resolve((status.0, detail));
        })
    };
    unsafe { container().accountStatusWithCompletionHandler(&reply) };
    done
}

/// The signed-in user's record name in this container: opaque, stable for one
/// Apple ID, different for another. What a connection is bound to.
pub(super) async fn user_record_name() -> Result<String> {
    start_user_record_name().await
}

fn start_user_record_name() -> Completion<Result<String>> {
    let done = Completion::new();
    let reply = {
        let done = done.clone();
        RcBlock::new(move |id: *mut CKRecordID, error: *mut NSError| {
            let outcome = match unsafe { id.as_ref() } {
                Some(id) => Ok(unsafe { id.recordName() }.to_string()),
                None => Err(unsafe { error_at(error) }
                    .unwrap_or_else(no_answer)
                    .into_error()),
            };
            done.resolve(outcome);
        })
    };
    unsafe { container().fetchUserRecordIDWithCompletionHandler(&reply) };
    done
}

/// Every `VaultEntry`, size field only, cursor followed to the end.
pub(super) async fn list() -> Result<Vec<RemoteEntry>> {
    start_list().await
}

fn start_list() -> Completion<Result<Vec<RemoteEntry>>> {
    let query = unsafe {
        CKQuery::initWithRecordType_predicate(
            CKQuery::alloc(),
            &ns(RECORD_TYPE),
            &NSPredicate::predicateWithValue(true),
        )
    };
    let first = unsafe { CKQueryOperation::initWithQuery(CKQueryOperation::alloc(), &query) };
    let done = Completion::new();
    query_page(
        database(),
        first,
        Arc::new(Mutex::new(Ok(Vec::new()))),
        done.clone(),
    );
    done
}

/// One page of the listing: every match goes into `found`, and the completion
/// either resolves `done` or, given a cursor, starts the next page with the
/// same accumulator. A record CloudKit could not materialise fails the whole
/// listing: an incomplete listing is the one thing a listing must not be.
fn query_page(
    db: Retained<CKDatabase>,
    op: Retained<CKQueryOperation>,
    found: Arc<Mutex<Result<Vec<RemoteEntry>>>>,
    done: Completion<Result<Vec<RemoteEntry>>>,
) {
    unsafe { op.setDesiredKeys(Some(&keys(&[SIZE_FIELD]))) };
    user_initiated(&op);
    let matched = {
        let found = found.clone();
        RcBlock::new(
            move |_id: NonNull<CKRecordID>, record: *mut CKRecord, error: *mut NSError| {
                let mut found = lock(&found);
                let Ok(entries) = found.as_mut() else {
                    return;
                };
                match unsafe { record.as_ref() } {
                    Some(record) => entries.push(entry(fields(record))),
                    None => {
                        *found = Err(unsafe { error_at(error) }
                            .unwrap_or_else(no_answer)
                            .into_error());
                    }
                }
            },
        )
    };
    let finished = {
        let db = db.clone();
        RcBlock::new(move |cursor: *mut CKQueryCursor, error: *mut NSError| {
            if let Some(failure) = unsafe { error_at(error) } {
                done.resolve(Err(failure.into_error()));
                return;
            }
            if let Some(cursor) = unsafe { cursor.as_ref() } {
                let next =
                    unsafe { CKQueryOperation::initWithCursor(CKQueryOperation::alloc(), cursor) };
                query_page(db.clone(), next, found.clone(), done.clone());
                return;
            }
            done.resolve(std::mem::replace(&mut *lock(&found), Ok(Vec::new())));
        })
    };
    unsafe {
        op.setRecordMatchedBlock(Some(&matched));
        op.setQueryCompletionBlock(Some(&finished));
        db.addOperation(&op);
    }
}

/// The record named `name`, size field only; `None` when there is none.
pub(super) async fn stat(name: &str) -> Result<Option<RemoteEntry>> {
    start_stat(name).await
}

/// The per-record block is what answers — it is handed either the record or
/// its own error, `UnknownItem` for a name that is not there — and the
/// operation's completion delivers that answer, or the operation's error when
/// it failed before any record was looked at (no network).
fn start_stat(name: &str) -> Completion<Result<Option<RemoteEntry>>> {
    let id = record_id(name);
    let op = unsafe {
        CKFetchRecordsOperation::initWithRecordIDs(
            CKFetchRecordsOperation::alloc(),
            &NSArray::from_slice(&[&*id]),
        )
    };
    unsafe { op.setDesiredKeys(Some(&keys(&[SIZE_FIELD]))) };
    user_initiated(&op);

    let answer: Arc<Mutex<Option<Result<Option<RemoteEntry>>>>> = Arc::default();
    let done = Completion::new();
    let per_record = {
        let answer = answer.clone();
        RcBlock::new(
            move |record: *mut CKRecord, _id: *mut CKRecordID, error: *mut NSError| {
                let outcome = match unsafe { record.as_ref() } {
                    Some(record) => Ok(Some(entry(fields(record)))),
                    None => match unsafe { error_at(error) } {
                        Some(failure) if failure.is(UNKNOWN_ITEM) => Ok(None),
                        Some(failure) => Err(failure.into_error()),
                        None => Err(no_answer().into_error()),
                    },
                };
                *lock(&answer) = Some(outcome);
            },
        )
    };
    let finished = {
        let done = done.clone();
        RcBlock::new(
            move |_records: *mut NSDictionary<CKRecordID, CKRecord>, error: *mut NSError| {
                let outcome = lock(&answer).take().unwrap_or_else(|| {
                    Err(unsafe { error_at(error) }
                        .unwrap_or_else(no_answer)
                        .into_error())
                });
                done.resolve(outcome);
            },
        )
    };
    unsafe {
        op.setPerRecordCompletionBlock(Some(&per_record));
        op.setFetchRecordsCompletionBlock(Some(&finished));
        database().addOperation(&op);
    }
    done
}

/// The record's pack, refused by its file size before it is read.
pub(super) async fn read(name: &str, max_bytes: usize) -> Result<Vec<u8>> {
    start_read(name, max_bytes).await
}

fn start_read(name: &str, max_bytes: usize) -> Completion<Result<Vec<u8>>> {
    let done = Completion::new();
    let reply = {
        let done = done.clone();
        RcBlock::new(move |record: *mut CKRecord, error: *mut NSError| {
            let outcome = match unsafe { record.as_ref() } {
                Some(record) => read_pack(record, max_bytes),
                None => Err(unsafe { error_at(error) }
                    .unwrap_or_else(no_answer)
                    .into_error()),
            };
            done.resolve(outcome);
        })
    };
    unsafe { database().fetchRecordWithID_completionHandler(&record_id(name), &reply) };
    done
}

// Inside the fetch block on purpose: the asset's file is CloudKit's cache,
// gone once the record is released (see `asset_path`).
fn read_pack(record: &CKRecord, max_bytes: usize) -> Result<Vec<u8>> {
    let path = asset_path(record)
        .ok_or_else(|| Error::Other("the iCloud record holds no vault file".into()))?;
    within_cap(std::fs::metadata(&path)?.len(), max_bytes)?;
    Ok(std::fs::read(&path)?)
}

/// A new record under `name`. The save is `ifServerRecordUnchanged`, so a
/// record already there (another device's first push, landed since the core's
/// `stat`) fails it rather than being replaced.
pub(super) async fn create(name: &str, bytes: &[u8]) -> Result<Option<String>> {
    let staged = stage(bytes)?;
    let outcome = start_create(name, staged.path(), bytes.len()).await;
    // Only now: CloudKit read the file while the save ran.
    drop(staged);
    outcome
}

fn start_create(name: &str, path: &Path, size: usize) -> Completion<Result<Option<String>>> {
    let record = unsafe {
        CKRecord::initWithRecordType_recordID(CKRecord::alloc(), &ns(RECORD_TYPE), &record_id(name))
    };
    set_pack(&record, path, size);
    let done = Completion::new();
    let saved = saved_block(done.clone(), create_raced);
    unsafe { database().saveRecord_completionHandler(&record, &saved) };
    done
}

/// Fetch the record, compare its change tag with the one the run pulled, put
/// the new pack on it and save — all inside the fetch's block, since the
/// record cannot leave it. The save is `ifServerRecordUnchanged` on top: a
/// write landing between the fetch and the save fails it too. A record that is
/// gone is `NotFound`, never a create.
pub(super) async fn update(
    name: &str,
    expected: Option<&str>,
    bytes: &[u8],
) -> Result<Option<String>> {
    let staged = stage(bytes)?;
    let outcome = start_update(name, expected, staged.path(), bytes.len()).await;
    drop(staged);
    outcome
}

/// The fetch asks for no fields at all (`desiredKeys = []`): the system
/// fields, change tag included, come regardless, and the pack this is about
/// to replace is the one thing a push must not download again — the run
/// already pulled it. CloudKit saves only the keys that changed, so a record
/// fetched without its fields is a fine one to put the new pack on.
///
/// As in [`start_stat`], the per-record block answers and the operation's
/// completion delivers the operation's own error when no record was looked
/// at; the save the per-record block starts answers on its own completion,
/// so the operation's must not speak for it.
fn start_update(
    name: &str,
    expected: Option<&str>,
    path: &Path,
    size: usize,
) -> Completion<Result<Option<String>>> {
    let path = path.to_path_buf();
    let expected = expected.map(str::to_owned);
    let db = database();
    let op = unsafe {
        CKFetchRecordsOperation::initWithRecordIDs(
            CKFetchRecordsOperation::alloc(),
            &NSArray::from_slice(&[&*record_id(name)]),
        )
    };
    unsafe { op.setDesiredKeys(Some(&keys(&[]))) };
    user_initiated(&op);

    let done = Completion::new();
    let answered = Arc::new(Mutex::new(false));
    let per_record = {
        let db = db.clone();
        let done = done.clone();
        let answered = answered.clone();
        RcBlock::new(
            move |record: *mut CKRecord, _id: *mut CKRecordID, error: *mut NSError| {
                *lock(&answered) = true;
                let Some(record) = (unsafe { record.as_ref() }) else {
                    done.resolve(Err(unsafe { error_at(error) }
                        .unwrap_or_else(no_answer)
                        .into_error()));
                    return;
                };
                let current = unsafe { record.recordChangeTag() }.map(|tag| tag.to_string());
                if stale(expected.as_deref(), current.as_deref()) {
                    done.resolve(Err(changed_since_pull()));
                    return;
                }
                set_pack(record, &path, size);
                let saved = saved_block(done.clone(), changed_since_pull);
                unsafe { db.saveRecord_completionHandler(record, &saved) };
            },
        )
    };
    let finished = {
        let done = done.clone();
        RcBlock::new(
            move |_records: *mut NSDictionary<CKRecordID, CKRecord>, error: *mut NSError| {
                if *lock(&answered) {
                    return;
                }
                done.resolve(Err(unsafe { error_at(error) }
                    .unwrap_or_else(no_answer)
                    .into_error()));
            },
        )
    };
    unsafe {
        op.setPerRecordCompletionBlock(Some(&per_record));
        op.setFetchRecordsCompletionBlock(Some(&finished));
        db.addOperation(&op);
    }
    done
}

/// Delete by name; a record that is already gone counts as deleted.
pub(super) async fn delete(name: &str) -> Result<()> {
    start_delete(name).await
}

fn start_delete(name: &str) -> Completion<Result<()>> {
    let done = Completion::new();
    let reply = {
        let done = done.clone();
        RcBlock::new(move |_id: *mut CKRecordID, error: *mut NSError| {
            let outcome = match unsafe { error_at(error) } {
                None => Ok(()),
                Some(failure) if failure.is(UNKNOWN_ITEM) => Ok(()),
                Some(failure) => Err(failure.into_error()),
            };
            done.resolve(outcome);
        })
    };
    unsafe { database().deleteRecordWithID_completionHandler(&record_id(name), &reply) };
    done
}
