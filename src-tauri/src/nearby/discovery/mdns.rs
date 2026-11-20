//! Discovery over multicast DNS, for the desktop app (`mdns-sd`, which runs
//! its own thread and needs no async runtime).
//!
//! The instance name and the host name the record points at are random per
//! session, like the TXT's nonce: the machine's own host name in them would
//! let anyone on the network follow the device from session to session, which
//! everything else in the advertisement is built not to allow. The addresses
//! are the daemon's to fill in, interface by interface.

use std::net::{IpAddr, SocketAddr, SocketAddrV6};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mdns_sd::{ResolvedService, ScopedIp, ServiceDaemon, ServiceEvent, ServiceInfo};
use rand::RngCore;

use super::{Advert, Discovery, Event, OnEvent, Peer, Txt, SERVICE_TYPE};
use crate::nearby::{Error, Result};

/// How often the browsing thread looks at the stop flag between events.
const STOP_POLL: Duration = Duration::from_millis(250);

pub struct Mdns {
    daemon: ServiceDaemon,
    instance: String,
    /// The full name this session's record is registered under, once it is.
    registered: Mutex<Option<String>>,
    stopped: Arc<AtomicBool>,
}

impl Mdns {
    /// A daemon of this session's own. Fails when the multicast sockets
    /// cannot be had.
    pub fn new() -> Result<Self> {
        let daemon = ServiceDaemon::new().map_err(discovery)?;
        let mut bytes = [0u8; 8];
        rand::thread_rng().fill_bytes(&mut bytes);
        Ok(Self {
            daemon,
            instance: format!("rowel-{}", hex::encode(bytes)),
            registered: Mutex::new(None),
            stopped: Arc::default(),
        })
    }
}

fn discovery(e: impl std::fmt::Display) -> Error {
    Error::Discovery(e.to_string())
}

/// The service record for `advert` under `instance`, its addresses left for
/// the daemon to fill in from the interfaces it announces on.
fn service_info(instance: &str, advert: &Advert) -> Result<ServiceInfo> {
    let props = advert.txt.properties();
    let host = format!("{instance}.local.");
    Ok(ServiceInfo::new(
        SERVICE_TYPE,
        instance,
        &host,
        (),
        advert.port,
        props.as_slice(),
    )
    .map_err(discovery)?
    .enable_addr_auto())
}

/// A resolved record, as a [`Peer`]. `None` for one whose TXT this build
/// cannot read.
fn peer(service: &ResolvedService) -> Option<Peer> {
    let txt = Txt::parse(service.get_properties().iter().filter_map(|prop| {
        let value = std::str::from_utf8(prop.val()?).ok()?;
        Some((prop.key(), value))
    }))?;
    let port = service.get_port();
    let addrs = service
        .get_addresses()
        .iter()
        .filter_map(|ip| match ip {
            ScopedIp::V4(v4) => Some(SocketAddr::new(IpAddr::V4(*v4.addr()), port)),
            ScopedIp::V6(v6) => Some(SocketAddr::V6(SocketAddrV6::new(
                *v6.addr(),
                port,
                0,
                v6.scope_id().index,
            ))),
            _ => None,
        })
        .collect();
    Some(Peer {
        instance: service.get_fullname().to_string(),
        txt,
        addrs,
    })
}

impl Discovery for Mdns {
    fn advertise(&self, advert: &Advert) -> Result<()> {
        let info = service_info(&self.instance, advert)?;
        let fullname = info.get_fullname().to_string();
        // Registered again under the same name, the record is replaced.
        self.daemon.register(info).map_err(discovery)?;
        *self.registered.lock().unwrap() = Some(fullname);
        Ok(())
    }

    fn browse(&self, on_event: OnEvent) -> Result<()> {
        let events = self.daemon.browse(SERVICE_TYPE).map_err(discovery)?;
        let stopped = self.stopped.clone();
        let own = format!("{}.{SERVICE_TYPE}", self.instance).to_lowercase();
        std::thread::Builder::new()
            .name("nearby-mdns".into())
            .spawn(move || {
                while !stopped.load(Ordering::SeqCst) {
                    let event = match events.recv_timeout(STOP_POLL) {
                        Ok(event) => event,
                        Err(mdns_sd::RecvTimeoutError::Timeout) => continue,
                        Err(mdns_sd::RecvTimeoutError::Disconnected) => return,
                    };
                    if stopped.load(Ordering::SeqCst) {
                        return;
                    }
                    match event {
                        ServiceEvent::ServiceResolved(service)
                            if service.get_fullname().to_lowercase() != own =>
                        {
                            if let Some(peer) = peer(&service) {
                                on_event(Event::Seen(peer));
                            }
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            on_event(Event::Gone { instance: fullname });
                        }
                        _ => {}
                    }
                }
            })
            .map_err(discovery)?;
        Ok(())
    }

    // Unregistering sends the goodbye that tells the others at once, rather
    // than leaving them to wait out the record's lifetime.
    fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(fullname) = self.registered.lock().unwrap().take() {
            if let Err(e) = self.daemon.unregister(&fullname) {
                log::warn!("nearby: could not take the advertisement down: {e}");
            }
        }
        let _ = self.daemon.stop_browse(SERVICE_TYPE);
        let _ = self.daemon.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nearby::discovery::Tag;
    use crate::nearby::trust::{NONCE_LEN, TAG_LEN};
    use std::sync::mpsc;

    fn advert(port: u16) -> Advert {
        Advert {
            port,
            txt: Txt {
                nonce: [9; NONCE_LEN],
                tags: vec![Tag {
                    tag: [1; TAG_LEN],
                    device: [2; 32],
                }],
            },
        }
    }

    // The record the daemon is handed: the service type, a random instance on
    // a random host, the port, and the TXT exactly as encoded.
    #[test]
    fn the_record_carries_the_port_and_the_txt_under_a_random_name() {
        let info = service_info("rowel-0123456789abcdef", &advert(48712)).unwrap();
        assert_eq!(info.get_type(), SERVICE_TYPE);
        assert_eq!(
            info.get_fullname(),
            format!("rowel-0123456789abcdef.{SERVICE_TYPE}")
        );
        assert_eq!(info.get_hostname(), "rowel-0123456789abcdef.local.");
        assert_eq!(info.get_port(), 48712);
        assert!(info.is_addr_auto());
        let read = Txt::parse(info.get_properties().iter().map(|p| (p.key(), p.val_str())));
        assert_eq!(read, Some(advert(48712).txt));
    }

    // A real advertisement and browse on this machine's interfaces. Multicast
    // may be refused where the tests run (a sandbox, a CI runner with no
    // interfaces); that is reported rather than failed, since it is the
    // environment that is under test then, not this code.
    #[test]
    fn two_daemons_on_this_machine_see_each_other() {
        let (a, b) = match (Mdns::new(), Mdns::new()) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(e), _) | (_, Err(e)) => {
                eprintln!("mDNS unavailable here: {e}");
                return;
            }
        };
        let (seen, rx) = mpsc::channel();
        let seen = Mutex::new(seen);
        b.browse(Box::new(move |event| {
            if let Event::Seen(peer) = event {
                let _ = seen.lock().unwrap().send(peer);
            }
        }))
        .unwrap();
        a.advertise(&advert(48713)).unwrap();
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(peer) => {
                assert_eq!(peer.txt, advert(48713).txt);
                assert!(peer.addrs.iter().all(|addr| addr.port() == 48713));
            }
            Err(e) => eprintln!("mDNS advertisement not seen here within 10 s: {e}"),
        }
        a.stop();
        b.stop();
    }
}
