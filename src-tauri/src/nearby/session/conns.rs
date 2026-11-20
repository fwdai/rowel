//! The open connections: how many there may be, how long one may take to
//! prove itself, and the one each pair of devices keeps.
//!
//! Every connection, dialled or accepted, is registered here from its first
//! byte with a copy of its socket, so it can be shut down from anywhere: when
//! its handshake runs past the deadline, when a better connection to the same
//! device arrives, when the device is removed, when the manager stops. A
//! shut-down socket fails the connection's own thread at its next read or
//! write, and that thread unregisters it on its way out.

use std::collections::{HashMap, HashSet};
use std::net::{Shutdown, TcpStream};
use std::sync::mpsc::Sender;
use std::time::Instant;

use crate::nearby::channel::Role;
use crate::nearby::identity::DeviceId;

/// The most connections open at once, handshakes included.
const MAX_CONNECTIONS: usize = 16;

/// What another thread asks of an established connection's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Signal {
    /// This device's lockboxes changed: the side that runs runs, the other
    /// asks it to.
    Run,
    /// The removed set grew: tell the peer.
    Removed,
}

struct Conn {
    sock: TcpStream,
    /// Until the handshake is through; a connection still in it past this is
    /// shut down.
    deadline: Option<Instant>,
    peer: Option<DeviceId>,
    role: Role,
    /// Whether the side that dialled has the smaller id: the orientation the
    /// pair keeps.
    dialler_smaller: bool,
    signals: Option<Sender<Signal>>,
}

#[derive(Default)]
pub(super) struct Conns {
    next: u64,
    open: HashMap<u64, Conn>,
    /// Devices a dial to is under way, so a second is not started beside it.
    dialling: HashSet<DeviceId>,
}

impl Conns {
    /// Register a new connection over `sock` (a copy of it is kept), its
    /// handshake due by `deadline`. `None` when [`MAX_CONNECTIONS`] are open.
    pub fn open(&mut self, sock: &TcpStream, role: Role, deadline: Instant) -> Option<u64> {
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
                role,
                dialler_smaller: false,
                signals: None,
            },
        );
        Some(self.next)
    }

    /// Connection `id`'s handshake named the device at the other end. The pair
    /// keeps one connection: the one dialled by the device with the smaller
    /// id, else the newer. Both devices apply the same rule to the same two
    /// connections, so they keep the same one. `false` when this one is not
    /// it; the one it beats is shut down.
    pub fn identified(&mut self, id: u64, local: &DeviceId, peer: DeviceId) -> bool {
        let Some(role) = self.open.get(&id).map(|c| c.role) else {
            return false;
        };
        let dialler_smaller = match role {
            Role::Connector => *local < peer,
            Role::Acceptor => peer < *local,
        };
        // This connection is not the peer's yet, so any connection that is
        // is the rival.
        let kept = |c: &Conn| c.peer == Some(peer) && c.dialler_smaller;
        if !dialler_smaller && self.open.values().any(kept) {
            return false;
        }
        self.shut_peer(&peer);
        if let Some(conn) = self.open.get_mut(&id) {
            conn.peer = Some(peer);
            conn.dialler_smaller = dialler_smaller;
        }
        true
    }

    /// Connection `id` is through its handshake; `signals` reaches its thread.
    pub fn established(&mut self, id: u64, signals: Sender<Signal>) {
        if let Some(conn) = self.open.get_mut(&id) {
            conn.deadline = None;
            conn.signals = Some(signals);
        }
    }

    /// Connection `id` ended. The device comes back when this was the last
    /// connection to it; not when it was shut down for another, or before it
    /// knew whose it was.
    pub fn close(&mut self, id: u64) -> Option<DeviceId> {
        let peer = self.open.remove(&id)?.peer?;
        (!self.has_peer(&peer)).then_some(peer)
    }

    pub fn has_peer(&self, peer: &DeviceId) -> bool {
        self.open.values().any(|c| c.peer.as_ref() == Some(peer))
    }

    /// Start a dial to `peer`, unless one is under way or a connection to it
    /// is open. `false` when not.
    pub fn start_dial(&mut self, peer: DeviceId) -> bool {
        !self.has_peer(&peer) && self.dialling.insert(peer)
    }

    pub fn end_dial(&mut self, peer: &DeviceId) {
        self.dialling.remove(peer);
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

    /// Tell every established connection `signal`, or only the ones to `peer`.
    pub fn signal(&self, peer: Option<&DeviceId>, signal: Signal) {
        for conn in self.open.values() {
            if peer.is_some_and(|p| conn.peer.as_ref() != Some(p)) {
                continue;
            }
            if let Some(signals) = &conn.signals {
                let _ = signals.send(signal.clone());
            }
        }
    }

    /// The established connections: the device and this side's role.
    #[cfg(test)]
    pub fn established_peers(&self) -> Vec<(DeviceId, Role)> {
        self.open
            .values()
            .filter(|c| c.signals.is_some())
            .filter_map(|c| Some((c.peer?, c.role)))
            .collect()
    }

    // Shut, not removed: the connection's own thread unregisters it once its
    // read or write fails, which keeps the count true until it has.
    fn shut_where(&mut self, which: impl Fn(&Conn) -> bool) {
        for conn in self.open.values_mut().filter(|c| which(c)) {
            let _ = conn.sock.shutdown(Shutdown::Both);
            conn.peer = None;
            conn.signals = None;
        }
    }
}
