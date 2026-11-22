//! Lockbox key proofs over a channel's binding, and an advertisement's tags and masked device ids.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use super::channel::{Channel, Role};

type HmacSha256 = Hmac<Sha256>;

pub const NONCE_LEN: usize = 16;
pub const TAG_LEN: usize = 8;

const MASK_LABEL: &[u8] = b"rowel-nearby device id\0";

pub fn own_proof(channel: &Channel, proof_key: &[u8; 32], vault_id: &str) -> [u8; 32] {
    proof_mac(proof_key, channel.binding(), channel.role(), vault_id)
        .finalize()
        .into_bytes()
        .into()
}

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

// The role keeps a proof from being reflected back to the side that sent it.
fn proof_mac(proof_key: &[u8; 32], binding: &[u8; 32], role: Role, vault_id: &str) -> HmacSha256 {
    let role = match role {
        Role::Connector => 1,
        Role::Acceptor => 2,
    };
    hmac(proof_key, &[binding, &[role], vault_id.as_bytes()])
}

pub fn discovery_tag(discovery_key: &[u8; 32], nonce: &[u8; NONCE_LEN]) -> [u8; TAG_LEN] {
    let digest = hmac(discovery_key, &[nonce]).finalize().into_bytes();
    std::array::from_fn(|i| digest[i])
}

// XOR with a pad: the same call masks and unmasks.
pub fn mask_device_id(
    discovery_key: &[u8; 32],
    nonce: &[u8; NONCE_LEN],
    id: &[u8; 32],
) -> [u8; 32] {
    let pad = hmac(discovery_key, &[MASK_LABEL, nonce])
        .finalize()
        .into_bytes();
    std::array::from_fn(|i| id[i] ^ pad[i])
}

fn hmac(key: &[u8; 32], parts: &[&[u8]]) -> HmacSha256 {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC takes a key of any length");
    for part in parts {
        mac.update(part);
    }
    mac
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
        assert!(!verify_peer(&answered, &OTHER_KEY, "vault-1", &sent));
        assert!(!verify_peer(&answered, &KEY, "vault-2", &sent));
        assert!(!verify_peer(&replayed_on, &KEY, "vault-1", &sent));
        // Reflected back to the side that sent it.
        assert!(!verify_peer(&dialled, &KEY, "vault-1", &sent));
        assert!(!verify_peer(&answered, &KEY, "vault-1", &sent[..16]));
        assert!(!verify_peer(&answered, &KEY, "vault-1", &[]));
    }

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

    // A device in the middle terminates a channel to each side and relays a proof untouched.
    #[test]
    fn a_relay_in_the_middle_yields_no_proof() {
        let (a, b, middle) = (identity(), identity(), identity());
        let (mut a_end, mut middle_to_a) = pair(&a, &middle);
        let (mut middle_to_b, mut b_end) = pair(&middle, &b);
        assert_ne!(a_end.binding(), b_end.binding());

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
