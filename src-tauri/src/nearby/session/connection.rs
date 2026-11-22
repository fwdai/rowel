//! One connection on its own thread: the dial or the accept, the handshake, the sync, the end.

use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::advert::Recognised;
use super::handshake::{self, Handshake};
use super::host::Host;
use super::listener::is_local;
use super::{sync, Shared};
use crate::nearby::channel;
use crate::nearby::Result;

/// How long a connection may take from its first byte to the end of its
/// handshake. Each read and write has the channel's own deadline as well;
/// this bounds the whole, so a device that trickles a byte at a time cannot
/// hold a connection slot.
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(15);

/// How long one address may take to answer a dial.
const DIAL_TIMEOUT: Duration = Duration::from_secs(3);

/// How a dial ended.
#[derive(Debug)]
pub(super) enum Dialled {
    /// The sync went through, or there is nothing to try again for.
    Done,
    /// It could not go through now: nothing answered, the connection failed,
    /// or a run claim was held on either side. Tried again later.
    Later,
}

/// Answer a connection the listener accepted.
pub(super) fn accept<H: Host>(shared: Arc<Shared<H>>, sock: TcpStream) {
    if let Err(e) = connected(&shared, sock, None) {
        log::info!("nearby: a connection ended: {e}");
    }
}

/// Dial the device `found` at the first of `addrs` that answers, and sync
/// with it.
pub(super) fn dial<H: Host>(
    shared: &Shared<H>,
    found: &Recognised,
    mut addrs: Vec<SocketAddr>,
) -> Dialled {
    shared.count_dial();
    // The listener is IPv4; an IPv6 address is a fallback at best.
    addrs.sort_by_key(SocketAddr::is_ipv6);
    let sock = addrs
        .iter()
        .filter(|addr| is_local(addr.ip()))
        .find_map(|addr| TcpStream::connect_timeout(addr, DIAL_TIMEOUT).ok());
    let device = found.device.to_string();
    let Some(sock) = sock else {
        log::info!("nearby: dial {device:.8}: could not connect");
        return Dialled::Later;
    };
    let dialled = connected(shared, sock, Some(found)).unwrap_or_else(|e| {
        log::info!("nearby: a sync did not go through: {e}");
        Dialled::Later
    });
    log::info!("nearby: dial {device:.8}: {dialled:?}");
    dialled
}

/// A connection from its first byte to its end, as the side that dialled
/// `dialled` or, with `None`, the side that answered.
fn connected<H: Host>(
    shared: &Shared<H>,
    sock: TcpStream,
    dialled: Option<&Recognised>,
) -> Result<Dialled> {
    let deadline = Instant::now() + HANDSHAKE_DEADLINE;
    let Some(conn) = shared.conns().open(&sock, deadline) else {
        return Ok(Dialled::Later);
    };
    let served = serve(shared, conn, sock, dialled);
    shared.conns().close(conn);
    served
}

fn serve<H: Host>(
    shared: &Shared<H>,
    conn: u64,
    sock: TcpStream,
    dialled: Option<&Recognised>,
) -> Result<Dialled> {
    let mut channel = match dialled {
        Some(found) => channel::connect_over(sock, &shared.identity, Some(&found.device)),
        None => channel::accept(sock, &shared.identity),
    }?;
    let peer = *channel.peer();
    let id = peer.to_string();
    // Nothing is said to a removed device, nor to this one.
    if peer == *shared.identity.device_id() || shared.book.is_removed(&id) {
        return Ok(Dialled::Done);
    }
    // Every step on this connection is gated on the keys it proves.
    let lockboxes = shared.lockboxes();
    let matches = dialled.map_or(&[][..], |found| &found.matches);
    let Handshake::Established { vaults, behind } =
        handshake::run(shared, &mut channel, &lockboxes, matches)?
    else {
        return Ok(Dialled::Done);
    };
    shared.conns().proved(conn, peer);
    shared.live(|live| {
        if behind {
            live.behind.insert(id.clone());
        } else {
            live.behind.remove(&id);
        }
    });
    sync::run(shared, &mut channel, &id, &lockboxes, &vaults)?;
    Ok(Dialled::Done)
}
