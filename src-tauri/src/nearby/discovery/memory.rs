//! Discovery on a network inside this process, for tests: every member sees the others at 127.0.0.1.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};

use super::{Advert, Discovery, Event, OnEvent, Peer};
use crate::nearby::Result;

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
    pub fn join(&self) -> MemoryDiscovery {
        let mut members = self.members();
        members.next += 1;
        let id = members.next;
        members.by_id.insert(id, Member::default());
        MemoryDiscovery {
            network: self.clone(),
            id,
        }
    }

    pub fn advertised(&self) -> Vec<u16> {
        let members = self.members();
        members
            .by_id
            .values()
            .filter_map(|m| m.advert.as_ref().map(|a| a.port))
            .collect()
    }

    fn members(&self) -> MutexGuard<'_, Members> {
        self.0.lock().unwrap()
    }
}

pub struct MemoryDiscovery {
    network: Network,
    id: u64,
}

impl MemoryDiscovery {
    fn listeners(&self, members: &Members) -> Vec<Arc<OnEvent>> {
        let others = members.by_id.iter().filter(|(id, _)| **id != self.id);
        others.filter_map(|(_, m)| m.on_event.clone()).collect()
    }
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

// Each tells the others outside the lock: a listener may advertise in turn.
impl Discovery for MemoryDiscovery {
    fn advertise(&self, advert: &Advert) -> Result<()> {
        let listeners = {
            let mut members = self.network.members();
            let Some(me) = members.by_id.get_mut(&self.id) else {
                return Ok(());
            };
            me.advert = Some(advert.clone());
            self.listeners(&members)
        };
        for on_event in listeners {
            on_event(Event::Seen(peer(self.id, advert)));
        }
        Ok(())
    }

    fn browse(&self, on_event: OnEvent) -> Result<()> {
        let on_event = Arc::new(on_event);
        let seen: Vec<Peer> = {
            let mut members = self.network.members();
            let Some(me) = members.by_id.get_mut(&self.id) else {
                return Ok(());
            };
            me.on_event = Some(on_event.clone());
            let others = members.by_id.iter().filter(|(id, _)| **id != self.id);
            others
                .filter_map(|(id, m)| m.advert.as_ref().map(|a| peer(*id, a)))
                .collect()
        };
        for peer in seen {
            on_event(Event::Seen(peer));
        }
        Ok(())
    }

    fn stop(&self) {
        let (advertised, listeners) = {
            let mut members = self.network.members();
            let gone = members.by_id.remove(&self.id);
            (
                gone.is_some_and(|m| m.advert.is_some()),
                self.listeners(&members),
            )
        };
        if advertised {
            for on_event in listeners {
                on_event(Event::Gone {
                    instance: instance(self.id),
                });
            }
        }
    }
}
