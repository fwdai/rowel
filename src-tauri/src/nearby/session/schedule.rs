//! When to dial: per advertisement seen, when a sync with its device is next due.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use super::advert::{self, Recognised};
use super::host::Lockbox;
use crate::nearby::discovery::{Peer, Txt};
use crate::nearby::identity::DeviceId;

pub(super) struct Schedule {
    period: Duration,
    seen: HashMap<String, Seen>,
}

struct Seen {
    txt: Txt,
    addrs: Vec<SocketAddr>,
    found: Option<Recognised>,
    due: Instant,
    dialling: bool,
}

pub(super) struct Dial {
    pub instance: String,
    pub found: Recognised,
    pub addrs: Vec<SocketAddr>,
}

impl Schedule {
    pub fn new(period: Duration) -> Self {
        Self {
            period,
            seen: HashMap::new(),
        }
    }

    pub fn seen(&mut self, peer: Peer, lockboxes: &[Lockbox]) {
        let found = advert::recognise(&peer.txt, lockboxes);
        if let Some(seen) = self.seen.get_mut(&peer.instance) {
            if seen.found != found {
                seen.due = Instant::now();
            }
            (seen.txt, seen.addrs, seen.found) = (peer.txt, peer.addrs, found);
            return;
        }
        log::info!(
            "nearby: saw {}, recognised: {}",
            peer.instance,
            found.is_some()
        );
        let seen = Seen {
            txt: peer.txt,
            addrs: peer.addrs,
            found,
            due: Instant::now(),
            dialling: false,
        };
        self.seen.insert(peer.instance, seen);
    }

    pub fn gone(&mut self, instance: &str) {
        self.seen.remove(instance);
    }

    pub fn rekeyed(&mut self, lockboxes: &[Lockbox]) {
        for seen in self.seen.values_mut() {
            seen.found = advert::recognise(&seen.txt, lockboxes);
        }
        self.due_now();
    }

    pub fn due_now(&mut self) {
        let now = Instant::now();
        for seen in self.seen.values_mut() {
            seen.due = now;
        }
    }

    pub fn take_due(&mut self, may_dial: impl Fn(&DeviceId) -> bool) -> Vec<Dial> {
        let now = Instant::now();
        let mut due = Vec::new();
        for (instance, seen) in &mut self.seen {
            let Some(found) = &seen.found else {
                continue;
            };
            if seen.dialling || seen.due > now || !may_dial(&found.device) {
                continue;
            }
            seen.dialling = true;
            seen.due = now + self.period;
            due.push(Dial {
                instance: instance.clone(),
                found: found.clone(),
                addrs: seen.addrs.clone(),
            });
        }
        due
    }

    pub fn dialled(&mut self, instance: &str) {
        if let Some(seen) = self.seen.get_mut(instance) {
            seen.dialling = false;
        }
    }

    // A sync asked for while this one ran stays due. The side that answered waits
    // half a period longer, so one of a pair keeps dialling and the other stands by.
    pub fn ended(&mut self, device: &DeviceId, answered: bool) {
        let now = Instant::now();
        let wait = if answered {
            self.period * 3 / 2
        } else {
            self.period
        };
        for seen in self.seen.values_mut() {
            if seen.due > now && seen.found.as_ref().is_some_and(|f| f.device == *device) {
                seen.due = now + wait;
            }
        }
    }

    pub fn here(&self) -> HashSet<String> {
        self.seen
            .values()
            .filter_map(|seen| seen.found.as_ref())
            .map(|found| found.device.to_string())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use zeroize::Zeroizing;

    use super::*;
    use crate::nearby::session::host::Keys;

    const INSTANCE: &str = "rowel-1";

    fn seeing(device: &DeviceId) -> Schedule {
        let keys = Keys {
            proof: Zeroizing::new([1; 32]),
            discovery: Zeroizing::new([2; 32]),
        };
        let lockboxes = [Lockbox {
            vault_id: "v".into(),
            current: keys,
            retired: Vec::new(),
        }];
        let peer = Peer {
            instance: INSTANCE.into(),
            txt: advert::txt(device, [0; 16], &lockboxes),
            addrs: Vec::new(),
        };
        let mut schedule = Schedule::new(Duration::from_secs(3600));
        schedule.seen(peer, &lockboxes);
        schedule
    }

    fn dial_once(
        schedule: &mut Schedule,
        device: &DeviceId,
        meanwhile: impl FnOnce(&mut Schedule),
    ) {
        assert_eq!(schedule.take_due(|_| true).len(), 1);
        meanwhile(schedule);
        schedule.ended(device, false);
        schedule.dialled(INSTANCE);
    }

    #[test]
    fn a_sync_that_ended_is_not_due_again_before_the_period() {
        let device = DeviceId::from_bytes([7; 32]);
        let mut schedule = seeing(&device);
        dial_once(&mut schedule, &device, |_| {});
        assert!(schedule.take_due(|_| true).is_empty());
    }

    #[test]
    fn a_sync_asked_for_while_one_runs_stays_due() {
        let device = DeviceId::from_bytes([7; 32]);
        let mut schedule = seeing(&device);
        dial_once(&mut schedule, &device, Schedule::due_now);
        assert_eq!(schedule.take_due(|_| true).len(), 1);
    }

    #[test]
    fn the_side_that_answered_waits_longer_than_the_one_that_dialled() {
        let device = DeviceId::from_bytes([7; 32]);
        let mut schedule = seeing(&device);
        schedule.take_due(|_| true);
        schedule.ended(&device, false);
        let dialled = schedule.seen[INSTANCE].due;
        schedule.ended(&device, true);
        assert!(schedule.seen[INSTANCE].due > dialled + schedule.period / 4);
    }
}
