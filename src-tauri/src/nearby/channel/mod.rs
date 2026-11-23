//! Whole messages between two devices over a mutually authenticated TLS channel.

mod control;
mod tls;

use std::io::{Read, Write};
use std::time::Duration;

use super::identity::DeviceId;
use super::{Error, Result};

pub use control::Control;
pub use tls::{accept, connect_over};

pub const PROTOCOL_VERSION: u32 = 7;
pub const MAX_FRAME: usize = 1024 * 1024;
const MAX_CONTROL: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Connector,
    Acceptor,
}

impl Role {
    pub fn other(self) -> Self {
        match self {
            Self::Connector => Self::Acceptor,
            Self::Acceptor => Self::Connector,
        }
    }
}

trait Stream: Read + Write + Send {}

impl<T: Read + Write + Send> Stream for T {}

pub struct Channel {
    stream: Box<dyn Stream>,
    role: Role,
    peer: DeviceId,
    binding: [u8; 32],
}

impl Channel {
    pub fn peer(&self) -> &DeviceId {
        &self.peer
    }

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn binding(&self) -> &[u8; 32] {
        &self.binding
    }

    pub fn send(&mut self, message: &[u8]) -> Result<()> {
        self.write_len(message.len())?;
        for frame in message.chunks(MAX_FRAME) {
            self.write_len(frame.len())?;
            self.stream.write_all(frame)?;
        }
        self.stream.flush()?;
        Ok(())
    }

    pub fn recv(&mut self, max_len: usize) -> Result<Vec<u8>> {
        let total = self.read_len()?;
        // Refused before allocating; the buffer then grows only as frames arrive.
        if total > max_len {
            return Err(Error::TooLarge {
                len: total,
                max: max_len,
            });
        }
        let mut message = Vec::new();
        while message.len() < total {
            let len = self.read_len()?;
            if len == 0 || len > MAX_FRAME || len > total - message.len() {
                return Err(Error::Protocol(format!(
                    "a frame of {len} bytes in a message of {total}"
                )));
            }
            let start = message.len();
            message.resize(start + len, 0);
            self.stream.read_exact(&mut message[start..])?;
        }
        Ok(message)
    }

    pub fn send_control(&mut self, message: &Control) -> Result<()> {
        self.send(&message.to_json())
    }

    pub fn recv_control(&mut self) -> Result<Control> {
        Control::from_json(&self.recv(MAX_CONTROL)?)
    }

    fn write_len(&mut self, len: usize) -> Result<()> {
        let len = u32::try_from(len).map_err(|_| Error::TooLarge {
            len,
            max: u32::MAX as usize,
        })?;
        self.stream.write_all(&len.to_be_bytes())?;
        Ok(())
    }

    fn read_len(&mut self) -> Result<usize> {
        let mut len = [0u8; 4];
        self.stream.read_exact(&mut len)?;
        Ok(u32::from_be_bytes(len) as usize)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::thread;
    use std::time::Instant;

    use super::*;
    use crate::nearby::identity::tests::identity;
    use crate::nearby::identity::Identity;

    pub fn connect(addr: SocketAddr, me: &Identity, expect: Option<&DeviceId>) -> Result<Channel> {
        connect_over(TcpStream::connect(addr)?, me, expect)
    }

    // `dial` run against `accept` over loopback: what each side got, and the acceptor's socket.
    pub fn meet<T>(
        acceptor: &Identity,
        dial: impl FnOnce(SocketAddr) -> T,
    ) -> (T, Result<Channel>, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::scope(|s| {
            let answered = s.spawn(|| {
                let sock = listener.accept().unwrap().0;
                (sock.try_clone().unwrap(), accept(sock, acceptor))
            });
            let dialled = dial(addr);
            let (sock, answered) = answered.join().unwrap();
            (dialled, answered, sock)
        })
    }

    pub fn pair(connector: &Identity, acceptor: &Identity) -> (Channel, Channel) {
        let (dialled, answered, _) = meet(acceptor, |addr| connect(addr, connector, None));
        (dialled.unwrap(), answered.unwrap())
    }

    fn pair_reading_for(deadline: Duration) -> (Channel, Channel) {
        let (a, b) = (identity(), identity());
        let (dialled, answered, sock) = meet(&b, |addr| connect(addr, &a, None));
        sock.set_read_timeout(Some(deadline)).unwrap();
        (dialled.unwrap(), answered.unwrap())
    }

    #[test]
    fn a_message_of_several_megabytes_crosses_in_frames() {
        let (a, b) = (identity(), identity());
        let (mut dialled, mut answered) = pair(&a, &b);
        let message: Vec<u8> = (0..3 * MAX_FRAME + 12345)
            .map(|i| (i % 251) as u8)
            .collect();
        thread::scope(|s| {
            s.spawn(|| dialled.send(&message).unwrap());
            assert_eq!(answered.recv(4 * MAX_FRAME).unwrap(), message);
        });
    }

    // Only a header is sent: a receiver that read on before refusing would wait out its deadline.
    #[test]
    fn a_declared_length_over_the_limit_is_refused_on_the_header() {
        let (mut dialled, mut answered) = pair_reading_for(Duration::from_secs(5));
        dialled.stream.write_all(&u32::MAX.to_be_bytes()).unwrap();
        dialled.stream.flush().unwrap();
        let started = Instant::now();
        assert!(matches!(
            answered.recv(MAX_FRAME),
            Err(Error::TooLarge { len, max }) if len == u32::MAX as usize && max == MAX_FRAME
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_frame_longer_than_its_message_is_refused() {
        let (a, b) = (identity(), identity());
        let (mut dialled, mut answered) = pair(&a, &b);
        dialled.stream.write_all(&4u32.to_be_bytes()).unwrap();
        dialled.stream.write_all(&5u32.to_be_bytes()).unwrap();
        dialled.stream.flush().unwrap();
        assert!(matches!(answered.recv(16), Err(Error::Protocol(_))));
    }

    #[test]
    fn a_stalled_peer_hits_the_read_deadline() {
        let (_dialled, mut answered) = pair_reading_for(Duration::from_millis(200));
        let started = Instant::now();
        assert!(matches!(answered.recv(16), Err(Error::TimedOut)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
