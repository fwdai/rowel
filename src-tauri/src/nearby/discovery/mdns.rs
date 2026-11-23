//! Discovery over multicast DNS (`mdns-sd`, on its own thread), for the desktop app.

use std::net::{SocketAddr, SocketAddrV6};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mdns_sd::{
    RecvTimeoutError, ResolvedService, ScopedIp, ServiceDaemon, ServiceEvent, ServiceInfo,
};
use rand::RngCore;

use super::{Advert, Discovery, Event, OnEvent, Peer, Txt, SERVICE_TYPE};
use crate::nearby::{Error, Result};

const STOP_POLL: Duration = Duration::from_millis(250);

pub struct Mdns {
    daemon: ServiceDaemon,
    instance: String,
    advertised: AtomicBool,
    stopped: Arc<AtomicBool>,
}

impl Mdns {
    pub fn new() -> Result<Self> {
        let daemon = ServiceDaemon::new().map_err(discovery)?;
        let mut bytes = [0u8; 8];
        rand::thread_rng().fill_bytes(&mut bytes);
        Ok(Self {
            daemon,
            // Random per session, as is the host name: the machine's own would let anyone follow it.
            instance: format!("rowel-{}", hex::encode(bytes)),
            advertised: AtomicBool::new(false),
            stopped: Arc::default(),
        })
    }

    fn fullname(&self) -> String {
        format!("{}.{SERVICE_TYPE}", self.instance)
    }
}

fn discovery(e: impl std::fmt::Display) -> Error {
    Error::Discovery(e.to_string())
}

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

fn event(found: ServiceEvent, own: &str) -> Option<Event> {
    match found {
        ServiceEvent::ServiceResolved(service)
            if !service.get_fullname().eq_ignore_ascii_case(own) =>
        {
            peer(&service).map(Event::Seen)
        }
        ServiceEvent::ServiceRemoved(_, instance) => Some(Event::Gone { instance }),
        _ => None,
    }
}

fn peer(service: &ResolvedService) -> Option<Peer> {
    let txt = Txt::parse(service.get_properties().iter().filter_map(|prop| {
        let value = std::str::from_utf8(prop.val()?).ok()?;
        Some((prop.key(), value))
    }))?;
    let port = service.get_port();
    let addrs = service
        .get_addresses()
        .iter()
        .map(|ip| match ip {
            ScopedIp::V6(v6) => {
                SocketAddr::V6(SocketAddrV6::new(*v6.addr(), port, 0, v6.scope_id().index))
            }
            ip => SocketAddr::new(ip.to_ip_addr(), port),
        })
        .collect();
    Some(Peer {
        instance: service.get_fullname().to_string(),
        txt,
        addrs,
    })
}

impl Discovery for Mdns {
    // Registered again under the same name, the record is replaced.
    fn advertise(&self, advert: &Advert) -> Result<()> {
        let info = service_info(&self.instance, advert)?;
        self.daemon.register(info).map_err(discovery)?;
        self.advertised.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn browse(&self, on_event: OnEvent) -> Result<()> {
        let events = self.daemon.browse(SERVICE_TYPE).map_err(discovery)?;
        let stopped = self.stopped.clone();
        let own = self.fullname();
        std::thread::Builder::new()
            .name("nearby-mdns".into())
            .spawn(move || {
                while !stopped.load(Ordering::SeqCst) {
                    match events.recv_timeout(STOP_POLL) {
                        Ok(found) if !stopped.load(Ordering::SeqCst) => {
                            if let Some(event) = event(found, &own) {
                                on_event(event);
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                        _ => {}
                    }
                }
            })
            .map_err(discovery)?;
        Ok(())
    }

    // Unregistering sends the goodbye that drops the record from the others at once.
    fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if self.advertised.swap(false, Ordering::SeqCst) {
            if let Err(e) = self.daemon.unregister(&self.fullname()) {
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
    use std::sync::{mpsc, Mutex};

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

    // Multicast may be refused where tests run: that is reported, not failed.
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
