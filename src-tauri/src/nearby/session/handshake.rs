//! The first exchange on a connection: hello, the lockbox key proofs, the removed sets.

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
    Established { vaults: Vec<String>, behind: bool },
    Over,
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
    let proved = prove(channel, lockboxes, matches)?;
    if proved.is_empty() {
        return Ok(Handshake::Over);
    }
    exchange_removed(shared, channel)?;
    shared
        .book
        .seen(&channel.peer().to_string(), &name, &platform);
    Ok(Handshake::Established {
        vaults: proved.mine.iter().map(|l| l.vault_id.clone()).collect(),
        behind: !proved.peer_behind.is_empty() || !proved.self_behind.is_empty(),
    })
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
// verified, so a device that dialled on a guess hears no vault id.
fn prove<'a>(
    channel: &mut Channel,
    lockboxes: &'a [Lockbox],
    matches: &[Match],
) -> Result<Proved<'a>> {
    match channel.role() {
        Role::Connector => {
            let sent: Vec<(&Lockbox, &Match)> = matches
                .iter()
                .filter_map(|found| Some((find(lockboxes, &found.vault_id)?, found)))
                .collect();
            send_proofs(
                channel,
                sent.iter().flat_map(|&(l, found)| proofs_of(l, found)),
            )?;
            read_proofs(channel, &sent.iter().map(|&(l, _)| l).collect::<Vec<_>>())
        }
        Role::Acceptor => {
            let proved = read_proofs(channel, &lockboxes.iter().collect::<Vec<_>>())?;
            send_proofs(channel, proved.answers())?;
            Ok(proved)
        }
    }
}

fn find<'a>(lockboxes: impl IntoIterator<Item = &'a Lockbox>, vault: &str) -> Option<&'a Lockbox> {
    lockboxes.into_iter().find(|l| l.vault_id == vault)
}

enum Proof<'a> {
    Current(&'a Lockbox),
    Retired(&'a Lockbox, &'a Keys),
}

// Under the current key, and under each retired key the advertisement carried.
fn proofs_of<'a>(lockbox: &'a Lockbox, found: &'a Match) -> impl Iterator<Item = Proof<'a>> {
    let retired = found.retired.iter().filter_map(|&i| lockbox.retired.get(i));
    once(Proof::Current(lockbox)).chain(retired.map(|keys| Proof::Retired(lockbox, keys)))
}

impl Proof<'_> {
    fn message(&self, channel: &Channel) -> Control {
        match *self {
            Self::Current(lockbox) => Control::Proof {
                vault: lockbox.vault_id.clone(),
                proof: own_proof(channel, &lockbox.current.proof, &lockbox.vault_id),
            },
            Self::Retired(lockbox, keys) => Control::RetiredProof {
                vault: lockbox.vault_id.clone(),
                proof: own_proof(channel, &keys.proof, &lockbox.vault_id),
            },
        }
    }
}

fn send_proofs<'a>(channel: &mut Channel, proofs: impl Iterator<Item = Proof<'a>>) -> Result<()> {
    for proof in proofs {
        channel.send_control(&proof.message(channel))?;
    }
    channel.send_control(&Control::ProofsDone)
}

fn read_proofs<'a>(channel: &mut Channel, lockboxes: &[&'a Lockbox]) -> Result<Proved<'a>> {
    let mut proved = Proved::default();
    for _ in 0..=MAX_PROOFS {
        let (vault, proof, retired) = match channel.recv_control()? {
            Control::ProofsDone => return Ok(proved),
            Control::Proof { vault, proof } => (vault, proof, false),
            Control::RetiredProof { vault, proof } => (vault, proof, true),
            other => return Err(out_of_step(&other)),
        };
        if proved.has(&vault) {
            continue;
        }
        if let Some(lockbox) = find(lockboxes.iter().copied(), &vault) {
            proved.judge(channel, lockbox, &proof, retired);
        }
    }
    Err(Error::Protocol(
        "more proofs than any device has lockboxes".into(),
    ))
}

#[derive(Default)]
struct Proved<'a> {
    mine: Vec<&'a Lockbox>,
    // Proved under one of this side's retired keys: the other side is behind.
    peer_behind: Vec<(&'a Lockbox, &'a Keys)>,
    // This side's current key, proved as retired: this side is behind.
    self_behind: Vec<&'a Lockbox>,
}

impl<'a> Proved<'a> {
    fn judge(&mut self, channel: &Channel, lockbox: &'a Lockbox, proof: &[u8], retired: bool) {
        let holds = |keys: &Keys| verify_peer(channel, &keys.proof, &lockbox.vault_id, proof);
        match (retired, holds(&lockbox.current)) {
            (false, true) => self.mine.push(lockbox),
            (true, true) => self.self_behind.push(lockbox),
            (false, false) => {
                if let Some(keys) = lockbox.retired.iter().find(|&keys| holds(keys)) {
                    self.peer_behind.push((lockbox, keys));
                }
            }
            (true, false) => {}
        }
    }

    fn all(&self) -> impl Iterator<Item = &'a Lockbox> + '_ {
        let behind = self.peer_behind.iter().map(|&(lockbox, _)| lockbox);
        self.mine
            .iter()
            .chain(&self.self_behind)
            .copied()
            .chain(behind)
    }

    fn is_empty(&self) -> bool {
        self.all().next().is_none()
    }

    fn has(&self, vault: &str) -> bool {
        self.all().any(|lockbox| lockbox.vault_id == vault)
    }

    fn answers(&self) -> impl Iterator<Item = Proof<'a>> + '_ {
        let current = self.mine.iter().chain(&self.self_behind);
        let retired = self.peer_behind.iter();
        current
            .map(|&lockbox| Proof::Current(lockbox))
            .chain(retired.map(|&(lockbox, keys)| Proof::Retired(lockbox, keys)))
    }
}

// Only once a key is proved: the removed set is the word of a device that holds the lockbox.
fn exchange_removed<H: Host>(shared: &Shared<H>, channel: &mut Channel) -> Result<()> {
    let mine = Control::Removed {
        ids: shared.book.removed(),
    };
    let answering = channel.role() == Role::Acceptor;
    if answering {
        channel.send_control(&mine)?;
    }
    match channel.recv_control()? {
        Control::Removed { ids } => shared.learn_removed(&ids),
        other => return Err(out_of_step(&other)),
    }
    if !answering {
        channel.send_control(&mine)?;
    }
    Ok(())
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
    use super::*;

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
}
