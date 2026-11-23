//! The open connections: their cap, their handshake deadlines, and a copy of each socket to shut.

use std::collections::HashMap;
use std::net::{Shutdown, TcpStream};
use std::time::{Duration, Instant};

const MAX_CONNECTIONS: usize = 16;
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(15);

struct Conn {
    sock: TcpStream,
    deadline: Option<Instant>,
}

#[derive(Default)]
pub(super) struct Conns {
    next: u64,
    open: HashMap<u64, Conn>,
}

impl Conns {
    pub fn open(&mut self, sock: &TcpStream) -> Option<u64> {
        if self.open.len() >= MAX_CONNECTIONS {
            return None;
        }
        let sock = sock.try_clone().ok()?;
        let deadline = Some(Instant::now() + HANDSHAKE_DEADLINE);
        self.next += 1;
        self.open.insert(self.next, Conn { sock, deadline });
        Some(self.next)
    }

    pub fn established(&mut self, id: u64) {
        if let Some(conn) = self.open.get_mut(&id) {
            conn.deadline = None;
        }
    }

    pub fn close(&mut self, id: u64) {
        self.open.remove(&id);
    }

    pub fn expire(&self) {
        let now = Instant::now();
        self.shut_where(|c| c.deadline.is_some_and(|d| now >= d));
    }

    pub fn shut_all(&self) {
        self.shut_where(|_| true);
    }

    // Shut, not removed: the connection's own thread closes it once its read or write fails.
    fn shut_where(&self, which: impl Fn(&Conn) -> bool) {
        for conn in self.open.values().filter(|c| which(c)) {
            let _ = conn.sock.shutdown(Shutdown::Both);
        }
    }
}
