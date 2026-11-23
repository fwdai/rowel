//! The first exchange on a connection: hello, then the lockbox key proofs.

use std::iter::once;

use super::advert::Match;
use super::host::{Host, Keys, Lockbox};
use super::Shared;
use crate::nearby::channel::{Channel, Control, Role, PROTOCOL_VERSION};
use crate::nearby::trust::{own_proof, verify_peer};
use crate::nearby::{Error, Result};

const MAX_PROOFS: usize = 64;

const MAX_NAME_CHARS: usize = 64;

pub(super) enum Handshake {
    Established(Established),
    Over,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Established {
    pub ids: Vec<String>,
    // This side's lockboxes on a key the peer has retired.
    pub behind: Vec<String>,
    // The peer's key id, and this side's lockbox that retired it.
    pub ahead: Vec<(String, String)>,
    // What the answering side asked for with its proofs: a lockbox, and the packs it has.
    pub asks: Vec<(String, Vec<String>)>,
}

pub(super) fn run<H: Host>(
    shared: &Shared<H>,
    channel: &mut Channel,
    lockboxes: &[Lockbox],
    matches: &[Match],
) -> Result<Handshake> {
    let Some((name, platform)) = hello(channel, shared.book.device_name())? else {
        return Ok(Handshake::Over);
    };
    let kept = shared.host.kept();
    let proved = prove(channel, lockboxes, matches, |id| kept.sealed_under(id))?;
    if proved.verified.is_empty() {
        return Ok(Handshake::Over);
    }
    shared
        .book
        .seen(&channel.peer().to_string(), &name, &platform);
    Ok(Handshake::Established(proved.established()))
}

// The peer's name and platform; None when it speaks another protocol version.
fn hello(channel: &mut Channel, name: &str) -> Result<Option<(String, String)>> {
    channel.send_control(&Control::hello(name))?;
    match channel.recv_control()? {
        Control::Hello { version, .. } if version != PROTOCOL_VERSION => Ok(None),
        Control::Hello { name, platform, .. } => Ok(Some((
            clean_name(&name, &channel.peer().to_string()),
            platform,
        ))),
        other => Err(out_of_step(&other)),
    }
}

// The dialling side proves what it recognised; the answering side proves back only what
// verified, so a device that dialled on a guess hears no key id. From here on the
// answering side only answers, so what it is behind on it asks for now.
fn prove<'a>(
    channel: &mut Channel,
    lockboxes: &'a [Lockbox],
    matches: &[Match],
    kept: impl Fn(&str) -> Vec<String>,
) -> Result<Proved<'a>> {
    match channel.role() {
        Role::Connector => {
            let sent: Vec<(&Lockbox, &Match)> = matches
                .iter()
                .filter_map(|found| Some((lockboxes.iter().find(|l| l.id() == found.id)?, found)))
                .collect();
            send_proofs(
                channel,
                sent.iter().flat_map(|&(l, found)| proofs_of(l, found)),
            )?;
            channel.send_control(&Control::ProofsDone)?;
            read_proofs(channel, &sent.iter().map(|&(l, _)| l).collect::<Vec<_>>())
        }
        Role::Acceptor => {
            let proved = read_proofs(channel, &lockboxes.iter().collect::<Vec<_>>())?;
            send_proofs(channel, proved.answers())?;
            for id in proved.behind() {
                let (lockbox, have) = (id.to_owned(), kept(id));
                channel.send_control(&Control::Ask { lockbox, have })?;
            }
            channel.send_control(&Control::ProofsDone)?;
            Ok(proved)
        }
    }
}

#[derive(Clone, Copy)]
enum Key<'a> {
    Current(&'a Lockbox),
    Retired(&'a Lockbox, &'a Keys),
}

impl<'a> Key<'a> {
    fn lockbox(self) -> &'a Lockbox {
        match self {
            Self::Current(lockbox) | Self::Retired(lockbox, _) => lockbox,
        }
    }

    fn keys(self) -> &'a Keys {
        match self {
            Self::Current(lockbox) => &lockbox.current,
            Self::Retired(_, keys) => keys,
        }
    }

    fn proof(self, channel: &Channel) -> Control {
        let keys = self.keys();
        let (lockbox, proof) = (keys.id.clone(), own_proof(channel, &keys.proof, &keys.id));
        match self {
            Self::Current(_) => Control::Proof { lockbox, proof },
            Self::Retired(..) => Control::RetiredProof { lockbox, proof },
        }
    }
}

fn key_named<'a>(lockboxes: &[&'a Lockbox], id: &str) -> Option<Key<'a>> {
    lockboxes.iter().find_map(|&lockbox| {
        if lockbox.id() == id {
            return Some(Key::Current(lockbox));
        }
        let retired = lockbox.retired.iter().find(|keys| keys.id == id);
        retired.map(|keys| Key::Retired(lockbox, keys))
    })
}

// Under the current key, and under each retired key the advertisement carried.
fn proofs_of<'a>(lockbox: &'a Lockbox, found: &'a Match) -> impl Iterator<Item = Key<'a>> {
    let retired = found.retired.iter().filter_map(|&i| lockbox.retired.get(i));
    once(Key::Current(lockbox)).chain(retired.map(|keys| Key::Retired(lockbox, keys)))
}

fn send_proofs<'a>(channel: &mut Channel, keys: impl Iterator<Item = Key<'a>>) -> Result<()> {
    for key in keys {
        channel.send_control(&key.proof(channel))?;
    }
    Ok(())
}

fn read_proofs<'a>(channel: &mut Channel, lockboxes: &[&'a Lockbox]) -> Result<Proved<'a>> {
    let mut proved = Proved::default();
    for _ in 0..=MAX_PROOFS {
        let (id, proof, retired) = match channel.recv_control()? {
            Control::ProofsDone => return Ok(proved),
            Control::Proof { lockbox, proof } => (lockbox, proof, false),
            Control::RetiredProof { lockbox, proof } => (lockbox, proof, true),
            Control::Ask { lockbox, have } => {
                proved.asks.push((lockbox, have));
                continue;
            }
            other => return Err(out_of_step(&other)),
        };
        if let Some(key) = key_named(lockboxes, &id) {
            proved.judge(channel, key, &proof, retired);
        }
    }
    Err(Error::Protocol(
        "more proofs than any device has lockboxes".into(),
    ))
}

// Each proof that verified: the key here it verified under, and whether it came as retired.
#[derive(Default)]
struct Proved<'a> {
    verified: Vec<(Key<'a>, bool)>,
    asks: Vec<(String, Vec<String>)>,
}

impl<'a> Proved<'a> {
    // A retired proof counts only under this side's current key; one verdict per lockbox.
    fn judge(&mut self, channel: &Channel, key: Key<'a>, proof: &[u8], retired: bool) {
        let ignored = (retired && matches!(key, Key::Retired(..))) || self.has(key.lockbox());
        let keys = key.keys();
        if !ignored && verify_peer(channel, &keys.proof, &keys.id, proof) {
            self.verified.push((key, retired));
        }
    }

    fn behind(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.verified
            .iter()
            .filter_map(|&(key, retired)| match key {
                Key::Current(lockbox) if retired => Some(lockbox.id()),
                _ => None,
            })
    }

    fn established(self) -> Established {
        let mut found = Established {
            asks: self.asks,
            ..Established::default()
        };
        for (key, retired) in self.verified {
            let mine = key.lockbox().id().to_owned();
            match key {
                Key::Current(_) if retired => found.behind.push(mine),
                Key::Current(_) => found.ids.push(mine),
                Key::Retired(_, keys) => found.ahead.push((keys.id.clone(), mine)),
            }
        }
        found
    }

    fn has(&self, lockbox: &Lockbox) -> bool {
        self.verified
            .iter()
            .any(|(key, _)| key.lockbox().id() == lockbox.id())
    }

    // Each under the key its proof verified under.
    fn answers(&self) -> impl Iterator<Item = Key<'a>> + '_ {
        self.verified.iter().map(|&(key, _)| key)
    }
}

fn clean_name(name: &str, device_id: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect();
    if cleaned.is_empty() {
        device_id.chars().take(8).collect()
    } else {
        cleaned
    }
}

pub(super) fn out_of_step(message: &Control) -> Error {
    Error::Protocol(format!("{} out of step", message.kind()))
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;
    use crate::nearby::channel::tests::pair;
    use crate::nearby::identity::tests::identity;
    use crate::nearby::session::advert;
    use crate::nearby::session::tests::{keyed, keys};
    use crate::nearby::trust::NONCE_LEN;

    #[test]
    fn a_name_from_the_network_is_cleaned_and_bounded() {
        assert_eq!(clean_name("  Studio Mac\n", "abcdef0123"), "Studio Mac");
        assert_eq!(clean_name("\u{7}\u{1b}[31m", "abcdef0123"), "[31m");
        assert_eq!(clean_name(" \t", "abcdef0123"), "abcdef01");
        assert_eq!(
            clean_name(&"é".repeat(100), "x").chars().count(),
            MAX_NAME_CHARS
        );
    }

    #[test]
    fn a_key_is_found_by_its_id_whether_current_or_retired() {
        let (one, two) = (keyed(1, &[2, 3]), keyed(4, &[]));
        let held = [&one, &two];
        assert!(matches!(
            key_named(&held, &keys(4).id),
            Some(Key::Current(l)) if l.id() == two.id()
        ));
        assert!(matches!(
            key_named(&held, &keys(3).id),
            Some(Key::Retired(l, k)) if l.id() == one.id() && k.id == keys(3).id
        ));
        assert!(key_named(&held, &keys(5).id).is_none());
    }

    const HELD: &str = "held";

    // What each side established, the dialler's first.
    fn shake(dialler: &[Lockbox], answerer: &[Lockbox]) -> [Established; 2] {
        let (a, b) = (identity(), identity());
        let (mut dialled, mut answered) = pair(&a, &b);
        let txt = advert::txt(b.device_id(), [1; NONCE_LEN], answerer);
        let matches = advert::recognise(&txt, dialler).map_or_else(Vec::new, |found| found.matches);
        let kept = |_: &str| vec![HELD.to_string()];
        thread::scope(|s| {
            let answering = s.spawn(|| prove(&mut answered, answerer, &[], kept));
            let dialling = prove(&mut dialled, dialler, &matches, kept);
            [dialling, answering.join().unwrap()].map(|proved| proved.unwrap().established())
        })
    }

    #[test]
    fn a_device_on_a_retired_key_is_behind_whichever_dials_and_asks_when_it_answers() {
        let (moved_on, on_old) = ([keyed(2, &[1])], [keyed(1, &[])]);
        let (new, old) = (keys(2).id, keys(1).id);
        let behind = Established {
            behind: vec![old.clone()],
            ..Established::default()
        };
        let ahead = |asks| Established {
            ahead: vec![(old.clone(), new.clone())],
            asks,
            ..Established::default()
        };
        let asked = vec![(old.clone(), vec![HELD.to_string()])];
        assert_eq!(shake(&moved_on, &on_old), [ahead(asked), behind.clone()]);
        assert_eq!(shake(&on_old, &moved_on), [behind, ahead(Vec::new())]);
        let level = Established {
            ids: vec![new.clone()],
            ..Established::default()
        };
        assert_eq!(shake(&moved_on, &moved_on), [level.clone(), level]);
    }
}
