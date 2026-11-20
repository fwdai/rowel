//! One connection's life, on a thread of its own: the dial or the accept, the
//! handshake, the runs, the end.
//!
//! A connection is one sync. The side that dialled runs every lockbox the
//! two proved they share, one after another, and closes; the other answers
//! until it does. Nothing is said on a connection but a request and its
//! answer, and nothing of it is kept afterwards but what the book shows.

use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::advert::Recognised;
use super::handshake::{self, Handshake};
use super::host::{Host, Lockbox};
use super::link::{failure_of, ChannelLink};
use super::listener::is_local;
use super::{answer, Shared};
use crate::error::Error;
use crate::nearby::channel::{self, Channel, Control};
use crate::nearby::exchange;
use crate::nearby::Result;

/// How long a connection may take from its first byte to the end of its
/// handshake. Each read and write has the channel's own deadline as well;
/// this bounds the whole, so a device that trickles a byte at a time cannot
/// hold a connection slot.
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(15);

/// How long one address may take to answer a dial.
const DIAL_TIMEOUT: Duration = Duration::from_secs(3);

/// How a dial ended.
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
        log::debug!("nearby: a connection ended: {e}");
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
    let Some(sock) = sock else {
        return Dialled::Later;
    };
    connected(shared, sock, Some(found)).unwrap_or_else(|e| {
        log::debug!("nearby: a sync did not go through: {e}");
        Dialled::Later
    })
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
    // The lockboxes as this connection proves them: every run on it is under
    // these keys ([`Shared::begin`]).
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
    if dialled.is_none() {
        answer::serve(shared, &mut channel, &id, &lockboxes, &vaults)?;
        return Ok(Dialled::Done);
    }
    for vault in &vaults {
        if !run(shared, &mut channel, &id, &lockboxes, vault)? {
            return Ok(Dialled::Later);
        }
    }
    Ok(Dialled::Done)
}

/// One run of `vault` with the device `id`, driven from here. `false` when it
/// could not start for a claim held on either side.
fn run<H: Host>(
    shared: &Shared<H>,
    channel: &mut Channel,
    id: &str,
    lockboxes: &[Lockbox],
    vault: &str,
) -> Result<bool> {
    let local = match shared.begin(vault, lockboxes) {
        Ok(local) => local,
        Err(Error::SyncBusy | Error::Locked) => return Ok(false),
        Err(e) => {
            shared.live(|live| {
                live.failed.insert(id.to_string(), failure_of(&e));
            });
            return Ok(true);
        }
    };
    shared.live(|live| {
        live.syncing.insert(id.to_string());
        live.failed.remove(id);
    });
    let now_ms = shared.host.now_ms();
    let link = ChannelLink::new(channel, vault, now_ms);
    let (outcome, result) = exchange::sync_with_peer(&link, &*local, now_ms);
    let broken = link.into_broken();
    let busy = matches!(result, Err(Error::SyncBusy));
    // The other side ends its half of the run on this. One that answered
    // busy opened none.
    let said = if broken.is_none() && !busy {
        channel.send_control(&Control::Done {
            vault: vault.to_string(),
            error: result.as_ref().err().map(failure_of),
        })
    } else {
        Ok(())
    };
    // However the run ended, what it merged here is announced and passed on.
    shared.host.finish(local, outcome);
    shared.live(|live| {
        live.syncing.remove(id);
    });
    if let Some(e) = broken {
        return Err(e);
    }
    said?;
    match result {
        Ok(()) => shared.book.converged(id, now_ms),
        Err(Error::SyncBusy) => return Ok(false),
        Err(e) => {
            log::info!("nearby: a run with {id} failed: {e}");
            shared.live(|live| {
                live.failed.insert(id.to_string(), failure_of(&e));
            });
        }
    }
    Ok(true)
}
