//! The device book: what nearby sync keeps per install, what is known of each device now, and the status built from both.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use chrono::{SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::error::Error as AppError;

// One count for the process, so a status read before a change is older than the one it announced.
static SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NearbyStatus {
    pub available: bool,
    pub enabled: bool,
    pub device_name: String,
    pub devices: Vec<NearbyDevice>,
    pub seq: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NearbyDevice {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub state: NearbyDeviceState,
    pub last_synced_at: Option<String>,
    pub detail: Option<String>,
    pub detail_kind: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum NearbyDeviceState {
    Paused,
    Offline,
    Syncing,
    Waiting,
    UpToDate,
    PasswordChanged,
    NeedsPassword,
    ClockSkew,
    Error,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Stored {
    pub enabled: bool,
    pub paused: BTreeSet<String>,
    pub removed: BTreeSet<String>,
    pub devices: Vec<Known>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Known {
    pub id: String,
    pub name: String,
    pub platform: String,
    #[serde(default)]
    pub last_synced_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Failure {
    pub kind: String,
    pub message: String,
}

impl From<&AppError> for Failure {
    fn from(e: &AppError) -> Self {
        Self {
            kind: e.kind().to_string(),
            message: e.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Outcome {
    Synced,
    // This device is on a password the other has changed; `Ahead` is the reverse.
    Behind,
    Ahead,
    Failed(Failure),
}

impl Outcome {
    // A connection's: its first that is not a success, else a success only if every lockbox had one.
    pub(super) fn of(lockboxes: Vec<Outcome>, proved: usize) -> Option<Outcome> {
        let complete = lockboxes.len() == proved;
        lockboxes
            .into_iter()
            .find(|outcome| *outcome != Outcome::Synced)
            .or(complete.then_some(Outcome::Synced))
    }
}

#[derive(Default)]
struct Live {
    here: HashSet<String>,
    syncing: HashSet<String>,
    outcomes: HashMap<String, Outcome>,
}

struct Inner {
    stored: Stored,
    live: Live,
    // A stopped manager's threads find their changes dropped.
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
    // `persist` and `emit` run under the book's lock and must not call back into it.
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

    pub fn status(&self) -> NearbyStatus {
        let inner = self.inner.lock().unwrap();
        self.status_of(&inner)
    }

    pub fn enabled(&self) -> bool {
        self.inner.lock().unwrap().stored.enabled
    }

    pub fn refuses(&self, id: &str) -> bool {
        let stored = &self.inner.lock().unwrap().stored;
        stored.removed.contains(id) || stored.paused.contains(id)
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

    pub fn remove(&self, id: &str) {
        self.change(|stored| {
            stored.paused.remove(id);
            stored.removed.insert(id.to_string());
        });
    }

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

    pub(super) fn attach(&self) -> u64 {
        let mut inner = self.inner.lock().unwrap();
        inner.generation += 1;
        inner.live = Live::default();
        self.changed(&mut inner, false);
        inner.generation
    }

    pub(super) fn detach(&self, generation: u64) {
        let mut inner = self.inner.lock().unwrap();
        if inner.generation != generation {
            return;
        }
        inner.generation += 1;
        inner.live = Live::default();
        self.changed(&mut inner, false);
    }

    pub(super) fn here(&self, generation: u64, here: HashSet<String>) {
        self.live(generation, |inner| {
            inner.live.here = here;
            false
        });
    }

    pub(super) fn syncing(&self, generation: u64, id: &str) {
        self.live(generation, |inner| {
            inner.live.syncing.insert(id.to_string());
            false
        });
    }

    // The end of every attempt with `id`; `None` is one that reached no verdict.
    pub(super) fn record(&self, generation: u64, id: &str, outcome: Option<Outcome>, at_ms: i64) {
        self.live(generation, |inner| {
            let synced = outcome == Some(Outcome::Synced);
            if let Some(known) = inner
                .stored
                .devices
                .iter_mut()
                .find(|d| synced && d.id == id)
            {
                known.last_synced_ms = Some(at_ms);
            }
            let live = &mut inner.live;
            live.syncing.remove(id);
            match outcome {
                Some(outcome) => live.outcomes.insert(id.to_string(), outcome),
                None => live.outcomes.remove(id),
            };
            synced
        });
    }

    // `change` says whether it touched what is kept.
    fn live(&self, generation: u64, change: impl FnOnce(&mut Inner) -> bool) {
        let mut inner = self.inner.lock().unwrap();
        if inner.generation == generation {
            let persist = change(&mut inner);
            self.changed(&mut inner, persist);
        }
    }

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

fn device(known: &Known, stored: &Stored, live: &Live) -> NearbyDevice {
    let (state, detail) = state_of(&known.id, stored, live);
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

fn state_of<'a>(
    id: &str,
    stored: &Stored,
    live: &'a Live,
) -> (NearbyDeviceState, Option<&'a Failure>) {
    use NearbyDeviceState::*;
    if stored.paused.contains(id) {
        return (Paused, None);
    }
    if !live.here.contains(id) {
        return (Offline, None);
    }
    if live.syncing.contains(id) {
        return (Syncing, None);
    }
    match live.outcomes.get(id) {
        None => (Waiting, None),
        Some(Outcome::Synced) => (UpToDate, None),
        Some(Outcome::Behind) => (PasswordChanged, None),
        Some(Outcome::Ahead) => (NeedsPassword, None),
        Some(Outcome::Failed(f)) if f.kind == "passwordChangedRemotely" => (PasswordChanged, None),
        Some(Outcome::Failed(f)) if f.kind == "vaultUnderRetiredPassword" => (NeedsPassword, None),
        Some(Outcome::Failed(f)) if f.kind == "clockSkew" => (ClockSkew, Some(f)),
        Some(Outcome::Failed(f)) => (Error, Some(f)),
    }
}

fn iso(ms: i64) -> Option<String> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|at| at.to_rfc3339_opts(SecondsFormat::Millis, true))
}

// The platforms the frontend draws; another desktop OS is drawn as the nearest, Linux.
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
    use super::NearbyDeviceState::*;
    use super::*;
    use std::sync::Arc;

    type Kept<T> = Arc<Mutex<Vec<T>>>;

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

    fn row(book: &Book) -> NearbyDevice {
        book.status().devices.remove(0)
    }

    fn failed(kind: &str) -> Outcome {
        Outcome::Failed(Failure {
            kind: kind.into(),
            message: format!("{kind} happened"),
        })
    }

    fn here(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn the_last_outcome_says_where_a_device_here_stands() {
        let (book, _, _) = book();
        let g = book.attach();
        book.seen("d1", "Laptop", "macos");
        book.here(g, here(&["d1"]));
        assert_eq!(row(&book).state, Waiting);
        for (outcome, state) in [
            (Outcome::Synced, UpToDate),
            (Outcome::Behind, PasswordChanged),
            (Outcome::Ahead, NeedsPassword),
            (failed("passwordChangedRemotely"), PasswordChanged),
            (failed("vaultUnderRetiredPassword"), NeedsPassword),
            (failed("clockSkew"), ClockSkew),
            (failed("vaultFull"), Error),
        ] {
            book.record(g, "d1", Some(outcome), 0);
            assert_eq!(row(&book).state, state);
        }
        let row = row(&book);
        assert_eq!(row.detail_kind.as_deref(), Some("vaultFull"));
        assert_eq!(row.detail.as_deref(), Some("vaultFull happened"));
    }

    #[test]
    fn paused_then_offline_then_syncing_outrank_the_last_outcome() {
        let (book, _, _) = book();
        let g = book.attach();
        book.seen("d1", "Laptop", "macos");
        book.record(g, "d1", Some(Outcome::Behind), 0);
        book.here(g, here(&["d1"]));
        book.syncing(g, "d1");
        assert_eq!(row(&book).state, Syncing);
        book.here(g, here(&[]));
        assert_eq!(row(&book).state, Offline);
        book.set_paused("d1", true);
        assert_eq!(row(&book).state, Paused);
        assert_eq!(row(&book).detail, None);
    }

    #[test]
    fn an_attempt_with_no_verdict_clears_the_outcome_and_keeps_the_last_sync() {
        let (book, kept, _) = book();
        let g = book.attach();
        book.seen("d1", "Laptop", "macos");
        book.here(g, here(&["d1"]));
        book.syncing(g, "d1");
        book.record(g, "d1", Some(Outcome::Synced), 1_700_000_000_000);
        assert_eq!(row(&book).state, UpToDate);
        book.record(g, "d1", None, 1_800_000_000_000);
        assert_eq!(row(&book).state, Waiting);
        assert_eq!(
            row(&book).last_synced_at.as_deref(),
            Some("2023-11-14T22:13:20.000Z")
        );
        let last = kept.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.devices[0].last_synced_ms, Some(1_700_000_000_000));
    }

    #[test]
    fn within_one_connection_a_failure_outranks_a_later_success() {
        assert_eq!(
            Outcome::of(vec![failed("other"), Outcome::Synced], 2),
            Some(failed("other"))
        );
        assert_eq!(
            Outcome::of(
                vec![Outcome::Synced, failed("vaultFull"), failed("other")],
                3
            ),
            Some(failed("vaultFull"))
        );
        assert_eq!(
            Outcome::of(vec![Outcome::Synced, Outcome::Synced], 2),
            Some(Outcome::Synced)
        );
        assert_eq!(Outcome::of(vec![Outcome::Synced], 2), None);
        assert_eq!(Outcome::of(Vec::new(), 1), None);
    }

    #[test]
    fn a_stopped_manager_changes_nothing_and_leaves_every_device_offline() {
        let (book, _, _) = book();
        let first = book.attach();
        book.seen("d1", "Laptop", "macos");
        book.here(first, here(&["d1"]));
        book.detach(first);
        assert_eq!(row(&book).state, Offline);
        book.here(first, here(&["d1"]));
        assert_eq!(row(&book).state, Offline);
        let second = book.attach();
        book.detach(first);
        book.here(second, here(&["d1"]));
        assert_eq!(row(&book).state, Waiting);
    }

    #[test]
    fn every_change_is_announced_under_a_higher_seq_and_kept_ones_are_persisted() {
        let (book, kept, shown) = book();
        let g = book.attach();
        book.set_enabled(true);
        book.seen("d1", "Laptop", "beos");
        book.here(g, here(&["d1"]));
        book.record(g, "d1", Some(Outcome::Synced), 5);
        let shown = shown.lock().unwrap();
        assert!(shown.windows(2).all(|w| w[0].seq < w[1].seq));
        assert_eq!(book.status().seq, shown.last().unwrap().seq);
        let last = kept.lock().unwrap().last().cloned().unwrap();
        assert!(last.enabled);
        assert_eq!(last.devices[0].platform, "linux");
        assert_eq!(last.devices[0].last_synced_ms, Some(5));
    }

    #[test]
    fn a_removed_device_leaves_the_list_and_stays_removed() {
        let (book, kept, _) = book();
        book.seen("d1", "Laptop", "macos");
        book.set_paused("d1", true);
        book.remove("d1");
        book.seen("d1", "Laptop", "macos");
        assert!(book.refuses("d1"));
        assert!(book.status().devices.is_empty());
        let last = kept.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.removed, ["d1".to_string()].into());
        assert!(last.paused.is_empty());
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
