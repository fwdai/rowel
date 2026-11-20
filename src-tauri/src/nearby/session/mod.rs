//! The session manager (design §5, "Discovery and sessions"): while the app
//! is unlocked and nearby sync is on, this device advertises itself, listens,
//! dials the devices it shares a lockbox with, proves the keys, and runs
//! every lockbox the two hold under the same key, through `exchange`.
//!
//! - [`advert`]: what this device's advertisement says, and what another's
//!   says to it.
//! - [`listener`] and [`connection`]: one listener and one thread per
//!   connection; [`conns`] keeps their count, their deadlines and the one
//!   connection a pair of devices keeps.
//! - [`handshake`]: `hello`, the key proofs, the removed sets.
//! - [`link`] and [`answer`]: a run, from the side that dialled and from the
//!   side that answers.
//! - [`book`]: what is kept per install and what is known now, and the
//!   status built from them.
//!
//! Blocking I/O on plain threads, as the channel is; no async runtime. The
//! lockboxes, the run claim and the clock are the [`Host`]'s, so the whole of
//! it runs in tests as two managers in one process.

mod advert;
mod answer;
pub mod book;
mod connection;
mod conns;
mod handshake;
pub mod host;
mod link;
mod listener;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rand::RngCore;

use self::conns::{Conns, Signal};
use crate::nearby::discovery::{Advert, Discovery, Event, Txt};
use crate::nearby::identity::{DeviceId, Identity};
use crate::nearby::trust::NONCE_LEN;
use crate::nearby::{Error, Result};

pub use book::{Book, NearbyStatus, Stored};
pub use host::{Host, Keys, Lockbox};

/// How often the manager looks for an overdue handshake.
const TICK: Duration = Duration::from_millis(200);

/// How often the lockboxes are read again, and the devices seen but not
/// connected are dialled again. The unlock puts every lockbox but the open
/// one into the keyring a moment after the session opens, and a password
/// change or a new lockbox changes the keys; nothing announces either.
const REFRESH: Duration = Duration::from_secs(3);

/// How long after a dial a device seen is dialled again, if it is still not
/// connected.
const REDIAL: Duration = Duration::from_secs(10);

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

struct Seen {
    txt: Txt,
    addrs: Vec<SocketAddr>,
    next_dial: Instant,
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
        Ok(Self { shared })
    }

    /// Take the advertisement down and close the listener and every
    /// connection. Returns at once: the threads end at their next look at
    /// the stop flag or their next read, and a run in flight ends with them.
    pub fn stop(&self) {
        self.shared.stop();
    }

    /// This device's lockboxes changed here: every connection runs, or asks
    /// the other side to.
    pub fn request_runs(&self) {
        self.shared.conns().signal(None, Signal::Run);
    }

    /// Stop running with `device_id`, or start again; a resume runs at once.
    pub fn set_paused(&self, device_id: &str, paused: bool) {
        self.shared.book.set_paused(device_id, paused);
        if !paused {
            if let Ok(peer) = device_id.parse::<DeviceId>() {
                self.shared.conns().signal(Some(&peer), Signal::Run);
            }
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
        self.discovery.stop();
        self.conns().shut_all();
        self.book.detach(self.generation);
    }

    fn conns(&self) -> MutexGuard<'_, Conns> {
        self.conns.lock().unwrap()
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
                self.redial();
            }
        }
    }

    /// Read the lockboxes again. When their keys changed, the advertisement
    /// follows and every connection is closed, to be proved again from the
    /// new advertisements: what a connection syncs was proved under the keys
    /// it was opened with.
    fn refresh(&self) {
        let fresh = self.host.lockboxes();
        if fresh == *self.lockboxes() {
            return;
        }
        *self.lockboxes.lock().unwrap() = Arc::new(fresh);
        self.advertise();
        self.conns().shut_all();
        let now = Instant::now();
        for seen in self.seen.lock().unwrap().values_mut() {
            seen.next_dial = now;
        }
    }

    fn on_event(self: &Arc<Self>, event: Event) {
        match event {
            Event::Seen(peer) => {
                self.seen.lock().unwrap().insert(
                    peer.instance,
                    Seen {
                        txt: peer.txt.clone(),
                        addrs: peer.addrs.clone(),
                        next_dial: Instant::now() + REDIAL,
                    },
                );
                self.consider(&peer.txt, peer.addrs);
            }
            Event::Gone { instance } => {
                self.seen.lock().unwrap().remove(&instance);
            }
        }
    }

    /// Dial again every device seen and not connected whose time has come.
    fn redial(self: &Arc<Self>) {
        let now = Instant::now();
        let due: Vec<(Txt, Vec<SocketAddr>)> = self
            .seen
            .lock()
            .unwrap()
            .values_mut()
            .filter(|seen| now >= seen.next_dial)
            .map(|seen| {
                seen.next_dial = now + REDIAL;
                (seen.txt.clone(), seen.addrs.clone())
            })
            .collect();
        for (txt, addrs) in due {
            self.consider(&txt, addrs);
        }
    }

    /// Dial the device behind `txt` if it shares a lockbox with this one, is
    /// not refused, is not this device, and has the larger id: of two devices
    /// that see each other, the smaller dials.
    fn consider(self: &Arc<Self>, txt: &Txt, mut addrs: Vec<SocketAddr>) {
        if self.stopped() {
            return;
        }
        let Some(found) = advert::recognise(txt, &self.lockboxes()) else {
            return;
        };
        let me = *self.identity.device_id();
        if found.device <= me || self.book.is_removed(&found.device.to_string()) {
            return;
        }
        if !self.conns().start_dial(found.device) {
            return;
        }
        // The listener is IPv4; an IPv6 address is a fallback at best.
        addrs.sort_by_key(SocketAddr::is_ipv6);
        let shared = self.clone();
        let dialled = spawn("nearby-dial", move || {
            connection::dial(shared, found.device, addrs, found.matches)
        });
        if dialled.is_err() {
            self.conns().end_dial(&found.device);
        }
    }

    /// Add `ids` to the removed set. Each one new to it is refused from now
    /// on: its connections close, its tombstone rows are forgotten in every
    /// lockbox, and every other connection is told, so the removal reaches
    /// the owner's other devices. This device's own id is never taken.
    fn learn_removed(&self, ids: &[String]) {
        let me = self.identity.device_id().to_string();
        let ids: Vec<String> = ids
            .iter()
            .take(handshake::MAX_REMOVED)
            .filter(|id| **id != me && id.parse::<DeviceId>().is_ok())
            .cloned()
            .collect();
        let added = self.book.remove(&ids);
        if added.is_empty() {
            return;
        }
        {
            let mut conns = self.conns();
            for id in &added {
                if let Ok(peer) = id.parse::<DeviceId>() {
                    conns.shut_peer(&peer);
                }
            }
            conns.signal(None, Signal::Removed);
        }
        for id in added {
            let host = self.host.clone();
            if let Err(e) = spawn("nearby-forget", move || host.forget(&id)) {
                log::warn!("nearby: a removed device's rows are kept for now: {e}");
            }
        }
    }

    /// The removed set as one `sync/removed` carries it.
    fn removed_to_send(&self) -> Vec<String> {
        let mut ids = self.book.removed();
        ids.truncate(handshake::MAX_REMOVED);
        ids
    }

    /// A device that holds one of this device's lockboxes says this device
    /// was removed: nearby sync goes off here, as the owner asked.
    fn removed_here(&self) {
        log::info!("nearby: this device was removed on another of the owner's devices");
        self.book.set_enabled(false);
        self.stop();
    }

    #[cfg(not(test))]
    fn count_dial(&self) {}
}
