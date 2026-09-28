//! Native messaging framing: a four-byte length in the host's byte order, then
//! that many bytes of JSON. It is what the browser speaks to its host over
//! stdio, and the proxy relays each frame to the app unchanged, so one reader
//! and one writer serve both ends of the relay and the app's own listener.

use std::io::{self, Read, Write};

/// The most a frame may carry, either way. Chrome refuses more than 1 MiB from
/// a host, and no request this host acts on comes anywhere near it.
pub const MAX_FRAME: usize = 1024 * 1024;

/// One frame, or `None` at a clean end of stream — nothing after the last
/// frame. A stream that ends inside a frame is an error, as is a length past
/// the cap: the bytes it announces are never read.
pub fn read(from: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match from.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_ne_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame of {len} bytes is over the {MAX_FRAME} byte cap"),
        ));
    }
    let mut body = vec![0u8; len];
    from.read_exact(&mut body)?;
    Ok(Some(body))
}

/// One frame, flushed: the reader on the other side waits on the whole of it.
pub fn write(to: &mut impl Write, body: &[u8]) -> io::Result<()> {
    if body.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "frame of {} bytes is over the {MAX_FRAME} byte cap",
                body.len()
            ),
        ));
    }
    to.write_all(&(body.len() as u32).to_ne_bytes())?;
    to.write_all(body)?;
    to.flush()
}
