//! Trust between two devices on a [`Channel`]: the proof that a device holds a
//! lockbox's key, with an HMAC under a subkey of it over the session's binding
//! ([`own_proof`]), which is what lets two devices sync that lockbox; and the tags
//! and masked device id an advertisement carries.
//!
//! The channel's TLS handshake has already done the key agreement and proved
//! each side holds the key its device id names; what is added here is HMACs
//! over the binding it exported.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use super::channel::{Channel, Role};

type HmacSha256 = Hmac<Sha256>;

/// This side's proof for `vault_id` on `channel`, to send as `trust/proof`:
/// `HMAC-SHA256(proof key, binding ‖ role ‖ vault id)`, the proof that the
/// side in `role` holds the key of the vault `vault_id`, good on the session
/// whose binding this is and nowhere else. The role keeps a proof from being
/// handed back to the side that sent it as the other side's.
pub fn own_proof(channel: &Channel, proof_key: &[u8; 32], vault_id: &str) -> [u8; 32] {
    proof_mac(proof_key, channel.binding(), channel.role(), vault_id)
        .finalize()
        .into_bytes()
        .into()
}

/// Whether the peer's `claimed` proof for `vault_id` on `channel` holds,
/// compared in constant time.
pub fn verify_peer(
    channel: &Channel,
    proof_key: &[u8; 32],
    vault_id: &str,
    claimed: &[u8],
) -> bool {
    proof_mac(
        proof_key,
        channel.binding(),
        channel.role().other(),
        vault_id,
    )
    .verify_slice(claimed)
    .is_ok()
}

fn proof_mac(proof_key: &[u8; 32], binding: &[u8; 32], role: Role, vault_id: &str) -> HmacSha256 {
    let mut mac = HmacSha256::new_from_slice(proof_key).expect("HMAC takes a key of any length");
    mac.update(binding);
    mac.update(&[match role {
        Role::Connector => 1,
        Role::Acceptor => 2,
    }]);
    mac.update(vault_id.as_bytes());
    mac
}

/// How long the random nonce of one advertisement is (`nearby::discovery`).
pub const NONCE_LEN: usize = 16;

/// How long a lockbox's tag in an advertisement is.
pub const TAG_LEN: usize = 8;

const MASK_LABEL: &[u8] = b"rowel-nearby device id\0";

/// `HMAC-SHA256(discovery key, nonce)` truncated to [`TAG_LEN`] bytes: one
/// lockbox key in an advertisement. A device holding the key recomputes it
/// over the advertisement's nonce and recognises it; to anyone else it is
/// noise, and with a new nonce every session it links nothing to the last.
pub fn discovery_tag(discovery_key: &[u8; 32], nonce: &[u8; NONCE_LEN]) -> [u8; TAG_LEN] {
    let mut mac =
        HmacSha256::new_from_slice(discovery_key).expect("HMAC takes a key of any length");
    mac.update(nonce);
    let digest = mac.finalize().into_bytes();
    digest[..TAG_LEN]
        .try_into()
        .expect("a SHA-256 MAC has TAG_LEN bytes")
}

/// The advertising device's id, readable only by a holder of the discovery
/// key: the id XOR `HMAC-SHA256(discovery key, label ‖ nonce)`. What lets a
/// device that recognises a tag know which device it is before it dials
/// (who dials, a device it refuses, the id to dial expecting), while an id
/// in the clear would let anyone follow the device from session to session.
/// Masking and unmasking are the same operation.
pub fn mask_device_id(
    discovery_key: &[u8; 32],
    nonce: &[u8; NONCE_LEN],
    id: &[u8; 32],
) -> [u8; 32] {
    let mut mac =
        HmacSha256::new_from_slice(discovery_key).expect("HMAC takes a key of any length");
    mac.update(MASK_LABEL);
    mac.update(nonce);
    let pad = mac.finalize().into_bytes();
    let mut masked = [0u8; 32];
    for (out, (a, b)) in masked.iter_mut().zip(id.iter().zip(pad.iter())) {
        *out = a ^ b;
    }
    masked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nearby::channel::tests::pair;
    use crate::nearby::channel::{Control, MAX_FRAME};
    use crate::nearby::identity::tests::identity;

    const KEY: [u8; 32] = [7; 32];
    const OTHER_KEY: [u8; 32] = [9; 32];

    #[test]
    fn a_key_proof_holds_for_its_key_session_and_role_only() {
        let (a, b) = (identity(), identity());
        let (dialled, answered) = pair(&a, &b);
        let (_, replayed_on) = pair(&a, &b);

        let sent = own_proof(&dialled, &KEY, "vault-1");
        assert!(verify_peer(&answered, &KEY, "vault-1", &sent));
        // Another key, another vault id.
        assert!(!verify_peer(&answered, &OTHER_KEY, "vault-1", &sent));
        assert!(!verify_peer(&answered, &KEY, "vault-2", &sent));
        // Replayed into another session of the same two devices.
        assert!(!verify_peer(&replayed_on, &KEY, "vault-1", &sent));
        // Reflected: handed back to the side that sent it, as the other's.
        assert!(!verify_peer(&dialled, &KEY, "vault-1", &sent));
        // Truncated or empty.
        assert!(!verify_peer(&answered, &KEY, "vault-1", &sent[..16]));
        assert!(!verify_peer(&answered, &KEY, "vault-1", &[]));
    }

    // A tag says one thing to the holder of its key and nothing to anyone
    // else, and nothing that carries over to the next session's nonce.
    #[test]
    fn a_tag_is_its_keys_and_its_nonces_and_the_mask_comes_off_with_the_key() {
        let (n1, n2) = ([1u8; NONCE_LEN], [2u8; NONCE_LEN]);
        let tag = discovery_tag(&KEY, &n1);
        assert_eq!(tag, discovery_tag(&KEY, &n1));
        assert_ne!(tag, discovery_tag(&OTHER_KEY, &n1));
        assert_ne!(tag, discovery_tag(&KEY, &n2));

        let id = *identity().device_id().as_bytes();
        let masked = mask_device_id(&KEY, &n1, &id);
        assert_ne!(masked, id);
        assert_ne!(masked, mask_device_id(&KEY, &n2, &id));
        assert_eq!(mask_device_id(&KEY, &n1, &masked), id);
        assert_ne!(mask_device_id(&OTHER_KEY, &n1, &masked), id);
    }

    // A device in the middle terminates two channels, one to each honest
    // device, and relays a key proof untouched.
    #[test]
    fn a_relay_in_the_middle_yields_no_proof() {
        let (a, b, middle) = (identity(), identity(), identity());
        let (mut a_end, mut middle_to_a) = pair(&a, &middle);
        let (mut middle_to_b, mut b_end) = pair(&middle, &b);
        assert_ne!(a_end.binding(), b_end.binding());

        // A's proof, relayed to B as the middle's own.
        a_end
            .send_control(&Control::Proof {
                vault: "vault-1".into(),
                proof: own_proof(&a_end, &KEY, "vault-1"),
            })
            .unwrap();
        let relayed = middle_to_a.recv(MAX_FRAME).unwrap();
        middle_to_b.send(&relayed).unwrap();
        let Control::Proof { vault, proof } = b_end.recv_control().unwrap() else {
            panic!("expected the relayed proof");
        };
        assert!(!verify_peer(&b_end, &KEY, &vault, &proof));
    }
}
