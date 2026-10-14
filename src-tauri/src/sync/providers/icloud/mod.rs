//! iCloud, through CloudKit.
//!
//! The account is the device's own: there is no sign-in and no token, so the
//! [`Auth`] a store is built with is kept only to be handed back
//! ([`Store::into_auth`]), and connecting is just [`check_account`]. What the
//! core calls a file in `Vaults/` is here one record:
//!
//! ```text
//! container   iCloud.app.rowel, private database, default zone
//! record type VaultEntry
//! record name <vault-id>.rowel / <vault-id>.deleted   (the core's entry name)
//! fields      pack: CKAsset (the bytes)   size: Int64 (their count)
//! ```
//!
//! The record name is the handle, the name and the key all at once: CloudKit
//! record names are unique in a zone, so like Dropbox there are no same-name
//! siblings to choose between. The revision is the server's `recordChangeTag`,
//! which every save is conditional on (`ifServerRecordUnchanged`): an update
//! over a record another device rewrote fails rather than overwrites, and a
//! create over a name that already exists fails rather than replaces.
//!
//! CloudKit is Apple's framework, so the adapter only does anything on Apple
//! targets ([`cloudkit`]), and even there only in a binary signed for it: an
//! unsigned build (`tauri dev`) that touched a container would be killed with
//! an Objective-C exception, which is what [`supported`] guards against.
//! Everywhere else the variant still compiles and every call says so.

// The plain-value half below (record fields, error codes, the compare-and-swap
// check, the cap) is what `cloudkit` and the tests consume. Off Apple nothing
// but the gate and the stub is live, and that is not dead code to fix — the
// adapter is simply absent there.
#![cfg_attr(not(target_vendor = "apple"), allow(dead_code))]

#[cfg(target_vendor = "apple")]
mod cloudkit;
#[cfg(target_vendor = "apple")]
mod completion;

use crate::error::{Error, Result};
use crate::sync::account::Auth;
use crate::sync::store::{RemoteEntry, VaultStore};
use crate::sync::Provider;

/// The CloudKit container the app is entitled to.
pub const CONTAINER_ID: &str = "iCloud.app.rowel";
/// The one record type; its two fields.
const RECORD_TYPE: &str = "VaultEntry";
const PACK_FIELD: &str = "pack";
const SIZE_FIELD: &str = "size";

// `CKErrorCode`, as plain numbers so the mapping below has nothing to link
// against off Apple. Pinned to the framework's constants by a test there.
const NETWORK_UNAVAILABLE: isize = 3;
const NETWORK_FAILURE: isize = 4;
const SERVICE_UNAVAILABLE: isize = 6;
const REQUEST_RATE_LIMITED: isize = 7;
const NOT_AUTHENTICATED: isize = 9;
const UNKNOWN_ITEM: isize = 11;
const SERVER_RECORD_CHANGED: isize = 14;
const ZONE_BUSY: isize = 23;
const QUOTA_EXCEEDED: isize = 25;
/// Transient: iCloud could not be reached or asked to wait. A run failing on
/// one is retried by the scheduler like any other run failure.
const UNREACHABLE: [isize; 5] = [
    NETWORK_UNAVAILABLE,
    NETWORK_FAILURE,
    SERVICE_UNAVAILABLE,
    REQUEST_RATE_LIMITED,
    ZONE_BUSY,
];

// `CKAccountStatus`, likewise.
const ACCOUNT_COULD_NOT_DETERMINE: isize = 0;
const ACCOUNT_AVAILABLE: isize = 1;
const ACCOUNT_RESTRICTED: isize = 2;
const ACCOUNT_NO_ACCOUNT: isize = 3;
const ACCOUNT_TEMPORARILY_UNAVAILABLE: isize = 4;

const SIGN_IN: &str = "Sign in to iCloud on this device in System Settings, then try again.";
const UNSUPPORTED: &str = "iCloud sync is only available on Apple devices";
const UNSIGNED: &str = "this build is not signed for iCloud";
const ACCOUNT_CHANGED: &str = "this device is signed into a different iCloud account than the one \
                               this vault syncs through; disconnect, then connect iCloud again";
const NOT_BOUND: &str =
    "this vault's iCloud connection does not say which account; disconnect, then connect again";

/// [`VaultStore`] over the device's iCloud account — the one the workspace was
/// connected to, and no other.
///
/// Nothing is cached and nothing is held: the container is looked up per call
/// (CloudKit keeps one instance per identifier), and every `stat` asks the
/// server, which is what `head_revision` relies on to see another device's
/// write. The one thing settled once per store is *whose* account the device
/// is signed into ([`Store::bound`]): a store lives for one run, and the
/// device cannot change its Apple ID in the middle of one.
pub struct Store {
    auth: Auth,
    /// The account the token file names, read when the store is built.
    expected: Option<String>,
    /// Whether the device's account has been checked against it.
    bound: std::sync::Mutex<bool>,
}

impl Store {
    pub fn new(auth: Auth) -> Self {
        let expected = auth.bound_account();
        Self {
            auth,
            expected,
            bound: std::sync::Mutex::new(false),
        }
    }

    pub fn into_auth(self) -> Auth {
        self.auth
    }

    /// Every call's first step: this build can reach CloudKit, and the account
    /// it reaches is the one the workspace was connected to. The device's
    /// account is asked once per store; a mismatch fails every call the same
    /// way until the user reconnects, so a vault is never pushed into an
    /// account it was not connected to because the device changed Apple ID.
    async fn bound(&self) -> Result<()> {
        ready()?;
        if *self.bound.lock().unwrap() {
            return Ok(());
        }
        // No lock across the round trip: a second call racing this one asks
        // too, and both compare against the same expectation.
        let current = imp::user_record_name().await?;
        same_account(self.expected.as_deref(), &current)?;
        *self.bound.lock().unwrap() = true;
        Ok(())
    }
}

/// Whether the account the device reaches is the one the workspace is bound
/// to. No binding at all is a connect that never finished recording one, and
/// is treated like a stranger's account: reconnecting is the only way to say
/// which account was meant.
fn same_account(expected: Option<&str>, current: &str) -> Result<()> {
    match expected {
        Some(expected) if expected == current => Ok(()),
        Some(_) => Err(Error::Other(ACCOUNT_CHANGED.into())),
        None => Err(Error::Other(NOT_BOUND.into())),
    }
}

/// Whether this build can sync through iCloud at all.
///
/// `false` off Apple. `true` on iOS, where the committed entitlements carry it.
/// On macOS, `true` only when the running binary is signed with the
/// `com.apple.developer.icloud-services` entitlement — `tauri dev` and unsigned
/// builds have none, and CloudKit aborts the process on first use without it.
/// Read once: the answer cannot change while the process runs.
pub fn supported() -> bool {
    imp::supported()
}

/// The device's iCloud account can be used — `CKContainer.accountStatus` is
/// `Available`; anything else is an error that says what the user can do —
/// and which account it is: CloudKit's user record name for this container,
/// an opaque id that is stable for one Apple ID and differs for another. The
/// connect seals it ([`crate::sync::account::Tokens::account`]) and every
/// store checks the device against it ([`Store::bound`]).
pub async fn check_account() -> Result<String> {
    ready()?;
    let (status, detail) = imp::account_status().await?;
    account_error(status, detail.as_deref())?;
    imp::user_record_name().await
}

/// The gate every call passes first. Off Apple, or in a build CloudKit would
/// kill, the call never reaches the framework.
fn ready() -> Result<()> {
    if cfg!(target_vendor = "apple") {
        if supported() {
            Ok(())
        } else {
            Err(Error::Unsupported(UNSIGNED.into()))
        }
    } else {
        Err(Error::Unsupported(UNSUPPORTED.into()))
    }
}

/// A record name as this adapter uses them: the core's entry name, which is
/// also the handle the frontend round-trips. CloudKit limits names to 255
/// ASCII characters and reserves a leading underscore; anything outside that
/// is refused here rather than sent — it cannot be an entry a listing or
/// `stat` returned.
fn record_name(name: &str) -> Result<&str> {
    let plain = !name.is_empty()
        && name.len() <= 255
        && !name.starts_with('_')
        && name.bytes().all(|b| b.is_ascii_graphic());
    if plain {
        Ok(name)
    } else {
        Err(Error::Other(format!("not an iCloud record name: {name:?}")))
    }
}

/// What the adapter reads off a `CKRecord`, as plain values: the Objective-C
/// object is not `Send` and never leaves the completion block it arrived in.
struct Fields {
    name: String,
    /// The `size` field, if the record has one (a listing asks only for it).
    size: Option<i64>,
    /// `modificationDate` as seconds since the epoch.
    modified: Option<f64>,
    change_tag: Option<String>,
}

/// A record as the core sees it: its name is both handle and name, its
/// modification date the time the probe orders by, its change tag the
/// revision — or `None` when the server gave none, never a made-up value.
fn entry(fields: Fields) -> RemoteEntry {
    RemoteEntry {
        handle: fields.name.clone(),
        name: fields.name,
        size: fields.size.and_then(|size| u64::try_from(size).ok()),
        modified: fields.modified.map(rfc3339).unwrap_or_default(),
        revision: fields.change_tag.filter(|tag| !tag.is_empty()),
    }
}

/// Seconds since the epoch as RFC 3339 UTC with milliseconds, like Drive's
/// `modifiedTime`. Empty for a value no date can hold.
fn rfc3339(secs: f64) -> String {
    let whole = secs.floor();
    let nanos = ((secs - whole) * 1e9).round() as u32;
    chrono::DateTime::<chrono::Utc>::from_timestamp(whole as i64, nanos.min(999_999_999))
        .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}

/// Whether an update over `expected` (the revision the run pulled) would
/// overwrite a write it never merged: the server holds `current`, and the two
/// are known and differ. An unknown on either side is nothing to compare, and
/// the save itself is still conditional on the record as fetched.
fn stale(expected: Option<&str>, current: Option<&str>) -> bool {
    matches!((expected, current), (Some(e), Some(c)) if e != c)
}

fn changed_since_pull() -> Error {
    Error::Other(
        "the vault file changed on iCloud since this device last pulled it; sync will retry".into(),
    )
}

fn create_raced() -> Error {
    Error::Other(
        "another device created the vault file on iCloud while this one was syncing; \
         sync will retry"
            .into(),
    )
}

/// The cap, checked against the file's size before a byte of it is read.
fn within_cap(len: u64, max_bytes: usize) -> Result<()> {
    if u64::try_from(max_bytes).is_ok_and(|max| len <= max) {
        Ok(())
    } else {
        Err(Error::FileTooLarge)
    }
}

/// An `NSError` as the adapter keeps it: the `CKErrorCode` when the error is
/// CloudKit's own, and the text Apple wrote for the user.
struct Failure {
    code: Option<isize>,
    description: String,
}

impl Failure {
    fn is(&self, code: isize) -> bool {
        self.code == Some(code)
    }

    /// The error the core gets. Transient codes say so, so a run failing on one
    /// reads as "try later" rather than as a broken account; a missing record
    /// is the core's `NotFound`; everything else carries Apple's description.
    fn into_error(self) -> Error {
        match self.code {
            Some(NOT_AUTHENTICATED) => Error::Other(SIGN_IN.into()),
            Some(UNKNOWN_ITEM) => Error::NotFound,
            Some(QUOTA_EXCEEDED) => Error::Other("this iCloud account is out of storage".into()),
            Some(code) if UNREACHABLE.contains(&code) => {
                Error::Other("iCloud could not be reached; sync will retry".into())
            }
            _ => Error::Other(format!("iCloud: {}", self.description)),
        }
    }
}

/// `CKContainer.accountStatus` as the connect reads it. Only `Available` is a
/// usable account; each of the others names the one thing the user can do.
fn account_error(status: isize, detail: Option<&str>) -> Result<()> {
    let why = match status {
        ACCOUNT_AVAILABLE => return Ok(()),
        ACCOUNT_NO_ACCOUNT => SIGN_IN.to_string(),
        ACCOUNT_RESTRICTED => {
            "iCloud is restricted on this device (parental controls or a device profile), \
             so it cannot be used for sync."
                .into()
        }
        ACCOUNT_TEMPORARILY_UNAVAILABLE => {
            "This device's iCloud account needs attention; finish signing in to iCloud in \
             System Settings, then try again."
                .into()
        }
        // `CouldNotDetermine` carries CloudKit's own reason when it has one;
        // a status a newer OS adds is read the same way, as nothing better
        // is known about it.
        ACCOUNT_COULD_NOT_DETERMINE => could_not_reach(detail),
        _ => could_not_reach(detail),
    };
    Err(Error::Other(why))
}

fn could_not_reach(detail: Option<&str>) -> String {
    match detail {
        Some(detail) => format!("iCloud could not be reached ({detail}); try again."),
        None => "iCloud could not be reached; try again.".into(),
    }
}

impl VaultStore for Store {
    fn provider(&self) -> Provider {
        Provider::ICloud
    }

    // A query over every `VaultEntry`, cursor followed to the end: an account
    // nothing has synced to simply matches nothing.
    async fn list_vaults(&self) -> Result<Vec<RemoteEntry>> {
        self.bound().await?;
        imp::list().await
    }

    async fn stat(&self, name: &str) -> Result<Option<RemoteEntry>> {
        self.bound().await?;
        imp::stat(record_name(name)?).await
    }

    async fn read(&self, handle: &str, max_bytes: usize) -> Result<Vec<u8>> {
        self.bound().await?;
        imp::read(record_name(handle)?, max_bytes).await
    }

    // A new record under the name, saved `ifServerRecordUnchanged`: one that
    // already exists — another device's first push landing between the core's
    // `stat` and this call — fails rather than being replaced.
    async fn create(&self, name: &str, bytes: &[u8]) -> Result<Option<String>> {
        self.bound().await?;
        imp::create(record_name(name)?, bytes).await
    }

    // A compare-and-swap on the revision the run pulled (see [`stale`]), then
    // on the record as fetched. A record that is gone is an error, never a
    // create: the core asks about the deletion marker on the create path only.
    async fn update(&self, entry: &RemoteEntry, bytes: &[u8]) -> Result<Option<String>> {
        self.bound().await?;
        let expected = entry.revision.as_deref().filter(|rev| !rev.is_empty());
        imp::update(record_name(&entry.handle)?, expected, bytes).await
    }

    async fn delete(&self, handle: &str) -> Result<()> {
        self.bound().await?;
        imp::delete(record_name(handle)?).await
    }
}

#[cfg(target_vendor = "apple")]
use cloudkit as imp;

/// Every other target: the variant exists, and every call says why it cannot
/// be served. Never reached through [`Store`] (`ready` fails first), but what
/// the public functions dispatch to has to exist.
#[cfg(not(target_vendor = "apple"))]
mod imp {
    use super::{RemoteEntry, Result};

    fn unsupported<T>() -> Result<T> {
        Err(crate::error::Error::Unsupported(super::UNSUPPORTED.into()))
    }

    pub(super) fn supported() -> bool {
        false
    }

    pub(super) async fn account_status() -> Result<(isize, Option<String>)> {
        unsupported()
    }

    pub(super) async fn user_record_name() -> Result<String> {
        unsupported()
    }

    pub(super) async fn list() -> Result<Vec<RemoteEntry>> {
        unsupported()
    }

    pub(super) async fn stat(_name: &str) -> Result<Option<RemoteEntry>> {
        unsupported()
    }

    pub(super) async fn read(_name: &str, _max_bytes: usize) -> Result<Vec<u8>> {
        unsupported()
    }

    pub(super) async fn create(_name: &str, _bytes: &[u8]) -> Result<Option<String>> {
        unsupported()
    }

    pub(super) async fn update(
        _name: &str,
        _expected: Option<&str>,
        _bytes: &[u8],
    ) -> Result<Option<String>> {
        unsupported()
    }

    pub(super) async fn delete(_name: &str) -> Result<()> {
        unsupported()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2024-05-04T10:11:12Z.
    const MAY_FOURTH: f64 = 1_714_817_472.0;

    fn record(name: &str) -> Fields {
        Fields {
            name: name.into(),
            size: Some(2048),
            modified: Some(MAY_FOURTH),
            change_tag: Some("k3f9a2c".into()),
        }
    }

    // A record reaches the core under its record name twice over — it is the
    // handle the frontend round-trips and the name the core matches — with
    // its change tag as the revision and its modification date as the time
    // the probe orders by.
    #[test]
    fn an_icloud_record_becomes_an_entry_named_by_its_record_name() {
        assert_eq!(
            entry(record("a1b2.rowel")),
            RemoteEntry {
                handle: "a1b2.rowel".into(),
                name: "a1b2.rowel".into(),
                size: Some(2048),
                modified: "2024-05-04T10:11:12.000Z".into(),
                revision: Some("k3f9a2c".into()),
            }
        );
    }

    // Unknown stays unknown: no tag (or an empty one) is `None`, never a
    // fabricated revision; no date is an empty string; a size the server
    // never set, or one no byte count can be, is `None`.
    #[test]
    fn a_record_without_a_change_tag_has_no_revision() {
        let mut fields = record("a1b2.rowel");
        fields.change_tag = None;
        fields.modified = None;
        fields.size = None;
        let e = entry(fields);
        assert_eq!(e.revision, None);
        assert_eq!(e.modified, "");
        assert_eq!(e.size, None);

        let mut fields = record("a1b2.rowel");
        fields.change_tag = Some(String::new());
        fields.size = Some(-1);
        let e = entry(fields);
        assert_eq!(e.revision, None);
        assert_eq!(e.size, None);
    }

    #[test]
    fn modification_dates_are_rfc_3339_utc_with_milliseconds() {
        assert_eq!(rfc3339(0.0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339(MAY_FOURTH), "2024-05-04T10:11:12.000Z");
        assert_eq!(rfc3339(MAY_FOURTH + 0.5), "2024-05-04T10:11:12.500Z");
        // Far outside what a date can hold: empty rather than nonsense.
        assert_eq!(rfc3339(1e300), "");
    }

    // The handle round-trips through the frontend: only something CloudKit
    // would accept as a record name is ever sent as one.
    #[test]
    fn only_plain_ascii_names_are_record_names() {
        assert_eq!(record_name("a1b2.rowel").unwrap(), "a1b2.rowel");
        assert_eq!(record_name("a1b2.deleted").unwrap(), "a1b2.deleted");
        assert!(record_name("").is_err());
        assert!(record_name("_system").is_err());
        assert!(record_name("with space").is_err());
        assert!(record_name("café.rowel").is_err());
        assert!(record_name("a\nb").is_err());
        assert!(record_name(&"x".repeat(255)).is_ok());
        assert!(record_name(&"x".repeat(256)).is_err());
    }

    fn failure(code: Option<isize>) -> Failure {
        Failure {
            code,
            description: "The operation couldn’t be completed.".into(),
        }
    }

    // Each code becomes the thing the user (or the scheduler) can do about it;
    // what is left carries Apple's own description so the log says something.
    #[test]
    fn cloudkit_errors_read_as_what_can_be_done_about_them() {
        match failure(Some(NOT_AUTHENTICATED)).into_error() {
            Error::Other(why) => assert_eq!(why, SIGN_IN),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            failure(Some(UNKNOWN_ITEM)).into_error(),
            Error::NotFound
        ));
        for code in UNREACHABLE {
            match failure(Some(code)).into_error() {
                Error::Other(why) => assert!(why.contains("sync will retry"), "{code}: {why}"),
                other => panic!("{other:?}"),
            }
        }
        match failure(Some(QUOTA_EXCEEDED)).into_error() {
            Error::Other(why) => assert!(why.contains("storage")),
            other => panic!("{other:?}"),
        }
        for code in [Some(SERVER_RECORD_CHANGED), Some(99), None] {
            match failure(code).into_error() {
                Error::Other(why) => {
                    assert!(why.contains("couldn’t be completed"), "{code:?}: {why}")
                }
                other => panic!("{other:?}"),
            }
        }
    }

    // Only an available account connects; every other status tells the user
    // where to go, and the one CloudKit itself could not settle carries what
    // it said.
    #[test]
    fn an_account_status_other_than_available_is_an_error() {
        assert!(account_error(ACCOUNT_AVAILABLE, None).is_ok());
        match account_error(ACCOUNT_NO_ACCOUNT, None) {
            Err(Error::Other(why)) => assert_eq!(why, SIGN_IN),
            other => panic!("{other:?}"),
        }
        match account_error(ACCOUNT_RESTRICTED, None) {
            Err(Error::Other(why)) => assert!(why.contains("restricted")),
            other => panic!("{other:?}"),
        }
        match account_error(ACCOUNT_TEMPORARILY_UNAVAILABLE, None) {
            Err(Error::Other(why)) => assert!(why.contains("System Settings")),
            other => panic!("{other:?}"),
        }
        match account_error(ACCOUNT_COULD_NOT_DETERMINE, Some("no network")) {
            Err(Error::Other(why)) => assert!(why.contains("no network")),
            other => panic!("{other:?}"),
        }
        assert!(account_error(99, None).is_err());
    }

    // The device's account has to be the one the workspace was connected to:
    // another Apple ID is a different, empty private database, and a run
    // there would push the vault into it. A connection that never recorded
    // its account is sent back to connect, not waved through.
    #[test]
    fn only_the_bound_account_is_synced_to() {
        assert!(same_account(Some("_1a2b"), "_1a2b").is_ok());
        match same_account(Some("_1a2b"), "_9z8y") {
            Err(Error::Other(why)) => assert!(why.contains("different iCloud account"), "{why}"),
            other => panic!("{other:?}"),
        }
        match same_account(None, "_1a2b") {
            Err(Error::Other(why)) => assert!(why.contains("connect again"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    // The compare half of the compare-and-swap: a known revision that differs
    // from the server's is a write another device made since the pull. An
    // unknown on either side is nothing to compare — the save is still
    // conditional on the record as fetched.
    #[test]
    fn an_update_is_stale_only_when_both_revisions_are_known_and_differ() {
        assert!(stale(Some("pulled"), Some("theirs")));
        assert!(!stale(Some("pulled"), Some("pulled")));
        assert!(!stale(None, Some("theirs")));
        assert!(!stale(Some("pulled"), None));
        assert!(!stale(None, None));
    }

    // The cap is applied to the file's size, before any of it is buffered.
    #[test]
    fn a_pack_over_the_cap_is_refused_before_it_is_read() {
        assert!(within_cap(10, 10).is_ok());
        assert!(matches!(within_cap(11, 10), Err(Error::FileTooLarge)));
        assert!(within_cap(0, 0).is_ok());
    }

    // An update hands the record name on, with the pulled revision — and an
    // empty one, the engine's "unknown", as nothing to compare.
    #[test]
    fn the_container_is_the_apps_own() {
        assert_eq!(CONTAINER_ID, "iCloud.app.rowel");
        assert_eq!(RECORD_TYPE, "VaultEntry");
    }

    // Off Apple every call is refused before it could reach anything, and the
    // provider is not offered.
    #[cfg(not(target_vendor = "apple"))]
    #[test]
    fn off_apple_icloud_is_unsupported() {
        use tauri::async_runtime::block_on;
        assert!(!supported());
        assert!(matches!(
            block_on(check_account()),
            Err(Error::Unsupported(_))
        ));
    }

    // The plain numbers above are CloudKit's: a framework update that renumbers
    // them (it never has) would fail here rather than mis-map an error.
    #[cfg(target_vendor = "apple")]
    #[test]
    fn the_error_and_account_codes_are_cloudkits() {
        use objc2_cloud_kit::{CKAccountStatus, CKErrorCode};
        assert_eq!(CKErrorCode::NetworkUnavailable.0, NETWORK_UNAVAILABLE);
        assert_eq!(CKErrorCode::NetworkFailure.0, NETWORK_FAILURE);
        assert_eq!(CKErrorCode::ServiceUnavailable.0, SERVICE_UNAVAILABLE);
        assert_eq!(CKErrorCode::RequestRateLimited.0, REQUEST_RATE_LIMITED);
        assert_eq!(CKErrorCode::NotAuthenticated.0, NOT_AUTHENTICATED);
        assert_eq!(CKErrorCode::UnknownItem.0, UNKNOWN_ITEM);
        assert_eq!(CKErrorCode::ServerRecordChanged.0, SERVER_RECORD_CHANGED);
        assert_eq!(CKErrorCode::ZoneBusy.0, ZONE_BUSY);
        assert_eq!(CKErrorCode::QuotaExceeded.0, QUOTA_EXCEEDED);
        assert_eq!(
            CKAccountStatus::CouldNotDetermine.0,
            ACCOUNT_COULD_NOT_DETERMINE
        );
        assert_eq!(CKAccountStatus::Available.0, ACCOUNT_AVAILABLE);
        assert_eq!(CKAccountStatus::Restricted.0, ACCOUNT_RESTRICTED);
        assert_eq!(CKAccountStatus::NoAccount.0, ACCOUNT_NO_ACCOUNT);
        assert_eq!(
            CKAccountStatus::TemporarilyUnavailable.0,
            ACCOUNT_TEMPORARILY_UNAVAILABLE
        );
    }

    // A test binary is never signed for iCloud, so on macOS the gate must hold
    // it back from CloudKit — this is the exact case `supported` exists for.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_unsigned_build_is_kept_away_from_cloudkit() {
        use tauri::async_runtime::block_on;
        assert!(!supported());
        match block_on(check_account()) {
            Err(Error::Unsupported(why)) => assert_eq!(why, UNSIGNED),
            other => panic!("{other:?}"),
        }
    }
}
