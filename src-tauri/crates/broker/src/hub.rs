//! The clear signals a host pushes to the connections serving it — the lock
//! signals and the [`ROWEL_SIGNAL`] wake-up — and the [`Hub`] they go out
//! through. A hub is a host's, never the process's: a process may serve
//! several hosts at once (a Rowel Server's lockboxes, a test suite's), and a
//! signal one of them raises is about its vault alone. The serving loops take
//! the hub from the host they serve ([`Host::hub`](crate::actions::Host::hub)),
//! so no connection can be served without saying whose signals it hears.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::signal::ROWEL_SIGNAL;

pub const LOCKED: &[u8] = br#"{"action":"database-locked"}"#;
pub const UNLOCKED: &[u8] = br#"{"action":"database-unlocked"}"#;

/// Where a connection's signals go: one whole message per call, framed
/// however that connection's transport frames one. The socket's is a
/// `frame::write` through its shared `server::Writer`; a transport that is
/// not a length-prefixed stream registers a sink of its own.
pub type Sink = Box<dyn FnMut(&[u8]) -> io::Result<()> + Send>;

// A connection's signal queue, by the id `register` handed out.
type Queue = (u64, Sender<&'static [u8]>);

/// The connections one host is served on, for its clear signals to reach.
/// Fed by the serving loops rather than by `server::serve`, which stays a
/// loop over any stream. Cheap to clone: every clone is the same hub.
#[derive(Clone, Default)]
pub struct Hub(Arc<Connections>);

#[derive(Default)]
struct Connections {
    queues: Mutex<Vec<Queue>>,
    next_id: AtomicU64,
}

impl Hub {
    fn queues(&self) -> MutexGuard<'_, Vec<Queue>> {
        self.0.queues.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Hand a connection's `sink` to this host's signals, until
    /// [`Hub::forget`] is called with the id this returns.
    ///
    /// Each connection gets a queue and a thread of its own to drain it, so
    /// the signals reach it in the order they were raised — a workspace
    /// switch raises a lock and an unlock back to back, and an extension that
    /// heard them the other way round would show the vault just opened as
    /// sealed — and so a peer that has stopped reading, its buffer full and
    /// the write to it blocking, stalls its own thread and nothing else: not
    /// the lock that raised the signal, not the accept loop, and not the
    /// signals to the other browsers. A write that fails ends the thread, and
    /// with it the sink.
    ///
    /// A thread blocked in such a write cannot see its queue dropped, so it
    /// outlives [`Hub::forget`] — for as long as the peer neither reads nor
    /// goes away. That is the proxy process the browser launched, and it is
    /// blocked on the browser's stdio in turn: the moment the browser exits,
    /// the proxy does, the socket closes, and the write fails. One thread and
    /// one socket per hung browser, for the life of that browser, is the
    /// whole of it.
    pub fn register(&self, mut sink: Sink) -> u64 {
        let (sender, receiver) = channel::<&'static [u8]>();
        std::thread::spawn(move || {
            for message in receiver {
                if sink(message).is_err() {
                    break;
                }
            }
        });
        let id = self.0.next_id.fetch_add(1, Ordering::SeqCst);
        self.queues().push((id, sender));
        id
    }

    /// The connection `id` has ended: its queue is dropped, which ends its
    /// thread.
    pub fn forget(&self, id: u64) {
        self.queues().retain(|(known, _)| *known != id);
    }

    /// The vault just locked: every connected extension flips its icon, and
    /// asks again before it fills anything.
    pub fn notify_locked(&self) {
        self.signal(LOCKED);
    }

    /// The vault just opened, or another workspace did: every connected
    /// extension re-checks the hash, which says which one it is talking to
    /// now.
    pub fn notify_unlocked(&self) {
        self.signal(UNLOCKED);
    }

    /// The host has something queued for an extension (`AgentState::signals`):
    /// a fill grant an agent obtained, a grant that ended. What it is stays
    /// inside the sealed `get-signals` the extension answers this with; the
    /// wake-up itself says nothing, like the lock signals beside it. Called
    /// with every lock let go — a queue is unbounded and a send never waits,
    /// but the order of things is that a signal follows the change it
    /// reports.
    pub fn notify_signals(&self) {
        self.signal(ROWEL_SIGNAL);
    }

    // Queue `message` for every connection to this host, in the clear and
    // unsolicited, as KeePassXC sends its signals: no nonce to seal under, and
    // nothing in them the extension would not learn from its next ask. Never
    // waits: a queue is unbounded, and one whose thread has ended — its write
    // failed, the peer is gone — refuses the send and goes from the list with
    // it.
    fn signal(&self, message: &'static [u8]) {
        self.queues()
            .retain(|(_, queue)| queue.send(message).is_ok());
    }
}
