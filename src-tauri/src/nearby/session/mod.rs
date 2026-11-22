//! The session manager (design §5, "Discovery and sessions"): while the app
//! is unlocked and nearby sync is on, this device advertises itself, listens,
//! and syncs with the devices it shares a lockbox with.
//!
//! A sync is one short connection: the device with something to say dials,
//! the two prove their keys, every lockbox they hold under the same key is
//! run through `exchange`, and the connection closes. Nothing outlives it but
//! what the book shows, so there is no connection to keep in step with the
//! keys, the removed set or the network.
//!
//! What is kept per device seen is when a sync with it is owed
//! ([`Seen::due`]): on first sight, when a lockbox changes here (an edit, a
//! merge from the account or from another device), and again after a try
//! that could not go through.
//!
//! - [`advert`]: what this device's advertisement says, and what another's
//!   says to it.
//! - [`listener`] and [`connection`]: one listener and one thread per
//!   connection; [`conns`] keeps their count and their deadlines.
//! - [`handshake`]: `hello`, the key proofs, the removed sets.
//! - [`sync`]: the lockboxes' sync, the same script on either side.
//! - [`book`]: what is kept per install and what is known now, and the
//!   status built from them.
//!
//! Blocking I/O on plain threads, as the channel is; no async runtime. The
//! lockboxes, the run claim and the clock are the [`Host`]'s, so the whole of
//! it runs in tests as two managers in one process.

mod advert;
pub mod book;
mod connection;
mod conns;
mod handshake;
pub mod host;
mod listener;
mod sync;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rand::RngCore;

use self::advert::Recognised;
use self::connection::Dialled;
use self::conns::Conns;
use crate::nearby::discovery::{Advert, Discovery, Event, Txt};
use crate::nearby::identity::{DeviceId, Identity};
use crate::nearby::trust::NONCE_LEN;
use crate::nearby::{Error, Result};

pub use book::{Book, NearbyStatus, Stored};
pub use host::{Host, Keys, Lockbox};

/// How often the manager looks for a sync that is due and a handshake that
/// is overdue.
const TICK: Duration = Duration::from_millis(200);

/// How often the lockboxes are read again. The unlock puts every lockbox but
/// the open one into the keyring a moment after the session opens, and a
/// password change or a new lockbox changes the keys; nothing announces
/// either.
const REFRESH: Duration = Duration::from_secs(3);

/// The first wait before a sync that could not go through is tried again,
/// doubling up to the second. A device that paused this one ends the
/// connection, so the ceiling is also how often a paused device is asked.
const RETRY_FIRST: Duration = Duration::from_millis(250);
const RETRY_MOST: Duration = Duration::from_secs(30);

/// How long the device with the larger id waits on first sight. Whichever
/// sees the other dials; the smaller dials at once, and by then the two are
/// usually level.
const SECOND_DIAL: Duration = Duration::from_secs(2);

/// What a manager runs with.
pub struct Config<H: Host> {
    pub host: Arc<H>,
    pub identity: Identity,
    pub discovery: Box<dyn Discovery>,
    pub book: Arc<Book>,
    /// The address the listener binds: every interface for the app, this
    /// machine's loopback for tests.
    pub listen: IpAddr,
}

/// A running manager. Stopped by [`Manager::stop`] or by being dropped.
pub struct Manager<H: Host> {
    shared: Arc<Shared<H>>,
}

/// What the manager's threads share.
struct Shared<H: Host> {
    host: Arc<H>,
    identity: Identity,
    discovery: Box<dyn Discovery>,
    book: Arc<Book>,
    /// This manager's generation in the book (`Book::attach`).
    generation: u64,
    stopped: AtomicBool,
    port: u16,
    /// This session's advertisement nonce.
    nonce: [u8; NONCE_LEN],
    lockboxes: Mutex<Arc<Vec<Lockbox>>>,
    conns: Mutex<Conns>,
    /// Every advertisement seen and not gone, by instance name.
    seen: Mutex<HashMap<String, Seen>>,
    #[cfg(test)]
    counts: tests::Counts,
}

/// An advertisement on the network.
struct Seen {
    txt: Txt,
    addrs: Vec<SocketAddr>,
    /// The device behind it and the lockboxes it shares with this one, as
    /// read with the lockboxes last read. `None` for a stranger's.
    found: Option<Recognised>,
    /// When a sync with it is owed. `None` once one went through.
    due: Option<Instant>,
    /// The wait before the next try, should this one not go through.
    backoff: Duration,
    dialling: bool,
}

impl<H: Host> Manager<H> {
    /// Listen, advertise and browse. Fails when the listener cannot be bound;
    /// discovery that fails is logged, and the manager still answers.
    pub fn start(config: Config<H>) -> Result<Self> {
        let listener = TcpListener::bind((config.listen, 0)).map_err(Error::Io)?;
        let port = listener.local_addr().map_err(Error::Io)?.port();
        let mut nonce = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce);
        let lockboxes = config.host.lockboxes();
        let shared = Arc::new(Shared {
            generation: config.book.attach(),
            host: config.host,
            identity: config.identity,
            discovery: config.discovery,
            book: config.book,
            stopped: AtomicBool::new(false),
            port,
            nonce,
            lockboxes: Mutex::new(Arc::new(lockboxes)),
            conns: Mutex::default(),
            seen: Mutex::default(),
            #[cfg(test)]
            counts: tests::Counts::default(),
        });
        shared.advertise();
        // Weak: the discovery is the manager's, and the manager is not its.
        let weak = Arc::downgrade(&shared);
        let browsed = shared.discovery.browse(Box::new(move |event| {
            if let Some(shared) = weak.upgrade() {
                shared.on_event(event);
            }
        }));
        if let Err(e) = browsed {
            log::warn!("nearby: not browsing for other devices: {e}");
        }
        spawn("nearby-listen", {
            let shared = shared.clone();
            move || listener::serve(shared, listener)
        })?;
        spawn("nearby-tick", {
            let shared = shared.clone();
            move || shared.tick()
        })?;
        log::info!(
            "nearby: started on port {port} with {} lockboxes",
            shared.lockboxes().len()
        );
        Ok(Self { shared })
    }

    /// Take the advertisement down and close the listener and every
    /// connection. Returns at once: the threads end at their next look at
    /// the stop flag or their next read, and a run in flight ends with them.
    pub fn stop(&self) {
        self.shared.stop();
    }

    /// This device's lockboxes changed: a sync with every device here is
    /// owed.
    pub fn request_runs(&self) {
        self.shared.owe_all();
    }

    /// Stop syncing with `device_id`, or start again; a resume syncs at once.
    pub fn set_paused(&self, device_id: &str, paused: bool) {
        self.shared.book.set_paused(device_id, paused);
        if !paused {
            self.shared.owe_all();
        }
    }

    /// Refuse `device_id` from now on: see [`Shared::learn_removed`].
    pub fn remove(&self, device_id: &str) {
        self.shared.learn_removed(&[device_id.to_string()]);
    }
}

impl<H: Host> Drop for Manager<H> {
    fn drop(&mut self) {
        self.shared.stop();
    }
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) -> Result<()> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(work)
        .map(drop)
        .map_err(Error::Io)
}

impl<H: Host> Shared<H> {
    fn stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    fn stop(&self) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        log::info!("nearby: stopped");
        self.discovery.stop();
        self.conns().shut_all();
        self.book.detach(self.generation);
    }

    fn conns(&self) -> MutexGuard<'_, Conns> {
        self.conns.lock().unwrap()
    }

    fn seen(&self) -> MutexGuard<'_, HashMap<String, Seen>> {
        self.seen.lock().unwrap()
    }

    /// Change what the book knows now, as this manager.
    fn live(&self, change: impl FnOnce(&mut book::Live)) {
        self.book.live(self.generation, change);
    }

    fn lockboxes(&self) -> Arc<Vec<Lockbox>> {
        self.lockboxes.lock().unwrap().clone()
    }

    fn advertise(&self) {
        if self.stopped() {
            return;
        }
        let txt = advert::txt(self.identity.device_id(), self.nonce, &self.lockboxes());
        let advert = Advert {
            port: self.port,
            txt,
        };
        if let Err(e) = self.discovery.advertise(&advert) {
            log::warn!("nearby: not advertising: {e}");
        }
    }

    fn tick(self: Arc<Self>) {
        let mut refreshed = Instant::now();
        while !self.stopped() {
            std::thread::sleep(TICK);
            self.conns().expire(Instant::now());
            if refreshed.elapsed() >= REFRESH {
                refreshed = Instant::now();
                self.refresh();
            }
            self.dial_due();
        }
    }

    /// Read the lockboxes again. When their keys changed, the advertisement
    /// follows, every advertisement seen is read again with them, and a sync
    /// with each device is owed.
    fn refresh(&self) {
        let fresh = self.host.lockboxes();
        if fresh == *self.lockboxes() {
            return;
        }
        *self.lockboxes.lock().unwrap() = Arc::new(fresh);
        self.advertise();
        let lockboxes = self.lockboxes();
        let now = Instant::now();
        for seen in self.seen().values_mut() {
            seen.found = advert::recognise(&seen.txt, &lockboxes);
            seen.due = Some(now);
        }
        self.show_here();
    }

    fn on_event(&self, event: Event) {
        match event {
            Event::Seen(peer) => {
                let mut seen = self.seen();
                let found = advert::recognise(&peer.txt, &self.lockboxes());
                match seen.get_mut(&peer.instance) {
                    // The same advertisement, changed.
                    Some(known) => {
                        known.txt = peer.txt;
                        known.addrs = peer.addrs;
                        known.found = found;
                    }
                    None => {
                        log::info!(
                            "nearby: saw {}, recognised: {}",
                            peer.instance,
                            found.is_some()
                        );
                        let larger = found
                            .as_ref()
                            .is_some_and(|found| found.device < *self.identity.device_id());
                        let wait = if larger { SECOND_DIAL } else { Duration::ZERO };
                        seen.insert(
                            peer.instance,
                            Seen {
                                txt: peer.txt,
                                addrs: peer.addrs,
                                found,
                                due: Some(Instant::now() + wait),
                                backoff: RETRY_FIRST,
                                dialling: false,
                            },
                        );
                    }
                }
            }
            Event::Gone { instance } => {
                self.seen().remove(&instance);
            }
        }
        self.show_here();
    }

    /// Show as here the devices whose advertisements are recognised.
    fn show_here(&self) {
        let here = self
            .seen()
            .values()
            .filter_map(|seen| seen.found.as_ref())
            .map(|found| found.device.to_string())
            .collect();
        self.live(|live| live.here = here);
    }

    /// A sync with every device seen is owed now.
    fn owe_all(&self) {
        let now = Instant::now();
        for seen in self.seen().values_mut() {
            seen.due = Some(now);
        }
    }

    /// Dial every device a sync is due with: one that shares a lockbox with
    /// this device, and is neither paused nor removed here.
    fn dial_due(self: &Arc<Self>) {
        let now = Instant::now();
        let me = *self.identity.device_id();
        let mut due = Vec::new();
        for (instance, seen) in self.seen().iter_mut() {
            let Some(found) = &seen.found else {
                continue;
            };
            let id = found.device.to_string();
            if seen.dialling
                || !seen.due.is_some_and(|at| now >= at)
                || found.device == me
                || self.book.is_removed(&id)
                || self.book.is_paused(&id)
            {
                continue;
            }
            seen.dialling = true;
            seen.due = None;
            due.push((instance.clone(), found.clone(), seen.addrs.clone()));
        }
        for (instance, found, addrs) in due {
            let shared = self.clone();
            let dialling = instance.clone();
            let spawned = spawn("nearby-dial", move || {
                let dialled = connection::dial(&shared, &found, addrs);
                shared.dialled(&dialling, dialled);
            });
            if spawned.is_err() {
                self.dialled(&instance, Dialled::Later);
            }
        }
    }

    /// The dial to the advertisement `instance` ended. One that could not go
    /// through is owed again after a wait that doubles, and that has a random
    /// part, so two devices that dialled each other at once do not again.
    fn dialled(&self, instance: &str, dialled: Dialled) {
        let mut seen = self.seen();
        let Some(seen) = seen.get_mut(instance) else {
            return;
        };
        seen.dialling = false;
        match dialled {
            Dialled::Done => seen.backoff = RETRY_FIRST,
            Dialled::Later => {
                let wait = seen.backoff.mul_f64(1.0 + rand::random::<f64>());
                let at = Instant::now() + wait;
                seen.due = Some(seen.due.map_or(at, |due| due.min(at)));
                seen.backoff = (seen.backoff * 2).min(RETRY_MOST);
            }
        }
    }

    /// Add `ids` to the removed set. Each one new to it is refused from now
    /// on: it is not dialled, its connections close, and a connection from it
    /// is closed as soon as it says whose it is. A sync with every other
    /// device is owed, and every handshake carries the set, so the removal
    /// reaches the owner's other devices. This device's own id is never
    /// taken.
    fn learn_removed(&self, ids: &[String]) {
        let me = self.identity.device_id().to_string();
        let ids: Vec<String> = ids
            .iter()
            .filter(|id| **id != me && id.parse::<DeviceId>().is_ok())
            .cloned()
            .collect();
        let added = self.book.remove(&ids);
        if added.is_empty() {
            return;
        }
        {
            let mut conns = self.conns();
            for peer in added.iter().filter_map(|id| id.parse::<DeviceId>().ok()) {
                conns.shut_peer(&peer);
            }
        }
        self.owe_all();
    }

    #[cfg(not(test))]
    fn count_dial(&self) {}

    #[cfg(not(test))]
    fn count_refused(&self) {}
}
