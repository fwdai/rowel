//! What this device's advertisement says, and what another's says to it.

use std::iter::once;

use super::host::{Keys, Lockbox};
use crate::nearby::discovery::{Tag, Txt, MAX_TAGS};
use crate::nearby::identity::DeviceId;
use crate::nearby::trust::{discovery_tag, mask_device_id, NONCE_LEN};

// Current keys first, so that past MAX_TAGS retired ones go before anything this device syncs.
pub(super) fn txt(device: &DeviceId, nonce: [u8; NONCE_LEN], lockboxes: &[Lockbox]) -> Txt {
    let current = lockboxes.iter().map(|lockbox| &lockbox.current);
    let retired = lockboxes.iter().flat_map(|lockbox| &lockbox.retired);
    let tags = current
        .chain(retired)
        .take(MAX_TAGS)
        .map(|keys| Tag {
            tag: discovery_tag(&keys.discovery, &nonce),
            device: mask_device_id(&keys.discovery, &nonce, device.as_bytes()),
        })
        .collect();
    Txt { nonce, tags }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Match {
    // This side's lockbox, by its current key's id, whichever of its keys matched.
    pub id: String,
    pub current: bool,
    // Places in `Lockbox::retired`: the other device is still on a key this one replaced.
    pub retired: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Recognised {
    pub device: DeviceId,
    pub matches: Vec<Match>,
}

pub(super) fn recognise(txt: &Txt, lockboxes: &[Lockbox]) -> Option<Recognised> {
    let mut device = None;
    let mut matches = Vec::new();
    for tag in &txt.tags {
        for (lockbox, retired, keys) in every_key(lockboxes) {
            let Some(named) = unmask(tag, keys, &txt.nonce) else {
                continue;
            };
            // A tag that names another device than the first was lifted from another advertisement.
            if *device.get_or_insert(named) == named {
                add(&mut matches, lockbox.id(), retired);
            }
        }
    }
    Some(Recognised {
        device: device?,
        matches,
    })
}

// With None for the current key and Some(i) for `retired[i]`.
fn every_key(lockboxes: &[Lockbox]) -> impl Iterator<Item = (&Lockbox, Option<usize>, &Keys)> {
    lockboxes.iter().flat_map(|lockbox| {
        let retired = lockbox.retired.iter().enumerate();
        once((None, &lockbox.current))
            .chain(retired.map(|(i, keys)| (Some(i), keys)))
            .map(move |(retired, keys)| (lockbox, retired, keys))
    })
}

fn unmask(tag: &Tag, keys: &Keys, nonce: &[u8; NONCE_LEN]) -> Option<DeviceId> {
    (discovery_tag(&keys.discovery, nonce) == tag.tag)
        .then(|| DeviceId::from_bytes(mask_device_id(&keys.discovery, nonce, &tag.device)))
}

fn add(matches: &mut Vec<Match>, id: &str, retired: Option<usize>) {
    let at = match matches.iter().position(|m| m.id == id) {
        Some(at) => at,
        None => {
            matches.push(Match {
                id: id.into(),
                current: false,
                retired: Vec::new(),
            });
            matches.len() - 1
        }
    };
    let found = &mut matches[at];
    match retired {
        None => found.current = true,
        Some(i) if !found.retired.contains(&i) => found.retired.push(i),
        Some(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nearby::discovery::Txt;
    use crate::nearby::session::tests::{keyed, keys};

    const ME: [u8; 32] = [0xaa; 32];
    const THEM: [u8; 32] = [0xbb; 32];

    // Through the properties, as another device reads it off the network.
    fn advertised(lockboxes: &[Lockbox], nonce: u8) -> Txt {
        let txt = txt(&DeviceId::from_bytes(THEM), [nonce; NONCE_LEN], lockboxes);
        let props = txt.properties();
        Txt::parse(props.iter().map(|(k, v)| (k.as_str(), v.as_str()))).unwrap()
    }

    #[test]
    fn a_shared_lockbox_is_recognised_and_names_the_device() {
        let theirs = [keyed(1, &[]), keyed(9, &[])];
        let mine = [keyed(1, &[]), keyed(2, &[])];
        let read = recognise(&advertised(&theirs, 1), &mine).unwrap();
        assert_eq!(read.device, DeviceId::from_bytes(THEM));
        assert_eq!(
            read.matches,
            [Match {
                id: keys(1).id,
                current: true,
                retired: vec![],
            }]
        );
    }

    #[test]
    fn nothing_in_common_is_nothing_recognised() {
        let theirs = [keyed(7, &[])];
        let mine = [keyed(1, &[]), keyed(2, &[])];
        assert_eq!(recognise(&advertised(&theirs, 1), &mine), None);
        assert_eq!(recognise(&advertised(&[], 1), &mine), None);
    }

    #[test]
    fn a_retired_key_is_recognised_both_ways() {
        let moved_on = [keyed(2, &[1])];
        let behind = [keyed(1, &[])];

        let seen_by_behind = recognise(&advertised(&moved_on, 1), &behind).unwrap();
        assert_eq!(
            seen_by_behind.matches,
            [Match {
                id: keys(1).id,
                current: true,
                retired: vec![],
            }]
        );
        let seen_by_moved_on = recognise(&advertised(&behind, 1), &moved_on).unwrap();
        assert_eq!(
            seen_by_moved_on.matches,
            [Match {
                id: keys(2).id,
                current: false,
                retired: vec![0],
            }]
        );
        let both = recognise(&advertised(&moved_on, 1), &moved_on).unwrap();
        assert!(both.matches[0].current);
        assert_eq!(both.matches[0].retired, [0]);
    }

    #[test]
    fn two_sessions_of_one_device_share_no_tag() {
        let lockboxes = [keyed(1, &[3])];
        let (one, two) = (advertised(&lockboxes, 1), advertised(&lockboxes, 2));
        for (a, b) in one.tags.iter().zip(&two.tags) {
            assert_ne!(a.tag, b.tag);
            assert_ne!(a.device, b.device);
        }
        assert_eq!(
            recognise(&two, &lockboxes).unwrap().device,
            DeviceId::from_bytes(THEM)
        );
    }

    #[test]
    fn tags_naming_two_devices_count_for_the_first() {
        let lockboxes = [keyed(1, &[]), keyed(2, &[])];
        let mut forged = txt(&DeviceId::from_bytes(ME), [1; NONCE_LEN], &lockboxes[..1]);
        let other = txt(&DeviceId::from_bytes(THEM), [1; NONCE_LEN], &lockboxes[1..]);
        forged.tags.extend(other.tags);
        let read = recognise(&forged, &lockboxes).unwrap();
        assert_eq!(read.device, DeviceId::from_bytes(ME));
        assert_eq!(read.matches.len(), 1);
        assert_eq!(read.matches[0].id, lockboxes[0].id());
    }

    #[test]
    fn current_keys_go_first_and_past_the_limit_retired_ones_go() {
        let lockboxes: Vec<Lockbox> = (0..10).map(|n| keyed(n, &[n + 20, n + 40])).collect();
        let txt = txt(&DeviceId::from_bytes(ME), [1; NONCE_LEN], &lockboxes);
        assert_eq!(txt.tags.len(), MAX_TAGS);
        let read = recognise(&txt, &lockboxes).unwrap();
        assert_eq!(read.matches.len(), 10);
        assert!(read.matches.iter().all(|m| m.current));
    }

    #[test]
    fn a_key_seen_twice_counts_once() {
        let mut matches = Vec::new();
        for retired in [Some(1), None, Some(1), Some(0)] {
            add(&mut matches, "k1", retired);
        }
        add(&mut matches, "k2", Some(0));
        assert_eq!(
            matches,
            [
                Match {
                    id: "k1".into(),
                    current: true,
                    retired: vec![1, 0],
                },
                Match {
                    id: "k2".into(),
                    current: false,
                    retired: vec![0],
                },
            ]
        );
    }
}
