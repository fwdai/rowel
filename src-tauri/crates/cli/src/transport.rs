//! The socket to the running app, as the extension's proxy reaches it: the
//! data directory's socket (a named pipe on Windows), native messaging frames
//! on it, one request out and one reply in.

use std::io;

use interprocess::local_socket::{prelude::*, Stream};
use rowel_broker::client::Transport;
use rowel_broker::frame;
use rowel_broker::socket::{root_dir, socket_name};

/// A connected stream to the app.
pub struct Socket(Stream);

/// Connect to the app, or say why not: no data directory for this user, or
/// nothing listening — which is the app not running.
pub fn connect() -> io::Result<Socket> {
    let root = root_dir().ok_or_else(|| io::Error::other("no data directory for this user"))?;
    socket_name(&root).and_then(Stream::connect).map(Socket)
}

impl Transport for Socket {
    fn roundtrip(&mut self, request: &[u8]) -> io::Result<Vec<u8>> {
        frame::write(&mut self.0, request)?;
        frame::read(&mut self.0)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the app closed the connection",
            )
        })
    }
}
