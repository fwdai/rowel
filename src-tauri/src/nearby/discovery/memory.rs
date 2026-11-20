//! Discovery on a network that exists only in this process, for tests: every
//! member sees every other member's advertisement at once, at 127.0.0.1.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use super::{Advert, Discovery, Event, OnEvent, Peer};
use crate::nearby::Result;

/// One network. Clone it to hand it to several devices.
#[derive(Clone, Default)]
pub struct Network(Arc<Mutex<Members>>);

#[derive(Default)]
struct Members {
    next: u64,
    by_id: HashMap<u64, Member>,
}

#[derive(Default)]
struct Member {
    advert: Option<Advert>,
    on_event: Option<Arc<OnEvent>>,
}

impl Network {
    /// A new member, advertising and browsing nothing yet.
    pub fn join(&self) -> MemoryDiscovery {
        let mut members = self.0.lock().unwrap();
        members.next += 1;
        let id = members.next;
        members.by_id.insert(id, Member::default());
        MemoryDiscovery {
            network: self.clone(),
            id,
        }
    }

    /// The ports advertised on the network now.
    pub fn advertised(&self) -> Vec<u16> {
        let members = self.0.lock().unwrap();
        members
            .by_id
            .values()
            .filter_map(|m| m.advert.as_ref().map(|a| a.port))
            .collect()
    }
}

pub struct MemoryDiscovery {
    network: Network,
    id: u64,
}

fn instance(id: u64) -> String {
    format!("member-{id}")
}

fn peer(id: u64, advert: &Advert) -> Peer {
    Peer {
        instance: instance(id),
        txt: advert.txt.clone(),
        addrs: vec![SocketAddr::from((Ipv4Addr::LOCALHOST, advert.port))],
    }
}

impl Discovery for MemoryDiscovery {
    fn advertise(&self, advert: &Advert) -> Result<()> {
        // Told outside the lock: a listener may well advertise in turn.
        let listeners: Vec<Arc<OnEvent>> = {
            let mut members = self.network.0.lock().unwrap();
            let Some(me) = members.by_id.get_mut(&self.id) else {
                return Ok(());
            };
            me.advert = Some(advert.clone());
            members
                .by_id
                .iter()
                .filter(|(id, _)| **id != self.id)
                .filter_map(|(_, m)| m.on_event.clone())
                .collect()
        };
        for on_event in listeners {
            on_event(Event::Seen(peer(self.id, advert)));
        }
        Ok(())
    }

    fn browse(&self, on_event: OnEvent) -> Result<()> {
        let on_event = Arc::new(on_event);
        let seen: Vec<Peer> = {
            let mut members = self.network.0.lock().unwrap();
            let Some(me) = members.by_id.get_mut(&self.id) else {
                return Ok(());
            };
            me.on_event = Some(on_event.clone());
            members
                .by_id
                .iter()
                .filter(|(id, _)| **id != self.id)
                .filter_map(|(id, m)| m.advert.as_ref().map(|a| peer(*id, a)))
                .collect()
        };
        for peer in seen {
            on_event(Event::Seen(peer));
        }
        Ok(())
    }

    fn stop(&self) {
        let (was_advertised, listeners): (bool, Vec<Arc<OnEvent>>) = {
            let mut members = self.network.0.lock().unwrap();
            let gone = members.by_id.remove(&self.id);
            (
                gone.is_some_and(|m| m.advert.is_some()),
                members
                    .by_id
                    .values()
                    .filter_map(|m| m.on_event.clone())
                    .collect(),
            )
        };
        if was_advertised {
            for on_event in listeners {
                on_event(Event::Gone {
                    instance: instance(self.id),
                });
            }
        }
    }
}
