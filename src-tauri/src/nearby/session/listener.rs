//! The one listener: connections from this network's private and link-local
//! addresses (and this machine's own), each handed to a thread of its own.

use std::net::{IpAddr, TcpListener};
use std::sync::Arc;
use std::time::Duration;

use super::host::Host;
use super::{connection, Shared};

/// How often the listener looks for a connection, and at the stop flag.
const ACCEPT_POLL: Duration = Duration::from_millis(100);

/// Whether a connection from `ip` may be answered, and `ip` dialled: a
/// private or link-local address on this network, or this machine. Anything
/// routed from beyond it is refused before a byte of TLS.
pub(super) fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local() || v4.is_loopback(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_local(IpAddr::V4(v4)),
            None => v6.is_loopback() || v6.is_unicast_link_local() || v6.is_unique_local(),
        },
    }
}

/// Accept until the manager stops; the listener closes as this returns.
pub(super) fn serve<H: Host>(shared: Arc<Shared<H>>, listener: TcpListener) {
    if let Err(e) = listener.set_nonblocking(true) {
        log::warn!("nearby: the listener cannot wait for connections: {e}");
        return;
    }
    while !shared.stopped() {
        let (sock, from) = match listener.accept() {
            Ok(accepted) => accepted,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(ACCEPT_POLL);
                continue;
            }
            Err(e) => {
                log::warn!("nearby: accepting a connection failed: {e}");
                std::thread::sleep(ACCEPT_POLL);
                continue;
            }
        };
        if !is_local(from.ip()) {
            continue;
        }
        // An accepted socket takes the listener's non-blocking mode on some
        // systems; the channel's deadlines need a blocking one.
        if sock.set_nonblocking(false).is_err() {
            continue;
        }
        let shared = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("nearby-conn".into())
            .spawn(move || connection::accept(shared, sock));
        if let Err(e) = spawned {
            log::warn!("nearby: could not answer a connection: {e}");
        }
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
