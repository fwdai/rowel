//! The session manager (design §5): advertise, listen, and sync with the devices that share a lockbox.

mod advert;
pub mod book;
mod connection;
mod conns;
mod handshake;
pub mod host;
mod listener;
mod schedule;
mod sync;
#[cfg(test)]
mod tests;

use std::net::{IpAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rand::RngCore;

use self::book::Outcome;
use self::conns::Conns;
use self::schedule::{Dial, Schedule};
use crate::nearby::discovery::{Advert, Discovery, Event};
use crate::nearby::identity::{DeviceId, Identity};
use crate::nearby::trust::NONCE_LEN;
use crate::nearby::{Error, Result};

pub use book::{Book, NearbyStatus, Stored};
pub use host::{Host, Keys, Lockbox};

const TICK: Duration = Duration::from_millis(200);
// Nothing announces a change of keys: the unlock fills the keyring late, a password change rekeys.
const REFRESH: Duration = Duration::from_secs(3);

pub struct Config<H: Host> {
    pub host: Arc<H>,
    pub identity: Identity,
    pub discovery: Box<dyn Discovery>,
    pub book: Arc<Book>,
    pub listen: IpAddr,
    pub period: Duration,
}

pub struct Manager<H: Host> {
    shared: Arc<Shared<H>>,
}

struct Shared<H: Host> {
    host: Arc<H>,
    identity: Identity,
    discovery: Box<dyn Discovery>,
    book: Arc<Book>,
    generation: u64,
    stopped: AtomicBool,
    port: u16,
    nonce: [u8; NONCE_LEN],
    lockboxes: Mutex<Arc<Vec<Lockbox>>>,
    conns: Mutex<Conns>,
    schedule: Mutex<Schedule>,
    #[cfg(test)]
    counts: tests::Counts,
}

impl<H: Host> Manager<H> {
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
            schedule: Mutex::new(Schedule::new(config.period)),
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

    pub fn stop(&self) {
        self.shared.stop();
    }

    pub fn request_runs(&self) {
        self.shared.schedule().due_now();
    }

    pub fn set_paused(&self, device_id: &str, paused: bool) {
        self.shared.book.set_paused(device_id, paused);
        if !paused {
            self.shared.schedule().due_now();
        }
    }

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

    fn schedule(&self) -> MutexGuard<'_, Schedule> {
        self.schedule.lock().unwrap()
    }

    fn syncing(&self, id: &str) {
        self.book.syncing(self.generation, id);
    }

    fn record(&self, id: &str, outcome: Option<Outcome>) {
        self.book
            .record(self.generation, id, outcome, self.host.now_ms());
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
            self.conns().expire();
            if refreshed.elapsed() >= REFRESH {
                refreshed = Instant::now();
                self.refresh();
            }
            self.dial_due();
        }
    }

    fn refresh(&self) {
        let fresh = self.host.lockboxes();
        if fresh == *self.lockboxes() {
            return;
        }
        let fresh = Arc::new(fresh);
        *self.lockboxes.lock().unwrap() = fresh.clone();
        self.advertise();
        self.schedule().rekeyed(&fresh);
        self.show_here();
    }

    fn on_event(&self, event: Event) {
        match event {
            Event::Seen(peer) => {
                let mut schedule = self.schedule();
                // Read under the schedule's lock, so a refresh's re-read cannot come in between.
                schedule.seen(peer, &self.lockboxes());
            }
            Event::Gone { instance } => self.schedule().gone(&instance),
        }
        self.show_here();
    }

    fn show_here(&self) {
        let here = self.schedule().here();
        self.book.here(self.generation, here);
    }

    fn dial_due(self: &Arc<Self>) {
        let due = self.schedule().take_due(|device| self.may_dial(device));
        for dial in due {
            let instance = dial.instance.clone();
            let shared = self.clone();
            if spawn("nearby-dial", move || shared.dial(dial)).is_err() {
                self.schedule().dialled(&instance);
            }
        }
    }

    fn may_dial(&self, device: &DeviceId) -> bool {
        device != self.identity.device_id() && !self.book.refuses(&device.to_string())
    }

    fn dial(&self, dial: Dial) {
        connection::dial(self, &dial.found, &dial.addrs);
        self.schedule().dialled(&dial.instance);
    }

    fn learn_removed(&self, ids: &[String]) {
        let me = self.identity.device_id().to_string();
        let ids: Vec<String> = ids
            .iter()
            .filter(|id| **id != me && id.parse::<DeviceId>().is_ok())
            .cloned()
            .collect();
        if !self.book.remove(&ids).is_empty() {
            self.schedule().due_now();
        }
    }

    #[cfg(not(test))]
    fn count_dial(&self) {}

    #[cfg(not(test))]
    fn count_refused(&self) {}
}
