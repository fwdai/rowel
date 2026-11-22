//! The device book: what nearby sync keeps per install — on or off, the
//! devices paused, removed and seen — and what is known about each device
//! right now, and the status the frontend is shown, built from the two.
//!
//! Every change goes through here, and every change is persisted (when it
//! touches what is kept) and announced in the same step, under the book's
//! lock, so neither the file nor the events can land in another order than
//! the changes did.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use chrono::{SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// The app's error kind (`crate::error::Error::kind`) and its sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Failure {
    pub kind: String,
    pub message: String,
}

/// The `seq` of every nearby status, counted once for the process, as the
/// sync status's is: a status read before a change reads as older than the
/// one the change announced, whichever manager or book made it.
static SEQ: AtomicU64 = AtomicU64::new(0);

/// The whole of nearby sync as the frontend sees it, carried by every
/// `nearby:status` event and by `nearby_status` (design §7). Like the sync
/// status, one snapshot rather than a sequence of events: the frontend keeps
/// the one with the higher `seq`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NearbyStatus {
    /// This build can do it at all.
    pub available: bool,
    pub enabled: bool,
    pub device_name: String,
    pub devices: Vec<NearbyDevice>,
    /// Monotonic within the process.
    pub seq: u64,
}

/// One of the owner's other devices: one that proved a lockbox key here.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NearbyDevice {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub state: NearbyDeviceState,
    /// RFC 3339 time of the last exchange with it that converged.
    pub last_synced_at: Option<String>,
    /// For `clockSkew` and `error`: the sentence, and its kind
    /// (`crate::error::Error::kind`), which is what the frontend translates.
    pub detail: Option<String>,
    pub detail_kind: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum NearbyDeviceState {
    /// A run with it is in flight.
    Syncing,
    /// Connected, nothing to send.
    UpToDate,
    /// Known, not on the network now.
    Offline,
    /// This device stopped syncing with it.
    Paused,
    /// One of the two is on a master password the other has replaced.
    NeedsPassword,
    ClockSkew,
    Error,
}

/// What is kept per install, beside the other settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Stored {
    pub enabled: bool,
    /// The devices this one does not sync with until resumed, by id.
    pub paused: BTreeSet<String>,
    /// The devices this one refuses, by id. Only ever grows: a device is
    /// removed here or learned removed from another device, and nothing takes
    /// it back out.
    pub removed: BTreeSet<String>,
    /// Every device that proved a lockbox key here, in the order first met.
    pub devices: Vec<Known>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Known {
    pub id: String,
    pub name: String,
    pub platform: String,
    /// When an exchange with it last converged, in ms since the Unix epoch.
    #[serde(default)]
    pub last_synced_ms: Option<i64>,
}

/// What is known about the devices right now: one manager's, reset when it
/// starts and when it stops.
#[derive(Default)]
pub(super) struct Live {
    /// The devices whose advertisements are seen on the network now.
    pub here: HashSet<String>,
    pub syncing: HashSet<String>,
    /// One side of the connection is on a key the other has retired.
    pub behind: HashSet<String>,
    /// What the last run with the device failed with, until the next starts.
    pub failed: HashMap<String, Failure>,
}

struct Inner {
    stored: Stored,
    live: Live,
    /// Which manager may change `live`: a thread of one that has stopped
    /// finds its changes dropped, rather than leaving a device "syncing"
    /// behind the lock screen.
    generation: u64,
    seq: u64,
}

type Persist = Box<dyn Fn(&Stored) + Send + Sync>;
type Emit = Box<dyn Fn(NearbyStatus) + Send + Sync>;

pub struct Book {
    inner: Mutex<Inner>,
    device_name: String,
    persist: Persist,
    emit: Emit,
}

impl Book {
    /// The book over `stored`, for the device called `device_name`. `persist`
    /// writes what is kept, `emit` announces a status; both are called under
    /// the book's lock and must not call back into it.
    pub fn new(stored: Stored, device_name: String, persist: Persist, emit: Emit) -> Self {
        Self {
            inner: Mutex::new(Inner {
                stored,
                live: Live::default(),
                generation: 0,
                seq: 0,
            }),
            device_name,
            persist,
            emit,
        }
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// The status as it stands, under the `seq` of the last change.
    pub fn status(&self) -> NearbyStatus {
        let inner = self.inner.lock().unwrap();
        self.status_of(&inner)
    }

    pub fn enabled(&self) -> bool {
        self.inner.lock().unwrap().stored.enabled
    }

    pub fn is_paused(&self, id: &str) -> bool {
        self.inner.lock().unwrap().stored.paused.contains(id)
    }

    pub fn is_removed(&self, id: &str) -> bool {
        self.inner.lock().unwrap().stored.removed.contains(id)
    }

    pub fn removed(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        inner.stored.removed.iter().cloned().collect()
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.change(|stored| stored.enabled = enabled);
    }

    pub fn set_paused(&self, id: &str, paused: bool) {
        self.change(|stored| {
            if paused {
                stored.paused.insert(id.to_string());
            } else {
                stored.paused.remove(id);
            }
        });
    }

    /// Add `ids` to the removed set. The ones it did not hold come back, and
    /// only a change that has some is persisted and announced.
    pub fn remove(&self, ids: &[String]) -> Vec<String> {
        let mut inner = self.inner.lock().unwrap();
        let added: Vec<String> = ids
            .iter()
            .filter(|id| inner.stored.removed.insert((*id).clone()))
            .cloned()
            .collect();
        if !added.is_empty() {
            for id in &added {
                inner.stored.paused.remove(id);
                inner.live.here.remove(id);
                inner.live.syncing.remove(id);
            }
            self.changed(&mut inner, true);
        }
        added
    }

    /// The device `id` proved a lockbox key and said who it is.
    pub(super) fn seen(&self, id: &str, name: &str, platform: &str) {
        let platform = known_platform(platform);
        self.change(
            |stored| match stored.devices.iter_mut().find(|d| d.id == id) {
                Some(known) => {
                    known.name = name.to_string();
                    known.platform = platform.to_string();
                }
                None => stored.devices.push(Known {
                    id: id.to_string(),
                    name: name.to_string(),
                    platform: platform.to_string(),
                    last_synced_ms: None,
                }),
            },
        );
    }

    /// An exchange with `id` converged at `at_ms`.
    pub(super) fn converged(&self, id: &str, at_ms: i64) {
        self.change(|stored| {
            if let Some(known) = stored.devices.iter_mut().find(|d| d.id == id) {
                known.last_synced_ms = Some(at_ms);
            }
        });
    }

    /// A manager starts: what was known now is forgotten, and the generation
    /// it may change it under comes back.
    pub(super) fn attach(&self) -> u64 {
        let mut inner = self.inner.lock().unwrap();
        inner.generation += 1;
        inner.live = Live::default();
        self.changed(&mut inner, false);
        inner.generation
    }

    /// The manager of `generation` stopped: every device is offline.
    pub(super) fn detach(&self, generation: u64) {
        let mut inner = self.inner.lock().unwrap();
        if inner.generation != generation {
            return;
        }
        inner.generation += 1;
        inner.live = Live::default();
        self.changed(&mut inner, false);
    }

    /// Change what is known right now, for the manager of `generation` only.
    pub(super) fn live(&self, generation: u64, change: impl FnOnce(&mut Live)) {
        let mut inner = self.inner.lock().unwrap();
        if inner.generation != generation {
            return;
        }
        change(&mut inner.live);
        self.changed(&mut inner, false);
    }

    /// Change what is kept: persisted and announced.
    fn change(&self, change: impl FnOnce(&mut Stored)) {
        let mut inner = self.inner.lock().unwrap();
        change(&mut inner.stored);
        self.changed(&mut inner, true);
    }

    fn changed(&self, inner: &mut Inner, persist: bool) {
        inner.seq = SEQ.fetch_add(1, Ordering::SeqCst) + 1;
        if persist {
            (self.persist)(&inner.stored);
        }
        (self.emit)(self.status_of(inner));
    }

    fn status_of(&self, inner: &Inner) -> NearbyStatus {
        let Inner { stored, live, .. } = inner;
        NearbyStatus {
            available: true,
            enabled: stored.enabled,
            device_name: self.device_name.clone(),
            devices: stored
                .devices
                .iter()
                .filter(|known| !stored.removed.contains(&known.id))
                .map(|known| device(known, stored, live))
                .collect(),
            seq: inner.seq,
        }
    }
}

/// One device's row. Paused says so whether or not it is here; then
/// whether it is here at all; then what needs doing; then what is happening.
fn device(known: &Known, stored: &Stored, live: &Live) -> NearbyDevice {
    let id = known.id.as_str();
    let failed = live.failed.get(id);
    let state = if stored.paused.contains(id) {
        NearbyDeviceState::Paused
    } else if !live.here.contains(id) {
        NearbyDeviceState::Offline
    } else if live.behind.contains(id) || failed.is_some_and(|f| needs_password(&f.kind)) {
        NearbyDeviceState::NeedsPassword
    } else if live.syncing.contains(id) {
        NearbyDeviceState::Syncing
    } else if failed.is_some_and(|f| f.kind == "clockSkew") {
        NearbyDeviceState::ClockSkew
    } else if failed.is_some() {
        NearbyDeviceState::Error
    } else {
        NearbyDeviceState::UpToDate
    };
    let detail = matches!(
        state,
        NearbyDeviceState::ClockSkew | NearbyDeviceState::Error
    )
    .then_some(failed)
    .flatten();
    NearbyDevice {
        id: known.id.clone(),
        name: known.name.clone(),
        platform: known.platform.clone(),
        state,
        last_synced_at: known.last_synced_ms.and_then(iso),
        detail: detail.map(|f| f.message.clone()),
        detail_kind: detail.map(|f| f.kind.clone()),
    }
}

fn needs_password(kind: &str) -> bool {
    kind == "passwordChangedRemotely" || kind == "vaultUnderRetiredPassword"
}

fn iso(ms: i64) -> Option<String> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|at| at.to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// The platforms the frontend draws. A desktop OS it does not name yet is
/// drawn as Linux, the nearest of them.
fn known_platform(os: &str) -> &'static str {
    match os {
        "macos" => "macos",
        "windows" => "windows",
        "ios" => "ios",
        _ => "linux",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    type Kept<T> = Arc<Mutex<Vec<T>>>;

    /// A book that keeps what it persisted and emitted.
    fn book() -> (Book, Kept<Stored>, Kept<NearbyStatus>) {
        let (kept, shown) = (Kept::<Stored>::default(), Kept::<NearbyStatus>::default());
        let (k, s) = (kept.clone(), shown.clone());
        let book = Book::new(
            Stored::default(),
            "Studio Mac".into(),
            Box::new(move |stored| k.lock().unwrap().push(stored.clone())),
            Box::new(move |status| s.lock().unwrap().push(status)),
        );
        (book, kept, shown)
    }

    fn state(book: &Book, id: &str) -> NearbyDeviceState {
        book.status()
            .devices
            .into_iter()
            .find(|d| d.id == id)
            .unwrap()
            .state
    }

    #[test]
    fn a_row_says_what_matters_most_first() {
        let (book, _, _) = book();
        let generation = book.attach();
        book.seen("d1", "Laptop", "macos");
        assert_eq!(state(&book, "d1"), NearbyDeviceState::Offline);
        book.live(generation, |live| {
            live.here.insert("d1".into());
        });
        assert_eq!(state(&book, "d1"), NearbyDeviceState::UpToDate);
        book.live(generation, |live| {
            live.syncing.insert("d1".into());
        });
        assert_eq!(state(&book, "d1"), NearbyDeviceState::Syncing);
        book.live(generation, |live| {
            live.syncing.clear();
            live.failed.insert(
                "d1".into(),
                Failure {
                    kind: "clockSkew".into(),
                    message: "6 minutes apart".into(),
                },
            );
        });
        let row = book.status().devices.remove(0);
        assert_eq!(row.state, NearbyDeviceState::ClockSkew);
        assert_eq!(row.detail_kind.as_deref(), Some("clockSkew"));
        assert_eq!(row.detail.as_deref(), Some("6 minutes apart"));
        book.live(generation, |live| {
            live.behind.insert("d1".into());
        });
        assert_eq!(state(&book, "d1"), NearbyDeviceState::NeedsPassword);
        assert_eq!(book.status().devices[0].detail, None);
        book.set_paused("d1", true);
        assert_eq!(state(&book, "d1"), NearbyDeviceState::Paused);
    }

    #[test]
    fn a_stopped_manager_changes_nothing_and_leaves_every_device_offline() {
        let (book, _, _) = book();
        let first = book.attach();
        book.seen("d1", "Laptop", "macos");
        book.live(first, |live| {
            live.here.insert("d1".into());
        });
        book.detach(first);
        assert_eq!(state(&book, "d1"), NearbyDeviceState::Offline);
        book.live(first, |live| {
            live.here.insert("d1".into());
        });
        assert_eq!(state(&book, "d1"), NearbyDeviceState::Offline);
        let second = book.attach();
        book.detach(first);
        book.live(second, |live| {
            live.here.insert("d1".into());
        });
        assert_eq!(state(&book, "d1"), NearbyDeviceState::UpToDate);
    }

    #[test]
    fn every_change_is_announced_under_a_higher_seq_and_kept_ones_are_persisted() {
        let (book, kept, shown) = book();
        let generation = book.attach();
        book.set_enabled(true);
        book.seen("d1", "Laptop", "beos");
        book.live(generation, |live| {
            live.here.insert("d1".into());
        });
        book.converged("d1", 1_700_000_000_000);
        let shown = shown.lock().unwrap();
        assert!(shown.windows(2).all(|w| w[0].seq < w[1].seq));
        assert_eq!(book.status().seq, shown.last().unwrap().seq);
        let last = kept.lock().unwrap().last().cloned().unwrap();
        assert!(last.enabled);
        assert_eq!(last.devices[0].platform, "linux");
        assert_eq!(last.devices[0].last_synced_ms, Some(1_700_000_000_000));
        assert_eq!(
            book.status().devices[0].last_synced_at.as_deref(),
            Some("2023-11-14T22:13:20.000Z")
        );
    }

    #[test]
    fn a_removed_device_leaves_the_list_and_stays_removed() {
        let (book, kept, _) = book();
        book.seen("d1", "Laptop", "macos");
        book.set_paused("d1", true);
        assert_eq!(book.remove(&["d1".into(), "d2".into()]), ["d1", "d2"]);
        assert_eq!(book.remove(&["d1".into()]), Vec::<String>::new());
        assert!(book.is_removed("d1") && !book.is_paused("d1"));
        assert!(book.status().devices.is_empty());
        let last = kept.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.removed, ["d1".to_string(), "d2".to_string()].into());
    }

    #[test]
    fn what_is_kept_reads_back_and_an_older_file_reads_as_off() {
        let stored = Stored {
            enabled: true,
            paused: ["a".to_string()].into(),
            removed: ["b".to_string()].into(),
            devices: vec![Known {
                id: "c".into(),
                name: "Laptop".into(),
                platform: "windows".into(),
                last_synced_ms: Some(5),
            }],
        };
        let json = serde_json::to_string(&stored).unwrap();
        assert!(json.contains("\"lastSyncedMs\":5"));
        assert_eq!(serde_json::from_str::<Stored>(&json).unwrap(), stored);
        assert_eq!(
            serde_json::from_str::<Stored>("{}").unwrap(),
            Stored::default()
        );
    }
}
