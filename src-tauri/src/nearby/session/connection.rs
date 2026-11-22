//! One connection on its own thread: dialled or accepted, then the handshake and the sync.

use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use super::advert::Recognised;
use super::handshake::{self, Handshake};
use super::host::Host;
use super::listener::is_local;
use super::{sync, Shared};
use crate::nearby::channel::{self, Channel};
use crate::nearby::{Error, Result};

const DIAL_TIMEOUT: Duration = Duration::from_secs(3);

pub(super) fn accept<H: Host>(shared: &Shared<H>, sock: TcpStream) {
    if let Err(e) = connected(shared, sock, None) {
        log::info!("nearby: a connection ended: {e}");
    }
}

pub(super) fn dial<H: Host>(shared: &Shared<H>, found: &Recognised, addrs: &[SocketAddr]) {
    shared.count_dial();
    let outcome = match reach(addrs).map(|sock| connected(shared, sock, Some(found))) {
        None => "could not connect".to_string(),
        Some(Ok(())) => "done".to_string(),
        Some(Err(e)) => e.to_string(),
    };
    log::info!("nearby: dial {:.8}: {outcome}", found.device.to_string());
}

fn reach(addrs: &[SocketAddr]) -> Option<TcpStream> {
    let mut addrs: Vec<&SocketAddr> = addrs.iter().filter(|a| is_local(a.ip())).collect();
    // The listener is IPv4; an IPv6 address is a fallback at best.
    addrs.sort_by_key(|a| a.is_ipv6());
    addrs
        .into_iter()
        .find_map(|a| TcpStream::connect_timeout(a, DIAL_TIMEOUT).ok())
}

fn connected<H: Host>(
    shared: &Shared<H>,
    sock: TcpStream,
    dialled: Option<&Recognised>,
) -> Result<()> {
    let conn = shared
        .conns()
        .open(&sock)
        .ok_or(Error::Ended("too many connections are open"))?;
    let served = serve(shared, conn, sock, dialled);
    shared.conns().close(conn);
    served
}

fn serve<H: Host>(
    shared: &Shared<H>,
    conn: u64,
    sock: TcpStream,
    dialled: Option<&Recognised>,
) -> Result<()> {
    let mut channel = match dialled {
        Some(found) => channel::connect_over(sock, &shared.identity, Some(&found.device)),
        None => channel::accept(sock, &shared.identity),
    }?;
    let synced = sync_with(shared, conn, &mut channel, dialled);
    shared.schedule().ended(channel.peer());
    synced
}

fn sync_with<H: Host>(
    shared: &Shared<H>,
    conn: u64,
    channel: &mut Channel,
    dialled: Option<&Recognised>,
) -> Result<()> {
    let peer = channel.peer().to_string();
    if channel.peer() == shared.identity.device_id() || shared.book.is_removed(&peer) {
        return Ok(());
    }
    let lockboxes = shared.lockboxes();
    let matches = dialled.map_or(&[][..], |found| &found.matches);
    let Handshake::Established { vaults, behind } =
        handshake::run(shared, channel, &lockboxes, matches)?
    else {
        return Ok(());
    };
    shared.conns().established(conn);
    shared.live(|live| {
        if behind {
            live.behind.insert(peer.clone());
        } else {
            live.behind.remove(&peer);
        }
    });
    sync::run(shared, channel, &peer, &lockboxes, &vaults)
}
