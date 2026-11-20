//! The open connections: how many there may be, how long one may take to
//! prove itself, and a copy of each socket.
//!
//! Every connection, dialled or accepted, is registered here from its first
//! byte, so it can be shut down from another thread: when its handshake runs
//! past the deadline, when its device is removed, when the manager stops. A
//! shut-down socket fails the connection's own thread at its next read or
//! write, and that thread unregisters it on its way out.

use std::collections::HashMap;
use std::net::{Shutdown, TcpStream};
use std::time::Instant;

use crate::nearby::identity::DeviceId;

/// The most connections open at once, handshakes included.
const MAX_CONNECTIONS: usize = 16;

struct Conn {
    sock: TcpStream,
    /// Until the handshake is through; a connection still in it past this is
    /// shut down.
    deadline: Option<Instant>,
    peer: Option<DeviceId>,
}

#[derive(Default)]
pub(super) struct Conns {
    next: u64,
    open: HashMap<u64, Conn>,
}

impl Conns {
    /// Register a new connection over `sock` (a copy of it is kept), its
    /// handshake due by `deadline`. `None` when [`MAX_CONNECTIONS`] are open.
    pub fn open(&mut self, sock: &TcpStream, deadline: Instant) -> Option<u64> {
        if self.open.len() >= MAX_CONNECTIONS {
            return None;
        }
        let sock = sock.try_clone().ok()?;
        self.next += 1;
        self.open.insert(
            self.next,
            Conn {
                sock,
                deadline: Some(deadline),
                peer: None,
            },
        );
        Some(self.next)
    }

    /// Connection `id` is through its handshake with `peer`.
    pub fn proved(&mut self, id: u64, peer: DeviceId) {
        if let Some(conn) = self.open.get_mut(&id) {
            conn.deadline = None;
            conn.peer = Some(peer);
        }
    }

    /// Connection `id` ended.
    pub fn close(&mut self, id: u64) {
        self.open.remove(&id);
    }

    /// Shut down every connection whose handshake is overdue.
    pub fn expire(&mut self, now: Instant) {
        self.shut_where(|c| c.deadline.is_some_and(|d| now >= d));
    }

    pub fn shut_peer(&mut self, peer: &DeviceId) {
        self.shut_where(|c| c.peer.as_ref() == Some(peer));
    }

    pub fn shut_all(&mut self) {
        self.shut_where(|_| true);
    }

    // Shut, not removed: the connection's own thread unregisters it once its
    // read or write fails, which keeps the count true until it has.
    fn shut_where(&self, which: impl Fn(&Conn) -> bool) {
        for conn in self.open.values().filter(|c| which(c)) {
            let _ = conn.sock.shutdown(Shutdown::Both);
        }
    }
}
