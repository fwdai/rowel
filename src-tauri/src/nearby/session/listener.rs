//! The one listener: connections from this network and this machine, each on a thread of its own.

use std::io::ErrorKind;
use std::net::{IpAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use super::host::Host;
use super::{connection, spawn, Shared};

const ACCEPT_POLL: Duration = Duration::from_millis(100);

pub(super) fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local() || v4.is_loopback(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_local(IpAddr::V4(v4)),
            None => v6.is_loopback() || v6.is_unicast_link_local() || v6.is_unique_local(),
        },
    }
}

pub(super) fn serve<H: Host>(shared: Arc<Shared<H>>, listener: TcpListener) {
    if let Err(e) = listener.set_nonblocking(true) {
        log::warn!("nearby: the listener cannot wait for connections: {e}");
        return;
    }
    while !shared.stopped() {
        match listener.accept() {
            Ok((sock, from)) if is_local(from.ip()) => answer(&shared, sock),
            Ok(_) => {}
            Err(e) => {
                if e.kind() != ErrorKind::WouldBlock {
                    log::warn!("nearby: accepting a connection failed: {e}");
                }
                std::thread::sleep(ACCEPT_POLL);
            }
        }
    }
}

fn answer<H: Host>(shared: &Arc<Shared<H>>, sock: TcpStream) {
    // An accepted socket takes the listener's non-blocking mode on some systems.
    if sock.set_nonblocking(false).is_err() {
        return;
    }
    let shared = shared.clone();
    if let Err(e) = spawn("nearby-conn", move || connection::accept(&shared, sock)) {
        log::warn!("nearby: could not answer a connection: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_this_network_and_this_machine_are_local() {
        for local in [
            "10.0.0.7",
            "172.16.4.1",
            "192.168.1.20",
            "169.254.3.3",
            "127.0.0.1",
            "::1",
            "fe80::1",
            "fd00::2",
            "::ffff:192.168.1.20",
        ] {
            assert!(is_local(local.parse().unwrap()), "{local}");
        }
        for remote in [
            "8.8.8.8",
            "100.64.0.1",
            "172.32.0.1",
            "2001:db8::1",
            "::ffff:8.8.8.8",
            "0.0.0.0",
        ] {
            assert!(!is_local(remote.parse().unwrap()), "{remote}");
        }
    }
}
