//! Managers in one process, over 127.0.0.1 and the in-memory network, each on
//! lockboxes of the two-devices harness (`commands::password_sync::
//! two_devices`): real SQLCipher vaults, real packs, real TLS.

use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::book::{NearbyDevice, NearbyDeviceState};
use super::*;
use crate::commands::password_sync::two_devices::{derive, params, Device, NEW, OLD};
use crate::error::{Error as AppError, Result as AppResult};
use crate::nearby::channel::{self, Control};
use crate::nearby::discovery::memory::Network;
use crate::nearby::exchange::Absorbed;
use crate::nearby::identity::tests::identity;
use crate::nearby::trust::own_proof;
use crate::retired;
use crate::sync::engine::{LocalVault, Reclaim, Snapshot};

const PERIOD: Duration = Duration::from_millis(500);

/// What the tests count on a manager.
#[derive(Default)]
pub(super) struct Counts {
    dials: AtomicUsize,
    refused: AtomicUsize,
}

impl<H: Host> Shared<H> {
    pub(super) fn count_dial(&self) {
        self.counts.dials.fetch_add(1, Ordering::SeqCst);
    }

    pub(super) fn count_refused(&self) {
        self.counts.refused.fetch_add(1, Ordering::SeqCst);
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

    /// Steps the gate refused.
    fn refused(&self) -> usize {
        self.shared.counts.refused.load(Ordering::SeqCst)
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

type Hook = Box<dyn FnOnce(&TestHost) + Send>;

/// One device's lockboxes, the run claim its steps take, and its clock.
#[derive(Default)]
struct TestHost {
    /// Behind a lock, for a test to change what the device holds mid-session.
    lockboxes: Mutex<Vec<Arc<Held>>>,
    claim: Arc<AtomicBool>,
    offset_ms: i64,
    /// Steps that took the claim.
    steps: AtomicUsize,
    packs: AtomicUsize,
    fail_packs: AtomicBool,
    /// Run inside the next step, under its claim, before the gate.
    next_step: Mutex<Option<Hook>>,
    /// Every merge announced, by vault id.
    announced: Mutex<Vec<String>>,
    passed_on: AtomicUsize,
}

impl TestHost {
    fn on_next_step(&self, hook: impl FnOnce(&TestHost) + Send + 'static) {
        *self.next_step.lock().unwrap() = Some(Box::new(hook));
    }
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

/// A lockbox as a step sees it: the device's, its packs counted.
struct Local<'a> {
    host: &'a TestHost,
    device: &'a Device,
}

impl LocalVault for Local<'_> {
    fn decode(&self, pack_bytes: &[u8]) -> AppResult<Snapshot> {
        self.device.decode(pack_bytes)
    }
    fn merge(&self, incoming: &Snapshot) -> AppResult<usize> {
        self.device.merge(incoming)
    }
    fn digest(&self) -> AppResult<[u8; 32]> {
        LocalVault::digest(self.device)
    }
    fn pack(&self, reclaim: Reclaim) -> AppResult<Vec<u8>> {
        if self.host.fail_packs.load(Ordering::SeqCst) {
            return Err(AppError::VaultFull);
        }
        self.host.packs.fetch_add(1, Ordering::SeqCst);
        self.device.pack(reclaim)
    }
    fn note_push(&self, revision: &str, at_ms: i64) -> AppResult<()> {
        self.device.note_push(revision, at_ms)
    }
    fn vault_id(&self) -> AppResult<Option<String>> {
        self.device.vault_id()
    }
    fn adopt_vault_id(&self, id: &str) -> AppResult<()> {
        self.device.adopt_vault_id(id)
    }
    fn name(&self) -> AppResult<(Option<String>, i64)> {
        self.device.name()
    }
    fn adopt_name(&self, name: &str, at_ms: i64) -> AppResult<()> {
        self.device.adopt_name(name, at_ms)
    }
}

impl Host for TestHost {
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

    fn step<T>(
        &self,
        vault_id: &str,
        step: impl FnOnce(&dyn LocalVault) -> AppResult<T>,
    ) -> AppResult<T> {
        let _claim = claim(&self.claim).ok_or(AppError::SyncBusy)?;
        self.steps.fetch_add(1, Ordering::SeqCst);
        let hook = self.next_step.lock().unwrap().take();
        if let Some(hook) = hook {
            hook(self);
        }
        let held = self
            .lockboxes
            .lock()
            .unwrap()
            .iter()
            .find(|held| held.vault == vault_id)
            .cloned()
            .ok_or(AppError::NotFound)?;
        step(&Local {
            host: self,
            device: &held.device,
        })
    }

    fn pass_on(&self, _vault_id: &str) {
        self.passed_on.fetch_add(1, Ordering::SeqCst);
    }

    fn merged(&self, vault_id: &str, _local: &dyn LocalVault, _absorbed: &Absorbed) {
        self.announced.lock().unwrap().push(vault_id.to_string());
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

fn host(lockboxes: &[&Arc<Held>]) -> TestHost {
    TestHost {
        lockboxes: Mutex::new(lockboxes.iter().map(|held| (*held).clone()).collect()),
        ..TestHost::default()
    }
}

fn node(network: &Network, lockboxes: &[&Arc<Held>]) -> Node {
    node_on(network, host(lockboxes), Stored::default())
}

fn removing(id: String) -> Stored {
    Stored {
        removed: [id].into(),
        ..Stored::default()
    }
}

fn node_on(network: &Network, host: TestHost, stored: Stored) -> Node {
    let host = Arc::new(host);
    let shown = Arc::<Mutex<Vec<NearbyStatus>>>::default();
    let emitted = shown.clone();
    let book = Arc::new(Book::new(
        Stored {
            enabled: true,
            ..stored
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
        period: PERIOD,
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
        self.row_of(&other.id())
    }

    fn row_of(&self, id: &str) -> Option<NearbyDevice> {
        self.book
            .status()
            .devices
            .into_iter()
            .find(|device| device.id == id)
    }

    fn state_of(&self, other: &Node) -> Option<NearbyDeviceState> {
        self.row(other).map(|row| row.state)
    }

    /// Every state this node has shown for the device `id`, in order.
    fn history(&self, id: &str) -> Vec<NearbyDeviceState> {
        self.shown
            .lock()
            .unwrap()
            .iter()
            .filter_map(|status| status.devices.iter().find(|d| d.id == id))
            .map(|row| row.state)
            .collect()
    }

    /// Whether this node shows `other` as here and level with it.
    fn level_with(&self, other: &Node) -> bool {
        self.state_of(other) == Some(NearbyDeviceState::UpToDate)
    }

    fn steps(&self) -> usize {
        self.host.steps.load(Ordering::SeqCst)
    }

    fn announced(&self) -> bool {
        !self.host.announced.lock().unwrap().is_empty()
            && self.host.passed_on.load(Ordering::SeqCst) > 0
    }

    /// Whether this node shows `other` failed with an error of `kind`.
    fn failed_with(&self, other: &Node, kind: &str) -> bool {
        self.row(other).is_some_and(|row| {
            row.state == NearbyDeviceState::Error && row.detail_kind.as_deref() == Some(kind)
        })
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
    eventually("each announced what it merged", || {
        na.announced() && nb.announced()
    });

    a.write("3", "edited on a");
    na.manager.request_runs();
    eventually("a's edit reaches b", || b.title("3").is_some());
    b.write("4", "edited on b");
    nb.manager.request_runs();
    eventually("b's edit reaches a", || a.title("4").is_some());
    eventually("level again", || a.digest() == b.digest());

    // Nothing changed: a sync each way reads both states and moves no pack.
    settle();
    let packs = na.manager.packs() + nb.manager.packs();
    let steps = (na.steps(), nb.steps());
    na.manager.request_runs();
    nb.manager.request_runs();
    eventually("both states read for a sync each way", || {
        na.steps() >= steps.0 + 2 && nb.steps() >= steps.1 + 2
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

    // It dials in anyway, proves its own key for the same vault id, and
    // offers a sync: what it hears is `hello` and `trust/done`, and nothing more.
    let stranger = identity();
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, na.manager.port()));
    let mut dialled = channel::tests::connect(addr, &stranger, None).unwrap();
    dialled.send_control(&Control::hello("Stranger")).unwrap();
    let proof = own_proof(&dialled, &Keys::of(&other.key).proof, "v1");
    for message in [
        Control::Proof {
            vault: "v1".into(),
            proof,
        },
        Control::ProofsDone,
        Control::State {
            vault: "v1".into(),
            revision: "r".into(),
            now_ms: 0,
        },
        Control::Pack {
            vault: "v1".into(),
            revision: "r".into(),
        },
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
fn a_lockbox_shared_after_the_two_met_syncs_unasked() {
    let network = Network::default();
    let (a1, a2, b1) = (
        lockbox("a1", "v1", 1),
        lockbox("a2", "v2", 2),
        lockbox("b1", "v1", 1),
    );
    a2.write("x", "on a, before b held it");
    let (na, nb) = (node(&network, &[&a1, &a2]), node(&network, &[&b1]));
    eventually("level on the one they share", || {
        na.level_with(&nb) && nb.level_with(&na)
    });

    let b2 = lockbox("b2", "v2", 2);
    nb.host.lockboxes.lock().unwrap().push(b2.clone());
    eventually("the lockbox shared later syncs", || {
        b2.title("x").is_some() && a2.digest() == b2.digest()
    });
}

#[test]
fn edits_made_on_both_at_once_cross_though_each_dials_the_other() {
    let network = Network::default();
    let (a, b) = (lockbox("a", "v1", 1), lockbox("b", "v1", 1));
    let (na, nb) = (node(&network, &[&a]), node(&network, &[&b]));
    eventually("level", || na.level_with(&nb) && nb.level_with(&na));

    // Each dials the other at once: the two connections take turns on each
    // side's claim, a step at a time, and both go through.
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

// C dials A again and again; it never gets as far as a step on A, or a row on C.
#[test]
fn a_device_removed_here_is_never_dialled_and_its_dials_end_before_the_handshake() {
    let network = Network::default();
    let (a, c) = (lockbox("a", "v1", 1), lockbox("c", "v1", 1));
    a.write("1", "on a");
    c.write("2", "on c");
    let nc = node(&network, &[&c]);
    let na = node_on(&network, host(&[&a]), removing(nc.id()));

    eventually("c dialled a, and again", || nc.manager.dials() >= 2);
    assert_eq!((na.manager.dials(), na.steps()), (0, 0));
    assert!(nc.row(&na).is_none());
    assert!(a.title("2").is_none() && c.title("1").is_none());
}

// "1" reaches B only from A, so B has met A by then.
#[test]
fn a_removal_stays_on_the_device_that_made_it() {
    let network = Network::default();
    let (a, b, c) = (
        lockbox("a", "v1", 1),
        lockbox("b", "v1", 1),
        lockbox("c", "v1", 1),
    );
    a.write("1", "on a");
    c.write("2", "on c");
    let nc = node(&network, &[&c]);
    let na = node_on(&network, host(&[&a]), removing(nc.id()));
    let nb = node(&network, &[&b]);

    eventually("b syncs with both", || {
        b.title("1").is_some() && b.title("2").is_some() && nb.level_with(&na) && nb.level_with(&nc)
    });
    assert!(!nb.book.refuses(&nc.id()));
}

#[test]
fn a_handshake_carries_nothing_about_removed_devices() {
    let network = Network::default();
    let a = lockbox("a", "v1", 1);
    let removed = removing(identity().device_id().to_string());
    let na = node_on(&network, host(&[&a]), removed);

    let peer = identity();
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, na.manager.port()));
    let mut dialled = channel::tests::connect(addr, &peer, None).unwrap();
    let proof = own_proof(&dialled, &Keys::of(&a.key).proof, "v1");
    for message in [
        Control::hello("Peer"),
        Control::Proof {
            vault: "v1".into(),
            proof,
        },
        Control::ProofsDone,
        Control::State {
            vault: "v1".into(),
            revision: "r".into(),
            now_ms: 0,
        },
    ] {
        dialled.send_control(&message).unwrap();
    }
    let heard: Vec<&str> = (0..4)
        .map(|_| dialled.recv_control().unwrap().kind())
        .collect();
    assert_eq!(
        heard,
        ["hello/hello", "trust/proof", "trust/done", "sync/state"]
    );
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

// Met in an earlier session: its advertisement is seen, but nothing answers at its address.
#[test]
fn a_device_seen_but_never_reached_waits_and_is_never_shown_up_to_date() {
    let network = Network::default();
    let a = lockbox("a", "v1", 1);
    let na = node(&network, &[&a]);
    let ghost = identity();
    let id = ghost.device_id().to_string();
    na.book.seen(&id, "Laptop", "macos");
    let closed = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|listener| listener.local_addr())
        .unwrap()
        .port();
    let txt = advert::txt(ghost.device_id(), [0; 16], &na.host.lockboxes());
    let member = network.join();
    member.advertise(&Advert { port: closed, txt }).unwrap();

    eventually("a dialled it, and again", || na.manager.dials() >= 2);
    let state = na.row_of(&id).map(|row| row.state);
    assert_eq!(state, Some(NearbyDeviceState::Waiting));
    assert!(!na.history(&id).contains(&NearbyDeviceState::UpToDate));
}

// B's run claim is never free, so every connection ends before a verdict, whichever side dialled.
#[test]
fn a_device_whose_syncs_always_end_early_waits_and_is_never_shown_up_to_date() {
    let network = Network::default();
    let (a, b) = (lockbox("a", "v1", 1), lockbox("b", "v1", 1));
    let busy = host(&[&b]);
    let _held = claim(&busy.claim).unwrap();
    let na = node(&network, &[&a]);
    let nb = node_on(&network, busy, Stored::default());

    eventually("each dialled the other, and again", || {
        na.manager.dials() >= 2 && nb.manager.dials() >= 2
    });
    for (node, other) in [(&na, &nb), (&nb, &na)] {
        eventually("waiting", || {
            node.state_of(other) == Some(NearbyDeviceState::Waiting)
        });
        assert!(!node
            .history(&other.id())
            .contains(&NearbyDeviceState::UpToDate));
    }
}

// The hook runs inside A's first step, so after the handshake and before
// anything moved, whichever side dialled.
#[test]
fn no_pack_moves_under_keys_the_connection_did_not_prove() {
    let network = Network::default();
    let (a, b) = (lockbox("a", "v1", 1), lockbox("b", "v1", 1));
    a.write("1", "under the proved key");
    b.write("2", "under the proved key");
    let rekeyed = held(
        "v1",
        Device::create("a2", "v1", "another password", &params(1, 0)),
    );
    rekeyed.write("3", "under the new key");
    let na = node(&network, &[&a]);
    let swapped = rekeyed.clone();
    na.host
        .on_next_step(move |host| *host.lockboxes.lock().unwrap() = vec![swapped]);
    let nb = node(&network, &[&b]);

    eventually("a's gate refuses the step", || na.manager.refused() > 0);
    eventually("each shows the other offline", || {
        na.state_of(&nb) == Some(NearbyDeviceState::Offline)
            && nb.state_of(&na) == Some(NearbyDeviceState::Offline)
    });
    assert_eq!(na.manager.packs() + nb.manager.packs(), 0);
    assert!(a.title("2").is_none() && rekeyed.title("2").is_none());
    assert!(b.title("1").is_none() && b.title("3").is_none());
}

// Removed in the book alone, no socket shut: the gate is what stops it.
#[test]
fn a_device_removed_after_the_handshake_is_sent_no_pack() {
    let network = Network::default();
    let (a, b) = (lockbox("a", "v1", 1), lockbox("b", "v1", 1));
    a.write("1", "only on a");
    let na = node(&network, &[&a]);
    let book = na.book.clone();
    na.host.on_next_step(move |_| {
        for device in book.status().devices {
            book.remove(&device.id);
        }
    });
    let nb = node(&network, &[&b]);

    eventually("a's gate refuses b", || na.manager.refused() > 0);
    assert!(na.book.refuses(&nb.id()));
    assert_eq!(na.manager.packs(), 0);
    assert!(b.title("1").is_none());
}

// B's packs always fail; whichever side dials, the script ends in step with
// B's own error on B and, with none of its own, "could not finish" on A.
#[test]
fn each_side_shows_its_own_error_and_the_other_that_the_sync_did_not_finish() {
    let network = Network::default();
    let (a, b) = (lockbox("a", "v1", 1), lockbox("b", "v1", 1));
    a.write("1", "on a");
    b.write("2", "on b");
    let nb = node(&network, &[&b]);
    nb.host.fail_packs.store(true, Ordering::SeqCst);
    let na = node(&network, &[&a]);

    eventually("b shows its own error, a that it did not finish", || {
        nb.failed_with(&na, "vaultFull") && na.failed_with(&nb, "other")
    });
    assert!(a.title("2").is_none());
}

// B cannot pack: v1, where it holds a row A lacks, always fails; v2 syncs after it.
#[test]
fn a_failure_on_one_lockbox_is_shown_though_the_next_one_synced() {
    let network = Network::default();
    let (a1, a2) = (lockbox("a1", "v1", 1), lockbox("a2", "v2", 2));
    let (b1, b2) = (lockbox("b1", "v1", 1), lockbox("b2", "v2", 2));
    b1.write("1", "only b can send this");
    a2.write("2", "a sends this");
    let nb = node(&network, &[&b1, &b2]);
    nb.host.fail_packs.store(true, Ordering::SeqCst);
    let na = node(&network, &[&a1, &a2]);

    eventually("v2 is level while both show v1's failure", || {
        a2.digest() == b2.digest()
            && nb.failed_with(&na, "vaultFull")
            && na.failed_with(&nb, "other")
    });
    assert!(b2.title("2").is_some() && a1.title("1").is_none());
    for (node, other) in [(&na, &nb), (&nb, &na)] {
        assert!(!node
            .history(&other.id())
            .contains(&NearbyDeviceState::UpToDate));
    }
}

// Each side dials once on first sight; every dial after that is the timer's.
#[test]
fn a_sync_that_failed_is_tried_again_at_the_next_period() {
    let network = Network::default();
    let (a, b) = (lockbox("a", "v1", 1), lockbox("b", "v1", 1));
    a.write("1", "on a");
    b.write("2", "on b");
    let nb = node(&network, &[&b]);
    nb.host.fail_packs.store(true, Ordering::SeqCst);
    let na = node(&network, &[&a]);

    // Each dials once on first sight; a third dial is a later period's.
    eventually("one of them dialled again after a failed sync", || {
        nb.failed_with(&na, "vaultFull") && na.manager.dials() + nb.manager.dials() >= 3
    });
    assert!(a.title("2").is_none());
    nb.host.fail_packs.store(false, Ordering::SeqCst);
    eventually("a later period's sync goes through", || {
        a.title("2").is_some() && a.digest() == b.digest()
    });
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
    let skewed_host = TestHost {
        offset_ms: 6 * 60 * 1000 - 30_000,
        ..host(&[&b])
    };
    let (na, nb) = (
        node(&network, &[&a]),
        node_on(&network, skewed_host, Stored::default()),
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

    // The answering side's claim held: its step waits for it, or the
    // connection ends and is tried again, until the sync goes through.
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
    let states = na.history(&nb.id());
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
