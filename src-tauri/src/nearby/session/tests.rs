//! Managers in one process, over 127.0.0.1 and the in-memory network, each on
//! lockboxes of the two-devices harness (`commands::password_sync::
//! two_devices`): real SQLCipher vaults, real packs, real TLS.

use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::book::{NearbyDevice, NearbyDeviceState};
use super::*;
use crate::commands::password_sync::two_devices::{derive, params, Device, NEW, OLD};
use crate::error::{Error as AppError, Result as AppResult};
use crate::nearby::channel::{self, Control};
use crate::nearby::discovery::memory::Network;
use crate::nearby::exchange::PeerOutcome;
use crate::nearby::identity::tests::identity;
use crate::nearby::trust::own_proof;
use crate::retired;
use crate::sync::engine::{LocalVault, Reclaim, Snapshot};

/// What the tests count on a manager.
#[derive(Default)]
pub(super) struct Counts {
    dials: AtomicUsize,
}

impl<H: Host> Shared<H> {
    pub(super) fn count_dial(&self) {
        self.counts.dials.fetch_add(1, Ordering::SeqCst);
    }
}

impl<H: Host> Manager<H> {
    fn port(&self) -> u16 {
        self.shared.port
    }

    fn id(&self) -> DeviceId {
        *self.shared.identity.device_id()
    }

    fn dials(&self) -> usize {
        self.shared.counts.dials.load(Ordering::SeqCst)
    }
}

impl Manager<TestHost> {
    /// Packs this manager made to send, either way.
    fn packs(&self) -> usize {
        self.shared.host.packs.load(Ordering::SeqCst)
    }
}

/// A lockbox on one device, and the vault id it was made under.
struct Held {
    vault: &'static str,
    device: Device,
}

impl std::ops::Deref for Held {
    type Target = Device;
    fn deref(&self) -> &Device {
        &self.device
    }
}

/// One device's lockboxes, the run claim its runs take, and its clock.
struct TestHost {
    /// Behind a lock, for a test to change what the device holds mid-session.
    lockboxes: Mutex<Vec<Arc<Held>>>,
    claim: Arc<AtomicBool>,
    offset_ms: i64,
    /// Every run that ended here, by vault id.
    finished: Mutex<Vec<(String, PeerOutcome)>>,
    /// Packs its runs made.
    packs: Arc<AtomicUsize>,
}

/// The test host's run claim, as the app's is: one flag, raised while held.
struct Claim(Arc<AtomicBool>);

impl Drop for Claim {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn claim(flag: &Arc<AtomicBool>) -> Option<Claim> {
    flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .ok()
        .map(|_| Claim(flag.clone()))
}

struct TestRun {
    held: Arc<Held>,
    packs: Arc<AtomicUsize>,
    _claim: Claim,
}

impl Deref for TestRun {
    type Target = dyn LocalVault;

    fn deref(&self) -> &Self::Target {
        self
    }
}

impl LocalVault for TestRun {
    fn decode(&self, pack_bytes: &[u8]) -> AppResult<Snapshot> {
        self.held.device.decode(pack_bytes)
    }
    fn merge(&self, incoming: &Snapshot) -> AppResult<usize> {
        self.held.device.merge(incoming)
    }
    fn digest(&self) -> AppResult<[u8; 32]> {
        LocalVault::digest(&self.held.device)
    }
    fn pack(&self, reclaim: Reclaim) -> AppResult<Vec<u8>> {
        self.packs.fetch_add(1, Ordering::SeqCst);
        self.held.device.pack(reclaim)
    }
    fn note_push(&self, revision: &str, at_ms: i64) -> AppResult<()> {
        self.held.device.note_push(revision, at_ms)
    }
    fn vault_id(&self) -> AppResult<Option<String>> {
        self.held.device.vault_id()
    }
    fn adopt_vault_id(&self, id: &str) -> AppResult<()> {
        self.held.device.adopt_vault_id(id)
    }
    fn name(&self) -> AppResult<(Option<String>, i64)> {
        self.held.device.name()
    }
    fn adopt_name(&self, name: &str, at_ms: i64) -> AppResult<()> {
        self.held.device.adopt_name(name, at_ms)
    }
}

impl Host for TestHost {
    type Run = TestRun;

    fn lockboxes(&self) -> Vec<Lockbox> {
        self.lockboxes
            .lock()
            .unwrap()
            .iter()
            .map(|device| Lockbox {
                vault_id: device.vault.to_string(),
                current: Keys::of(&device.key),
                retired: retired::load(device.path(), &device.key, device.id)
                    .iter()
                    .map(|epoch| Keys::of(&epoch.key()))
                    .collect(),
            })
            .collect()
    }

    fn begin(&self, vault_id: &str) -> AppResult<TestRun> {
        let claim = claim(&self.claim).ok_or(AppError::SyncBusy)?;
        let held = self
            .lockboxes
            .lock()
            .unwrap()
            .iter()
            .find(|held| held.vault == vault_id)
            .cloned()
            .ok_or(AppError::NotFound)?;
        Ok(TestRun {
            held,
            packs: self.packs.clone(),
            _claim: claim,
        })
    }

    fn finish(&self, run: TestRun, outcome: PeerOutcome) {
        let vault = run.held.vault.to_string();
        drop(run);
        self.finished.lock().unwrap().push((vault, outcome));
    }

    fn now_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis() + self.offset_ms
    }
}

/// One device on the network: its host, its book and what the book showed.
struct Node {
    host: Arc<TestHost>,
    book: Arc<Book>,
    shown: Arc<Mutex<Vec<NearbyStatus>>>,
    manager: Manager<TestHost>,
}

fn node(network: &Network, lockboxes: &[&Arc<Held>]) -> Node {
    node_at(network, lockboxes, 0)
}

/// A node whose clock is `offset_ms` off the others'.
fn node_at(network: &Network, lockboxes: &[&Arc<Held>], offset_ms: i64) -> Node {
    let host = Arc::new(TestHost {
        lockboxes: Mutex::new(lockboxes.iter().map(|held| (*held).clone()).collect()),
        claim: Arc::default(),
        offset_ms,
        finished: Mutex::default(),
        packs: Arc::default(),
    });
    let shown = Arc::<Mutex<Vec<NearbyStatus>>>::default();
    let emitted = shown.clone();
    let book = Arc::new(Book::new(
        Stored {
            enabled: true,
            ..Stored::default()
        },
        "Test device".into(),
        Box::new(|_| {}),
        Box::new(move |status| emitted.lock().unwrap().push(status)),
    ));
    let manager = Manager::start(Config {
        host: host.clone(),
        identity: identity(),
        discovery: Box::new(network.join()),
        book: book.clone(),
        listen: Ipv4Addr::LOCALHOST.into(),
    })
    .unwrap();
    Node {
        host,
        book,
        shown,
        manager,
    }
}

impl Node {
    fn id(&self) -> String {
        self.manager.id().to_string()
    }

    /// This node's row for `other`.
    fn row(&self, other: &Node) -> Option<NearbyDevice> {
        self.book
            .status()
            .devices
            .into_iter()
            .find(|device| device.id == other.id())
    }

    fn state_of(&self, other: &Node) -> Option<NearbyDeviceState> {
        self.row(other).map(|row| row.state)
    }

    /// Whether this node shows `other` as here and level with it.
    fn level_with(&self, other: &Node) -> bool {
        self.state_of(other) == Some(NearbyDeviceState::UpToDate)
    }

    fn runs_ended(&self) -> usize {
        self.host.finished.lock().unwrap().len()
    }
}

fn lockbox(id: &'static str, vault: &'static str, salt: u8) -> Arc<Held> {
    held(vault, Device::create(id, vault, OLD, &params(salt, 0)))
}

fn held(vault: &'static str, device: Device) -> Arc<Held> {
    Arc::new(Held { vault, device })
}

/// Wait for `done`, up to a deadline long enough for a slow machine.
fn eventually(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `node`'s run claim, once whatever run holds it lets go.
fn hold(node: &Node) -> Claim {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(claim) = claim(&node.host.claim) {
            return claim;
        }
        assert!(Instant::now() < deadline, "the claim never came free");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Long enough for anything that was going to happen to have happened.
fn settle() {
    std::thread::sleep(Duration::from_millis(1500));
}

#[test]
fn two_devices_holding_one_lockbox_converge_and_each_edit_reaches_the_other() {
    let network = Network::default();
    let (a, b) = (lockbox("a", "v1", 1), lockbox("b", "v1", 1));
    a.write("1", "made on a");
    b.write("2", "made on b");
    let (na, nb) = (node(&network, &[&a]), node(&network, &[&b]));

    eventually("the two converge", || {
        a.title("2").is_some() && b.title("1").is_some() && a.digest() == b.digest()
    });
    eventually("both show the other up to date", || {
        na.state_of(&nb) == Some(NearbyDeviceState::UpToDate)
            && nb.state_of(&na) == Some(NearbyDeviceState::UpToDate)
    });

    a.write("3", "edited on a");
    na.manager.request_runs();
    eventually("a's edit reaches b", || b.title("3").is_some());
    b.write("4", "edited on b");
    nb.manager.request_runs();
    eventually("b's edit reaches a", || a.title("4").is_some());
    eventually("level again", || a.digest() == b.digest());

    // Nothing changed: a run each way moves no pack.
    settle();
    let packs = na.manager.packs() + nb.manager.packs();
    let ended = (na.runs_ended(), nb.runs_ended());
    na.manager.request_runs();
    nb.manager.request_runs();
    eventually("a run ends on both", || {
        na.runs_ended() > ended.0 && nb.runs_ended() > ended.1
    });
    settle();
    assert_eq!(na.manager.packs() + nb.manager.packs(), packs);
}

#[test]
fn a_device_on_another_key_is_never_dialled_and_is_shown_nothing() {
    let network = Network::default();
    let a = lockbox("a", "v1", 1);
    let other = held(
        "v1",
        Device::create("c", "v1", "another password", &params(1, 0)),
    );
    a.write("1", "secret");
    let (na, nc) = (node(&network, &[&a]), node(&network, &[&other]));
    settle();
    assert_eq!((na.manager.dials(), nc.manager.dials()), (0, 0));
    assert!(na.book.status().devices.is_empty());

    // It dials in anyway, proves its own key for the same vault id, and asks
    // for a run: what it hears is `hello` and `trust/done`, and nothing more.
    let stranger = identity();
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, na.manager.port()));
    let mut dialled = channel::connect(addr, &stranger, None).unwrap();
    dialled.send_control(&Control::hello("Stranger")).unwrap();
    let proof = own_proof(&dialled, &Keys::of(&other.key).proof, "v1");
    for message in [
        Control::Proof {
            vault: "v1".into(),
            proof,
        },
        Control::ProofsDone,
        Control::Head {
            vault: "v1".into(),
            now_ms: 0,
        },
        Control::Fetch { vault: "v1".into() },
    ] {
        if dialled.send_control(&message).is_err() {
            break;
        }
    }
    let mut heard = Vec::new();
    while let Ok(message) = dialled.recv_control() {
        heard.push(message);
    }
    assert!(matches!(heard[0], Control::Hello { .. }), "{heard:?}");
    assert_eq!(&heard[1..], [Control::ProofsDone]);
    assert_eq!(na.manager.packs(), 0);
    assert!(na.book.status().devices.is_empty());
}

#[test]
fn two_shared_lockboxes_sync_each_into_its_own_and_one_held_alone_is_untouched() {
    let network = Network::default();
    let (a1, a2, a3) = (
        lockbox("a1", "v1", 1),
        lockbox("a2", "v2", 2),
        lockbox("a3", "v3", 3),
    );
    let (b1, b2) = (lockbox("b1", "v1", 1), lockbox("b2", "v2", 2));
    a1.write("1", "one on a");
    b1.write("2", "one on b");
    a2.write("x", "two on a");
    b2.write("y", "two on b");
    a3.write("z", "three, only on a");
    let alone = a3.digest();
    let (_na, _nb) = (
        node(&network, &[&a1, &a2, &a3]),
        node(&network, &[&b1, &b2]),
    );

    eventually("both lockboxes converge", || {
        a1.digest() == b1.digest()
            && a2.digest() == b2.digest()
            && b1.title("1").is_some()
            && b2.title("x").is_some()
    });
    for one in [&a1, &b1] {
        assert!(one.title("1").is_some() && one.title("2").is_some());
        assert!(one.title("x").is_none() && one.title("y").is_none());
    }
    for two in [&a2, &b2] {
        assert!(two.title("x").is_some() && two.title("y").is_some());
        assert!(two.title("1").is_none() && two.title("z").is_none());
    }
    assert_eq!(b1.device.vault_id().unwrap().as_deref(), Some("v1"));
    assert_eq!(b2.device.vault_id().unwrap().as_deref(), Some("v2"));
    assert_eq!(a3.digest(), alone);
}

#[test]
fn edits_made_on_both_at_once_cross_though_each_dials_the_other() {
    let network = Network::default();
    let (a, b) = (lockbox("a", "v1", 1), lockbox("b", "v1", 1));
    let (na, nb) = (node(&network, &[&a]), node(&network, &[&b]));
    eventually("level", || na.level_with(&nb) && nb.level_with(&na));

    // Each dials the other and finds its claim held by the other's own dial:
    // both try again, apart, and the first through carries both edits.
    for round in ["1", "2", "3"] {
        a.write(&format!("a{round}"), "made on a");
        b.write(&format!("b{round}"), "made on b");
        na.manager.request_runs();
        nb.manager.request_runs();
        eventually("both edits cross", || {
            b.title(&format!("a{round}")).is_some() && a.title(&format!("b{round}")).is_some()
        });
    }
    eventually("level again", || a.digest() == b.digest());
}

#[test]
fn pause_stops_runs_with_that_device_and_resume_starts_them_again() {
    let network = Network::default();
    let (a, b) = (lockbox("a", "v1", 1), lockbox("b", "v1", 1));
    let (na, nb) = (node(&network, &[&a]), node(&network, &[&b]));
    eventually("connected", || {
        na.state_of(&nb) == Some(NearbyDeviceState::UpToDate)
            && nb.state_of(&na) == Some(NearbyDeviceState::UpToDate)
    });

    na.manager.set_paused(&nb.id(), true);
    assert_eq!(na.state_of(&nb), Some(NearbyDeviceState::Paused));
    a.write("p", "while paused, on a");
    na.manager.request_runs();
    b.write("q", "while paused, on b");
    nb.manager.request_runs();
    settle();
    assert!(b.title("p").is_none() && a.title("q").is_none());

    na.manager.set_paused(&nb.id(), false);
    eventually("both edits cross", || {
        b.title("p").is_some() && a.title("q").is_some()
    });
    eventually("up to date again", || {
        na.state_of(&nb) == Some(NearbyDeviceState::UpToDate)
    });
}

#[test]
fn a_removed_device_is_refused_by_every_device_the_removal_reaches() {
    let network = Network::default();
    let (a, b, c) = (
        lockbox("a", "v1", 1),
        lockbox("b", "v1", 1),
        lockbox("c", "v1", 1),
    );
    c.write("1", "from c, before");
    let (na, nb, nc) = (
        node(&network, &[&a]),
        node(&network, &[&b]),
        node(&network, &[&c]),
    );
    eventually("all three converge", || {
        a.title("1").is_some() && b.title("1").is_some() && na.row(&nc).is_some()
    });

    na.manager.remove(&nc.id());
    assert!(na.row(&nc).is_none());
    eventually("b learns it from a", || nb.book.is_removed(&nc.id()));

    // Nothing crosses between C and the other two from now on, whichever
    // side has something to say; A and B go on as before.
    c.write("2", "from c, after");
    a.write("3", "from a, after");
    for node in [&na, &nb, &nc] {
        node.manager.request_runs();
    }
    eventually("a's edit reaches b", || b.title("3").is_some());
    settle();
    assert!(a.title("2").is_none() && b.title("2").is_none());
    assert!(c.title("3").is_none());
}

#[test]
fn a_long_removed_set_reaches_the_other_device_whole() {
    let network = Network::default();
    let (a, b) = (lockbox("a", "v1", 1), lockbox("b", "v1", 1));
    let na = node(&network, &[&a]);
    let ids: Vec<String> = (0..600).map(|i| format!("{i:064x}")).collect();
    na.book.remove(&ids);
    let nb = node(&network, &[&b]);
    eventually("b holds every id", || nb.book.removed().len() == ids.len());
}

#[test]
fn a_device_whose_shared_lockbox_went_is_shown_offline() {
    let network = Network::default();
    let (a, b) = (lockbox("a", "v1", 1), lockbox("b", "v1", 1));
    let (na, nb) = (node(&network, &[&a]), node(&network, &[&b]));
    eventually("connected", || {
        na.state_of(&nb) == Some(NearbyDeviceState::UpToDate)
            && nb.state_of(&na) == Some(NearbyDeviceState::UpToDate)
    });

    // A no longer holds the lockbox: neither recognises the other's
    // advertisement any more.
    na.host.lockboxes.lock().unwrap().clear();
    eventually("each shows the other offline", || {
        na.state_of(&nb) == Some(NearbyDeviceState::Offline)
            && nb.state_of(&na) == Some(NearbyDeviceState::Offline)
    });
}

#[test]
fn no_run_is_begun_under_keys_its_connection_did_not_prove() {
    let network = Network::default();
    let (a, b) = (lockbox("a", "v1", 1), lockbox("b", "v1", 1));
    let (na, nb) = (node(&network, &[&a]), node(&network, &[&b]));
    eventually("connected", || {
        na.state_of(&nb) == Some(NearbyDeviceState::UpToDate)
            && nb.state_of(&na) == Some(NearbyDeviceState::UpToDate)
    });
    settle();
    let packs = na.manager.packs() + nb.manager.packs();

    // A's lockbox is under another key, as after a password change, and a
    // sync is asked for on both before A's manager has looked at the keys
    // again: whichever side dials, no pack moves.
    let rekeyed = held(
        "v1",
        Device::create("a2", "v1", "another password", &params(1, 0)),
    );
    rekeyed.write("1", "under the new key");
    b.write("2", "under the old key");
    *na.host.lockboxes.lock().unwrap() = vec![rekeyed.clone()];
    na.manager.request_runs();
    nb.manager.request_runs();
    eventually("each shows the other offline", || {
        na.state_of(&nb) == Some(NearbyDeviceState::Offline)
            && nb.state_of(&na) == Some(NearbyDeviceState::Offline)
    });
    settle();
    assert_eq!(na.manager.packs() + nb.manager.packs(), packs);
    assert!(rekeyed.title("2").is_none() && b.title("1").is_none());
}

#[test]
fn a_device_on_a_retired_key_needs_the_password_on_both_sides_and_gets_no_pack() {
    let network = Network::default();
    let mut moved_on = Device::create("a", "v1", OLD, &params(1, 0));
    moved_on.write("1", "made before the change");
    let gen1 = params(2, 1);
    moved_on.rekey(derive(NEW, &gen1), &gen1);
    moved_on.write("2", "made after the change");
    let (a, b) = (held("v1", moved_on), lockbox("b", "v1", 1));
    b.write("3", "made on the old password");
    let before = (a.digest(), b.digest());
    let (na, nb) = (node(&network, &[&a]), node(&network, &[&b]));

    eventually("both need the password", || {
        na.state_of(&nb) == Some(NearbyDeviceState::NeedsPassword)
            && nb.state_of(&na) == Some(NearbyDeviceState::NeedsPassword)
    });
    na.manager.request_runs();
    nb.manager.request_runs();
    settle();
    assert_eq!(na.manager.packs() + nb.manager.packs(), 0);
    assert_eq!((a.digest(), b.digest()), before);
    assert!(b.title("1").is_none() && a.title("3").is_none());
}

#[test]
fn clocks_six_minutes_apart_stop_the_run_and_nothing_merges() {
    let network = Network::default();
    let (a, b) = (lockbox("a", "v1", 1), lockbox("b", "v1", 1));
    a.write("1", "on a");
    b.write("2", "on b");
    let before = (a.digest(), b.digest());
    // Half a minute short of six, so the difference each side measures (this
    // offset, give or take the milliseconds between the two readings) rounds
    // up to six whichever of them dials.
    let (na, nb) = (
        node(&network, &[&a]),
        node_at(&network, &[&b], 6 * 60 * 1000 - 30_000),
    );
    let skewed = |n: &Node, other: &Node| {
        n.row(other).is_some_and(|row| {
            row.state == NearbyDeviceState::ClockSkew
                && row.detail_kind.as_deref() == Some("clockSkew")
                && row
                    .detail
                    .as_deref()
                    .is_some_and(|d| d.contains("6 minutes"))
        })
    };
    eventually("both show the skew", || {
        skewed(&na, &nb) && skewed(&nb, &na)
    });
    assert_eq!((a.digest(), b.digest()), before);
    assert_eq!(na.manager.packs() + nb.manager.packs(), 0);
}

#[test]
fn a_run_asked_for_while_the_claim_is_held_runs_once_it_is_released() {
    let network = Network::default();
    let (a, b) = (lockbox("a", "v1", 1), lockbox("b", "v1", 1));
    let (na, nb) = (node(&network, &[&a]), node(&network, &[&b]));
    eventually("connected", || {
        na.state_of(&nb) == Some(NearbyDeviceState::UpToDate)
            && nb.state_of(&na) == Some(NearbyDeviceState::UpToDate)
    });

    // The answering side's claim held: it answers `sync/busy`, and the sync
    // is tried again until it goes through.
    let held = hold(&nb);
    a.write("1", "while b was busy");
    na.manager.request_runs();
    settle();
    assert!(b.title("1").is_none());
    drop(held);
    eventually("it arrives once released", || b.title("1").is_some());

    // The dialling side's own claim held: the same.
    let held = hold(&na);
    a.write("2", "while a was busy");
    na.manager.request_runs();
    settle();
    assert!(b.title("2").is_none());
    drop(held);
    eventually("it arrives once released", || b.title("2").is_some());
}

#[test]
fn stopping_closes_the_listener_and_every_connection_and_takes_the_advert_down() {
    let network = Network::default();
    let (a, b) = (lockbox("a", "v1", 1), lockbox("b", "v1", 1));
    let (na, nb) = (node(&network, &[&a]), node(&network, &[&b]));
    eventually("level", || na.level_with(&nb) && nb.level_with(&na));
    let port = na.manager.port();
    assert!(network.advertised().contains(&port));

    na.manager.stop();
    assert!(!network.advertised().contains(&port));
    eventually("the listener is closed", || {
        TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_err()
    });
    eventually("b sees a go", || {
        nb.state_of(&na) == Some(NearbyDeviceState::Offline)
    });
    assert_eq!(na.state_of(&nb), Some(NearbyDeviceState::Offline));
    settle();
    assert!(!network.advertised().contains(&port));
}

#[test]
fn the_status_moves_forward_and_says_what_happened() {
    let network = Network::default();
    let (a, b) = (lockbox("a", "v1", 1), lockbox("b", "v1", 1));
    a.write("1", "on a");
    let (na, nb) = (node(&network, &[&a]), node(&network, &[&b]));
    eventually("synced", || {
        b.title("1").is_some()
            && na.row(&nb).is_some_and(|row| {
                row.state == NearbyDeviceState::UpToDate && row.last_synced_at.is_some()
            })
    });
    let shown = na.shown.lock().unwrap().clone();
    assert!(shown.windows(2).all(|w| w[0].seq < w[1].seq));
    let states: Vec<NearbyDeviceState> = shown
        .iter()
        .filter_map(|status| status.devices.iter().find(|d| d.id == nb.id()))
        .map(|row| row.state)
        .collect();
    let syncing = states
        .iter()
        .position(|s| *s == NearbyDeviceState::Syncing)
        .expect("a run was shown");
    assert!(states[syncing..].contains(&NearbyDeviceState::UpToDate));

    nb.manager.stop();
    eventually("offline, with the time it last synced", || {
        na.row(&nb).is_some_and(|row| {
            row.state == NearbyDeviceState::Offline && row.last_synced_at.is_some()
        })
    });
}
