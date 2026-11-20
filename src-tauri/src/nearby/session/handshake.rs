//! The first exchange on a connection: who each side is, which lockboxes the
//! two prove they hold, and the devices each refuses (design §5,
//! "Connections").
//!
//! The side that dialled proves first, and only the lockboxes it recognised
//! in the other's advertisement. The side that answered then proves back the
//! ones it verified and no others, so a device that dialled in on a guess, or
//! with nothing in common, is shown no vault id, no proof and no list: what it
//! hears is `hello` and `trust/done`, and the connection ends.
//!
//! A proof that verifies only under a key one side has retired means the
//! other is still on the old password. The newer side answers with a proof
//! under that old key (`trust/proof-retired`), which the older one verifies
//! under its current key, so each learns it from a proof rather than from the
//! other's word; neither syncs the lockbox, and the device shows as needing
//! the new password on both.

use super::advert::Match;
use super::host::{Host, Lockbox};
use super::Shared;
use crate::nearby::channel::{Channel, Control, Role, PROTOCOL_VERSION};
use crate::nearby::trust::{own_proof, verify_peer};
use crate::nearby::{Error, Result};

/// The most proofs one side may send: more lockboxes than anyone holds.
const MAX_PROOFS: usize = 64;

/// The longest device name kept from a `hello`, in characters.
const MAX_NAME_CHARS: usize = 64;

pub(super) enum Handshake {
    /// Lockbox keys were proved one way or the other.
    Established {
        /// The lockboxes proved both ways under their current keys, by vault
        /// id: the ones this connection syncs.
        vaults: Vec<String>,
        /// A lockbox one of the two holds under a key the other retired.
        behind: bool,
    },
    /// Over: nothing in common.
    Over,
}

/// The verdict on the other side's proofs.
#[derive(Default)]
struct Proved {
    /// Proved under this side's current key.
    mine: Vec<String>,
    /// Proved under one of this side's retired keys (its place in
    /// `Lockbox::retired`): the other side is behind.
    peer_behind: Vec<(String, usize)>,
    /// The other side proved, as retired, this side's current key: this
    /// side is behind.
    self_behind: Vec<String>,
}

impl Proved {
    fn is_empty(&self) -> bool {
        self.mine.is_empty() && self.peer_behind.is_empty() && self.self_behind.is_empty()
    }

    fn has(&self, vault: &str) -> bool {
        self.mine.iter().any(|v| v == vault)
            || self.peer_behind.iter().any(|(v, _)| v == vault)
            || self.self_behind.iter().any(|v| v == vault)
    }
}

/// Run the handshake on a fresh channel, with the keys in `lockboxes`.
/// `matches` are what the dialling side recognised in the other's
/// advertisement; the answering side has none.
pub(super) fn run<H: Host>(
    shared: &Shared<H>,
    channel: &mut Channel,
    lockboxes: &[Lockbox],
    matches: &[Match],
) -> Result<Handshake> {
    channel.send_control(&Control::hello(shared.book.device_name()))?;
    let (name, platform) = match channel.recv_control()? {
        // Another version of the protocol: nothing this build can say to it.
        Control::Hello { version, .. } if version != PROTOCOL_VERSION => {
            return Ok(Handshake::Over)
        }
        Control::Hello { name, platform, .. } => {
            (clean_name(&name, &channel.peer().to_string()), platform)
        }
        other => return Err(out_of_step(&other)),
    };
    let proved = match channel.role() {
        Role::Connector => {
            let sent = send_proofs(channel, lockboxes, matches)?;
            read_proofs(channel, lockboxes, Some(&sent))?
        }
        Role::Acceptor => {
            let proved = read_proofs(channel, lockboxes, None)?;
            answer_proofs(channel, lockboxes, &proved)?;
            proved
        }
    };
    if proved.is_empty() {
        return Ok(Handshake::Over);
    }

    // Each keeps the union of the two removed sets, so a Remove on one of the
    // owner's devices reaches the others as they meet. Taken only now the two
    // have proved a key to each other: it is the word of a device that holds
    // the lockbox. One message carries every id, some nine hundred at most.
    let mine = Control::Removed {
        ids: shared.book.removed(),
    };
    if channel.role() == Role::Acceptor {
        channel.send_control(&mine)?;
    }
    match channel.recv_control()? {
        Control::Removed { ids } => shared.learn_removed(&ids),
        other => return Err(out_of_step(&other)),
    }
    if channel.role() == Role::Connector {
        channel.send_control(&mine)?;
    }
    shared
        .book
        .seen(&channel.peer().to_string(), &name, &platform);
    Ok(Handshake::Established {
        behind: !proved.peer_behind.is_empty() || !proved.self_behind.is_empty(),
        vaults: proved.mine,
    })
}

/// The dialling side's proofs: under the current key for every lockbox it
/// recognised, and under each of its retired keys the advertisement carried
/// too, for the other side to learn it is behind. The vault ids sent come
/// back: the only ones an answer is taken for.
fn send_proofs(
    channel: &mut Channel,
    lockboxes: &[Lockbox],
    matches: &[Match],
) -> Result<Vec<String>> {
    let mut sent = Vec::new();
    for found in matches {
        let Some(lockbox) = lockboxes.iter().find(|l| l.vault_id == found.vault_id) else {
            continue;
        };
        let vault = &lockbox.vault_id;
        channel.send_control(&Control::Proof {
            vault: vault.clone(),
            proof: own_proof(channel, &lockbox.current.proof, vault),
        })?;
        for keys in found.retired.iter().filter_map(|&i| lockbox.retired.get(i)) {
            channel.send_control(&Control::RetiredProof {
                vault: vault.clone(),
                proof: own_proof(channel, &keys.proof, vault),
            })?;
        }
        sent.push(vault.clone());
    }
    channel.send_control(&Control::ProofsDone)?;
    Ok(sent)
}

/// The answering side's proofs, for exactly what it verified: its current
/// key for a lockbox both hold under it, and for one where it is the one
/// behind; the retired key the other proved, for one where the other is.
fn answer_proofs(channel: &mut Channel, lockboxes: &[Lockbox], proved: &Proved) -> Result<()> {
    let find = |vault: &str| lockboxes.iter().find(|l| l.vault_id == vault);
    for vault in proved.mine.iter().chain(&proved.self_behind) {
        if let Some(lockbox) = find(vault) {
            channel.send_control(&Control::Proof {
                vault: vault.clone(),
                proof: own_proof(channel, &lockbox.current.proof, vault),
            })?;
        }
    }
    for (vault, retired) in &proved.peer_behind {
        if let Some(keys) = find(vault).and_then(|l| l.retired.get(*retired)) {
            channel.send_control(&Control::RetiredProof {
                vault: vault.clone(),
                proof: own_proof(channel, &keys.proof, vault),
            })?;
        }
    }
    channel.send_control(&Control::ProofsDone)
}

/// Read the other side's proofs up to its `trust/done` and judge each with
/// this side's keys. With `only`, a proof for any other vault id is ignored.
/// One verdict per lockbox: the first proof that verifies.
fn read_proofs(
    channel: &mut Channel,
    lockboxes: &[Lockbox],
    only: Option<&[String]>,
) -> Result<Proved> {
    let mut proved = Proved::default();
    for _ in 0..=MAX_PROOFS {
        let (vault, proof, retired) = match channel.recv_control()? {
            Control::ProofsDone => return Ok(proved),
            Control::Proof { vault, proof } => (vault, proof, false),
            Control::RetiredProof { vault, proof } => (vault, proof, true),
            Control::Unsupported => continue,
            other => return Err(out_of_step(&other)),
        };
        if only.is_some_and(|only| !only.contains(&vault)) || proved.has(&vault) {
            continue;
        }
        let Some(lockbox) = lockboxes.iter().find(|l| l.vault_id == vault) else {
            continue;
        };
        let current = verify_peer(channel, &lockbox.current.proof, &vault, &proof);
        if retired {
            if current {
                proved.self_behind.push(vault);
            }
        } else if current {
            proved.mine.push(vault);
        } else if let Some(at) = lockbox
            .retired
            .iter()
            .position(|keys| verify_peer(channel, &keys.proof, &vault, &proof))
        {
            proved.peer_behind.push((vault, at));
        }
    }
    Err(Error::Protocol(
        "more proofs than any device has lockboxes".into(),
    ))
}

/// A name from the network, for a row in Settings: trimmed, without control
/// characters, at most [`MAX_NAME_CHARS`]; the start of the device id when
/// nothing is left.
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
    Error::Protocol(format!("{message:?} out of step"))
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
