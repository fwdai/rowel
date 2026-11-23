//! One connection on its own thread: dialled or accepted, then the handshake and the sync.

use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use super::advert::Recognised;
use super::book::Outcome;
use super::handshake::{self, Established, Handshake};
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
    let device = found.device.to_string();
    let failed = match reach(addrs).map(|sock| connected(shared, sock, Some(found))) {
        Some(Ok(())) => None,
        None => Some("could not connect".to_string()),
        Some(Err(e)) => Some(e.to_string()),
    };
    if failed.is_some() {
        shared.record(&device, None);
    }
    log::info!(
        "nearby: dial {device:.8}: {}",
        failed.as_deref().unwrap_or("done")
    );
}

fn reach(addrs: &[SocketAddr]) -> Option<TcpStream> {
    let mut addrs: Vec<&SocketAddr> = addrs.iter().filter(|a| is_local(a.ip())).collect();
    // The listener is IPv4; an IPv6 address is a fallback at best.
    addrs.sort_by_key(|a| a.is_ipv6());
    addrs
        .into_iter()
        .find_map(|a| TcpStream::connect_timeout(a, DIAL_TIMEOUT).ok())
}

// An error only before the channel: past it, the outcome is recorded instead.
fn connected<H: Host>(
    shared: &Shared<H>,
    sock: TcpStream,
    dialled: Option<&Recognised>,
) -> Result<()> {
    let conn = shared
        .conns()
        .open(&sock)
        .ok_or(Error::Ended("too many connections are open"))?;
    let opened = match dialled {
        Some(found) => channel::connect_over(sock, &shared.identity, Some(&found.device)),
        None => channel::accept(sock, &shared.identity),
    };
    let served = opened.map(|mut channel| serve(shared, conn, &mut channel, dialled));
    shared.conns().close(conn);
    served
}

fn serve<H: Host>(
    shared: &Shared<H>,
    conn: u64,
    channel: &mut Channel,
    dialled: Option<&Recognised>,
) {
    let peer = channel.peer().to_string();
    let outcome = sync_with(shared, conn, channel, dialled).unwrap_or_else(|e| {
        log::info!("nearby: with {peer:.8}: {e}");
        None
    });
    shared.record(&peer, outcome);
    shared.schedule().ended(channel.peer(), dialled.is_none());
}

fn sync_with<H: Host>(
    shared: &Shared<H>,
    conn: u64,
    channel: &mut Channel,
    dialled: Option<&Recognised>,
) -> Result<Option<Outcome>> {
    let peer = channel.peer().to_string();
    if channel.peer() == shared.identity.device_id() || shared.book.refuses(&peer) {
        return Ok(None);
    }
    let lockboxes = shared.lockboxes();
    let matches = dialled.map_or(&[][..], |found| &found.matches);
    let Handshake::Established(found) = handshake::run(shared, channel, &lockboxes, matches)?
    else {
        return Ok(None);
    };
    shared.conns().established(conn);
    shared.syncing(&peer);
    let synced = sync::run(shared, channel, &peer, &lockboxes, &found);
    Ok(on_another_password(&found).or(synced))
}

fn on_another_password(found: &Established) -> Option<Outcome> {
    if !found.behind.is_empty() {
        return Some(Outcome::Behind);
    }
    (!found.ahead.is_empty()).then_some(Outcome::Ahead)
}
