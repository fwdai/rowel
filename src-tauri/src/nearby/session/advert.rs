//! What this device's advertisement says, and what another's says to it.

use super::host::Lockbox;
use crate::nearby::discovery::{Tag, Txt, MAX_TAGS};
use crate::nearby::identity::DeviceId;
use crate::nearby::trust::{discovery_tag, mask_device_id, NONCE_LEN};

/// This device's TXT: a tag and the masked id for every key of every
/// lockbox, the current keys first so that a device with more than
/// [`MAX_TAGS`] keys drops retired ones before anything it syncs.
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

/// One of this device's lockboxes, recognised in another's advertisement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Match {
    pub vault_id: String,
    /// Its current key is among the ones advertised.
    pub current: bool,
    /// Which of its retired keys are, by their place in `Lockbox::retired`:
    /// the other device is still on a key this one has replaced.
    pub retired: Vec<usize>,
}

/// Another device's advertisement, read with this device's keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Recognised {
    /// The device, as the masks under the recognised keys name it.
    pub device: DeviceId,
    pub matches: Vec<Match>,
}

/// Which of `lockboxes` the advertisement `txt` carries a key of, and the
/// device it names. `None` when it carries none: it is a stranger's, or a
/// device's that shares nothing with this one, and the two are not told
/// apart. Every recognised tag of one advertisement has to unmask to the same
/// device; one that names another is not believed.
pub(super) fn recognise(txt: &Txt, lockboxes: &[Lockbox]) -> Option<Recognised> {
    let mut device: Option<DeviceId> = None;
    let mut matches: Vec<Match> = Vec::new();
    for tag in &txt.tags {
        for lockbox in lockboxes {
            let keys = std::iter::once((None, &lockbox.current)).chain(
                lockbox
                    .retired
                    .iter()
                    .enumerate()
                    .map(|(i, k)| (Some(i), k)),
            );
            for (retired, keys) in keys {
                if discovery_tag(&keys.discovery, &txt.nonce) != tag.tag {
                    continue;
                }
                let named =
                    DeviceId::from_bytes(mask_device_id(&keys.discovery, &txt.nonce, &tag.device));
                if *device.get_or_insert(named) != named {
                    continue;
                }
                let found = match matches.iter().position(|m| m.vault_id == lockbox.vault_id) {
                    Some(at) => &mut matches[at],
                    None => {
                        matches.push(Match {
                            vault_id: lockbox.vault_id.clone(),
                            current: false,
                            retired: Vec::new(),
                        });
                        matches.last_mut().expect("just pushed")
                    }
                };
                match retired {
                    None => found.current = true,
                    Some(i) if !found.retired.contains(&i) => found.retired.push(i),
                    Some(_) => {}
                }
            }
        }
    }
    Some(Recognised {
        device: device?,
        matches,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nearby::discovery::Txt;
    use crate::nearby::session::host::Keys;
    use zeroize::Zeroizing;

    fn keys(n: u8) -> Keys {
        Keys {
            proof: Zeroizing::new([n; 32]),
            discovery: Zeroizing::new([n.wrapping_add(100); 32]),
        }
    }

    fn lockbox(vault: &str, current: u8, retired: &[u8]) -> Lockbox {
        Lockbox {
            vault_id: vault.into(),
            current: keys(current),
            retired: retired.iter().map(|&n| keys(n)).collect(),
        }
    }

    const ME: [u8; 32] = [0xaa; 32];
    const THEM: [u8; 32] = [0xbb; 32];

    fn advertised(lockboxes: &[Lockbox], nonce: u8) -> Txt {
        // Through the properties, as another device reads it off the network.
        let txt = txt(&DeviceId::from_bytes(THEM), [nonce; NONCE_LEN], lockboxes);
        let props = txt.properties();
        Txt::parse(props.iter().map(|(k, v)| (k.as_str(), v.as_str()))).unwrap()
    }

    #[test]
    fn a_shared_lockbox_is_recognised_and_names_the_device() {
        let theirs = [lockbox("v1", 1, &[]), lockbox("v9", 9, &[])];
        let mine = [lockbox("v1", 1, &[]), lockbox("v2", 2, &[])];
        let read = recognise(&advertised(&theirs, 1), &mine).unwrap();
        assert_eq!(read.device, DeviceId::from_bytes(THEM));
        assert_eq!(
            read.matches,
            [Match {
                vault_id: "v1".into(),
                current: true,
                retired: vec![],
            }]
        );
    }

    #[test]
    fn nothing_in_common_is_nothing_recognised() {
        let theirs = [lockbox("v1", 7, &[])];
        let mine = [lockbox("v1", 1, &[]), lockbox("v2", 2, &[])];
        assert_eq!(recognise(&advertised(&theirs, 1), &mine), None);
        assert_eq!(recognise(&advertised(&[], 1), &mine), None);
    }

    // A device that changed the password advertises its retired key too; one
    // still on that key recognises it as current, and the other way round.
    #[test]
    fn a_retired_key_is_recognised_both_ways() {
        let moved_on = [lockbox("v1", 2, &[1])];
        let behind = [lockbox("v1", 1, &[])];

        let seen_by_behind = recognise(&advertised(&moved_on, 1), &behind).unwrap();
        assert_eq!(
            seen_by_behind.matches,
            [Match {
                vault_id: "v1".into(),
                current: true,
                retired: vec![],
            }]
        );
        let seen_by_moved_on = recognise(&advertised(&behind, 1), &moved_on).unwrap();
        assert_eq!(
            seen_by_moved_on.matches,
            [Match {
                vault_id: "v1".into(),
                current: false,
                retired: vec![0],
            }]
        );
        // Both on the new key, one still carrying the old: current, and the
        // retired one recognised as well.
        let both = recognise(&advertised(&moved_on, 1), &moved_on).unwrap();
        assert!(both.matches[0].current);
        assert_eq!(both.matches[0].retired, [0]);
    }

    // The same lockboxes under another nonce: other tags, other masks.
    #[test]
    fn two_sessions_of_one_device_share_no_tag() {
        let lockboxes = [lockbox("v1", 1, &[3])];
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

    // A tag lifted from another advertisement and put beside one's own does
    // not change whose advertisement it is.
    #[test]
    fn tags_naming_two_devices_count_for_the_first() {
        let lockboxes = [lockbox("v1", 1, &[]), lockbox("v2", 2, &[])];
        let mut forged = txt(&DeviceId::from_bytes(ME), [1; NONCE_LEN], &lockboxes[..1]);
        let other = txt(&DeviceId::from_bytes(THEM), [1; NONCE_LEN], &lockboxes[1..]);
        forged.tags.extend(other.tags);
        let read = recognise(&forged, &lockboxes).unwrap();
        assert_eq!(read.device, DeviceId::from_bytes(ME));
        assert_eq!(read.matches.len(), 1);
        assert_eq!(read.matches[0].vault_id, "v1");
    }

    #[test]
    fn current_keys_go_first_and_past_the_limit_retired_ones_go() {
        let lockboxes: Vec<Lockbox> = (0..10)
            .map(|n| lockbox(&format!("v{n}"), n, &[n + 20, n + 40]))
            .collect();
        let txt = txt(&DeviceId::from_bytes(ME), [1; NONCE_LEN], &lockboxes);
        assert_eq!(txt.tags.len(), MAX_TAGS);
        let read = recognise(&txt, &lockboxes).unwrap();
        assert_eq!(read.matches.len(), 10);
        assert!(read.matches.iter().all(|m| m.current));
    }
}
