//! Versioned key-derivation primitive.
//!
//! A [`KdfParams`] descriptor names the algorithm and its parameters and is
//! meant to be stored (as a JSON string) alongside a vault — e.g. in the
//! store's `meta` table — so a derivation can be reproduced from what was
//! persisted. [`derive`] dispatches on that descriptor.
//!
//! Two algorithms are supported:
//! - **Argon2id** — the default for new vaults (memory-hard, OWASP-reasonable).
//! - **PBKDF2-HMAC-SHA512** — a configurable fallback and the shape the legacy
//!   `.swftx` vaults derive with. The byte-compatible *legacy* read path lives
//!   in [`super::Cryptor`] and is untouched by this module.
//!
//! A third descriptor, [`KdfParams::External`], derives nothing: it names a
//! master a Rowel Server holds for an org lockbox. It exists so the sidecar
//! and every pack header keep one shape whoever minted the key, and so the
//! generation bookkeeping of a re-key works the same for a master the server
//! rotates as for a password the user changes.
//!
//! This is a self-contained primitive: it turns password bytes into ≥32 bytes
//! of key material suitable as HKDF input. Splitting that output into the
//! SQLCipher and payload subkeys (HKDF) is done by the store integration, not
//! here. Callers own the choice of what `password` bytes to feed: for Argon2id,
//! feed the master password **directly** (no `hash_secret` pre-hash); the
//! `base64(SHA512(pw))` pre-hash survives only on the legacy `Cryptor` path.

use argon2::{Algorithm, Argon2, Params as Argon2Params, Version};
use base64::{engine::general_purpose::STANDARD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;
use zeroize::Zeroizing;

use crate::error::{Error, Result};

/// Derived key length in bytes. 32 bytes is a valid HKDF input keying material
/// length; the store's HKDF step expands it into the SQLCipher + payload subkeys.
pub const KEY_LEN: usize = 32;

/// Salt length in bytes for freshly generated params. 16 bytes is the Argon2
/// minimum/recommended salt size; we use 32 for headroom.
pub const SALT_LEN: usize = 32;

// Default Argon2id cost parameters (OWASP-reasonable, 2024). Tunable: raise as
// hardware improves. `m_cost` is in KiB, so 65536 KiB = 64 MiB.
pub const DEFAULT_M_COST: u32 = 65_536; // 64 MiB
pub const DEFAULT_T_COST: u32 = 3; // iterations (time cost)
pub const DEFAULT_P_COST: u32 = 4; // parallelism (lanes)

// The most a descriptor may ask for. A descriptor is read off things this
// build did not write — a `.rowel` backup, a pack pulled from Drive, the
// sidecar on disk — and the `argon2` crate accepts costs up to `u32::MAX`,
// so without a ceiling a crafted header is a 4 TiB allocation (an abort) or a
// derivation that never returns. The ceilings are an order of magnitude past
// the defaults: room to raise the defaults as hardware improves, never room
// for a hostile file to hang the app. `MAX_M_COST` is in KiB, like `m_cost`.
pub const MAX_M_COST: u32 = 1_048_576; // 1 GiB
pub const MAX_T_COST: u32 = 64;
pub const MAX_P_COST: u32 = 64;
pub const MAX_PBKDF2_ITERATIONS: u32 = 10_000_000;

fn err<E: std::fmt::Display>(e: E) -> Error {
    Error::Crypto(e.to_string())
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// A self-describing KDF descriptor. Serializes to a compact JSON object tagged
/// by `algo`, e.g.
/// `{"algo":"argon2id","m_cost":65536,"t_cost":3,"p_cost":4,"salt":"<base64>"}`
/// or `{"algo":"pbkdf2-sha512","iterations":100000,"salt":"<base64>"}`.
///
/// The `salt` is stored as standard base64. Store the whole thing as a string
/// in the vault's metadata; [`derive`] reproduces the key from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "algo")]
pub enum KdfParams {
    /// Memory-hard Argon2id. Costs: `m_cost` in KiB, `t_cost` passes,
    /// `p_cost` lanes.
    #[serde(rename = "argon2id")]
    Argon2id {
        m_cost: u32,
        t_cost: u32,
        p_cost: u32,
        /// base64(salt).
        salt: String,
        /// Key generation of the lockbox this descriptor belongs to: bumped
        /// by every re-key, so a device can tell "the password changed
        /// elsewhere" (higher) from "a copy under a password this device
        /// already replaced" (lower). Written only when non-zero, so every
        /// descriptor written before the field existed keeps its exact bytes.
        #[serde(default, skip_serializing_if = "is_zero")]
        gen: u32,
    },
    /// PBKDF2-HMAC-SHA512 fallback / legacy shape.
    #[serde(rename = "pbkdf2-sha512")]
    Pbkdf2Sha512 {
        iterations: u32,
        /// base64(salt).
        salt: String,
    },
    /// A master no password derives: an org lockbox's 32 random bytes, minted
    /// by a Rowel Server and handed to each enrolled device wrapped to its
    /// enrollment key. `kid` names the key at the server, so a rotation is a
    /// new `kid` as well as the next `gen`; the generation means what
    /// Argon2id's does, and is always written, since no descriptor of this
    /// shape predates the field.
    #[serde(rename = "external")]
    External { kid: String, gen: u32 },
}

impl KdfParams {
    /// Default Argon2id params (`m=64MiB, t=3, p=4`) with a **fresh random
    /// salt** — the descriptor to persist for a brand-new vault.
    pub fn default_argon2id() -> Self {
        Self::argon2id(
            &random_salt(),
            DEFAULT_M_COST,
            DEFAULT_T_COST,
            DEFAULT_P_COST,
        )
    }

    /// Build Argon2id params from an explicit salt and costs (used by tests and
    /// callers that must reproduce a specific derivation).
    pub fn argon2id(salt: &[u8], m_cost: u32, t_cost: u32, p_cost: u32) -> Self {
        Self::Argon2id {
            m_cost,
            t_cost,
            p_cost,
            salt: STANDARD.encode(salt),
            gen: 0,
        }
    }

    /// The descriptor of a master a Rowel Server holds under `kid`, at key
    /// generation `gen` (see [`KdfParams::External`]).
    pub fn external(kid: impl Into<String>, gen: u32) -> Self {
        Self::External {
            kid: kid.into(),
            gen,
        }
    }

    /// Whether the master is held by a server rather than derived here.
    pub fn is_external(&self) -> bool {
        matches!(self, Self::External { .. })
    }

    /// The key generation this descriptor carries (0 when it has none).
    pub fn gen(&self) -> u32 {
        match self {
            Self::Argon2id { gen, .. } | Self::External { gen, .. } => *gen,
            Self::Pbkdf2Sha512 { .. } => 0,
        }
    }

    /// The descriptor for the next re-key of the same lockbox, at the
    /// generation one past this one: default Argon2id costs and a fresh salt
    /// for a password-derived key, a fresh `kid` for a server-held one, since
    /// a rotation there is a new key under a new name.
    pub fn next_generation(&self) -> Self {
        let gen = self.gen().saturating_add(1);
        if self.is_external() {
            return Self::external(super::random_hex_id(), gen);
        }
        match Self::default_argon2id() {
            Self::Argon2id {
                m_cost,
                t_cost,
                p_cost,
                salt,
                ..
            } => Self::Argon2id {
                m_cost,
                t_cost,
                p_cost,
                salt,
                gen,
            },
            other => other,
        }
    }

    /// Build PBKDF2-HMAC-SHA512 params from an explicit salt and iteration count.
    pub fn pbkdf2_sha512(salt: &[u8], iterations: u32) -> Self {
        Self::Pbkdf2Sha512 {
            iterations,
            salt: STANDARD.encode(salt),
        }
    }

    /// The `algo` tag as stored (`"argon2id"` / `"pbkdf2-sha512"` /
    /// `"external"`).
    pub fn algo(&self) -> &'static str {
        match self {
            Self::Argon2id { .. } => "argon2id",
            Self::Pbkdf2Sha512 { .. } => "pbkdf2-sha512",
            Self::External { .. } => "external",
        }
    }

    /// Serialize to a compact JSON string suitable for the `meta` table.
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    /// Parse from the JSON string produced by [`to_json`](Self::to_json).
    ///
    /// Bounds-checked on the way in (see [`KdfParams::validate`]): every
    /// descriptor this parses came off disk or off the network, and refusing it
    /// here keeps an absurd one out of every path that would derive from it.
    pub fn from_json(s: &str) -> Result<Self> {
        let params: Self = serde_json::from_str(s)?;
        params.validate()?;
        Ok(params)
    }

    /// Refuse costs outside what this build is prepared to pay. Zero and
    /// below-minimum values are left to the primitives, which reject them
    /// with their own message; this is the ceiling they do not have. An
    /// external descriptor costs nothing to hold, so there is nothing to
    /// bound.
    pub fn validate(&self) -> Result<()> {
        let too_costly = match self {
            Self::Argon2id {
                m_cost,
                t_cost,
                p_cost,
                ..
            } => *m_cost > MAX_M_COST || *t_cost > MAX_T_COST || *p_cost > MAX_P_COST,
            Self::Pbkdf2Sha512 { iterations, .. } => *iterations > MAX_PBKDF2_ITERATIONS,
            Self::External { .. } => false,
        };
        if too_costly {
            return Err(Error::Crypto(
                "key derivation parameters exceed the supported bounds".into(),
            ));
        }
        Ok(())
    }

    fn decoded_salt(salt: &str) -> Result<Vec<u8>> {
        STANDARD.decode(salt).map_err(err)
    }
}

/// Generate `SALT_LEN` cryptographically-random bytes.
pub fn random_salt() -> Vec<u8> {
    let mut salt = vec![0u8; SALT_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    salt
}

/// Derive `KEY_LEN` bytes of key material from `password` per `params`.
///
/// The output is zeroized on drop. `password` is treated as opaque input bytes:
/// callers feed the master password directly for Argon2id.
///
/// Refuses an [`KdfParams::External`] descriptor: no password derives that
/// master, and every path that asks for one (an unlock, a restore, a join)
/// has to go to the server that holds it instead.
pub fn derive(password: &[u8], params: &KdfParams) -> Result<Zeroizing<Vec<u8>>> {
    // Checked here as well as at parse time: a descriptor can be built in
    // code, and this is the one place the cost is actually paid.
    params.validate()?;
    let mut out = Zeroizing::new(vec![0u8; KEY_LEN]);
    match params {
        KdfParams::External { .. } => {
            return Err(Error::Crypto(
                "this lockbox's key is held by a Rowel Server, not derived from a password".into(),
            ));
        }
        KdfParams::Argon2id {
            m_cost,
            t_cost,
            p_cost,
            salt,
            ..
        } => {
            let salt = KdfParams::decoded_salt(salt)?;
            let params =
                Argon2Params::new(*m_cost, *t_cost, *p_cost, Some(KEY_LEN)).map_err(err)?;
            let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
            argon
                .hash_password_into(password, &salt, &mut out)
                .map_err(err)?;
        }
        KdfParams::Pbkdf2Sha512 { iterations, salt } => {
            let salt = KdfParams::decoded_salt(salt)?;
            let iters = NonZeroU32::new(*iterations)
                .ok_or_else(|| Error::Crypto("pbkdf2 iterations must be > 0".into()))?;
            // Matches the legacy Cryptor's PBKDF2-HMAC-SHA512, so a fallback
            // descriptor reproduces the same key path (minus the pre-hash,
            // which is the caller's choice of `password` bytes).
            ring::pbkdf2::derive(
                ring::pbkdf2::PBKDF2_HMAC_SHA512,
                iters,
                &salt,
                password,
                &mut out,
            );
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SALT_A: &[u8] = b"0123456789abcdef0123456789abcdef";
    const SALT_B: &[u8] = b"fedcba9876543210fedcba9876543210";

    fn argon(salt: &[u8]) -> KdfParams {
        // Low cost so the test suite stays fast; production uses the defaults.
        KdfParams::argon2id(salt, 256, 1, 1)
    }

    #[test]
    fn argon2id_is_deterministic() {
        let p = argon(SALT_A);
        let a = derive(b"correct horse", &p).unwrap();
        let b = derive(b"correct horse", &p).unwrap();
        assert_eq!(&*a, &*b);
        assert_eq!(a.len(), KEY_LEN);
    }

    #[test]
    fn argon2id_salt_changes_key() {
        let a = derive(b"pw", &argon(SALT_A)).unwrap();
        let b = derive(b"pw", &argon(SALT_B)).unwrap();
        assert_ne!(&*a, &*b);
    }

    #[test]
    fn argon2id_password_changes_key() {
        let p = argon(SALT_A);
        let a = derive(b"pw-one", &p).unwrap();
        let b = derive(b"pw-two", &p).unwrap();
        assert_ne!(&*a, &*b);
    }

    #[test]
    fn argon2id_cost_change_changes_key() {
        let a = derive(b"pw", &KdfParams::argon2id(SALT_A, 256, 1, 1)).unwrap();
        let b = derive(b"pw", &KdfParams::argon2id(SALT_A, 256, 2, 1)).unwrap();
        assert_ne!(&*a, &*b, "changing t_cost must change the derived key");
    }

    #[test]
    fn pbkdf2_is_deterministic() {
        let p = KdfParams::pbkdf2_sha512(SALT_A, 1000);
        let a = derive(b"pw", &p).unwrap();
        let b = derive(b"pw", &p).unwrap();
        assert_eq!(&*a, &*b);
        assert_eq!(a.len(), KEY_LEN);
    }

    #[test]
    fn pbkdf2_iterations_change_key() {
        let a = derive(b"pw", &KdfParams::pbkdf2_sha512(SALT_A, 1000)).unwrap();
        let b = derive(b"pw", &KdfParams::pbkdf2_sha512(SALT_A, 2000)).unwrap();
        assert_ne!(&*a, &*b);
    }

    #[test]
    fn dispatch_distinguishes_algos() {
        // Same password + same salt, different algorithm → different key.
        let argon = derive(b"pw", &KdfParams::argon2id(SALT_A, 256, 1, 1)).unwrap();
        let pbkdf2 = derive(b"pw", &KdfParams::pbkdf2_sha512(SALT_A, 1000)).unwrap();
        assert_ne!(&*argon, &*pbkdf2);
    }

    #[test]
    fn serde_round_trip_argon2id() {
        let p = KdfParams::argon2id(SALT_A, DEFAULT_M_COST, DEFAULT_T_COST, DEFAULT_P_COST);
        let json = p.to_json().unwrap();
        assert!(json.contains("\"algo\":\"argon2id\""));
        let back = KdfParams::from_json(&json).unwrap();
        assert_eq!(p, back);
        // The reconstructed descriptor derives the same key.
        assert_eq!(
            &*derive(b"pw", &p).unwrap(),
            &*derive(b"pw", &back).unwrap()
        );
    }

    #[test]
    fn serde_round_trip_pbkdf2() {
        let p = KdfParams::pbkdf2_sha512(SALT_A, 100_000);
        let json = p.to_json().unwrap();
        assert!(json.contains("\"algo\":\"pbkdf2-sha512\""));
        let back = KdfParams::from_json(&json).unwrap();
        assert_eq!(p, back);
    }

    // Every descriptor written before `gen` existed must keep its exact bytes
    // through a parse and re-serialize, or every synced device would read its
    // own sidecar as "a different install".
    #[test]
    fn gen_round_trips_byte_for_byte_and_is_absent_when_zero() {
        let legacy = format!(
            r#"{{"algo":"argon2id","m_cost":65536,"t_cost":3,"p_cost":4,"salt":"{}"}}"#,
            STANDARD.encode(SALT_A)
        );
        let parsed = KdfParams::from_json(&legacy).unwrap();
        assert_eq!(parsed.gen(), 0);
        assert_eq!(parsed.to_json().unwrap(), legacy);
        assert!(!KdfParams::default_argon2id()
            .to_json()
            .unwrap()
            .contains("gen"));

        let bumped = parsed.next_generation();
        let json = bumped.to_json().unwrap();
        assert!(json.ends_with(r#","gen":1}"#), "{json}");
        let back = KdfParams::from_json(&json).unwrap();
        assert_eq!(back, bumped);
        assert_eq!(back.to_json().unwrap(), json);

        let pbkdf2 = KdfParams::pbkdf2_sha512(SALT_A, 1000).to_json().unwrap();
        assert_eq!(
            KdfParams::from_json(&pbkdf2).unwrap().to_json().unwrap(),
            pbkdf2
        );
    }

    #[test]
    fn next_generation_bumps_and_resalts() {
        let first = KdfParams::argon2id(SALT_A, 256, 1, 1);
        let second = first.next_generation();
        let third = second.next_generation();
        assert_eq!((second.gen(), third.gen()), (1, 2));
        match (&first, &second) {
            (KdfParams::Argon2id { salt: a, .. }, KdfParams::Argon2id { salt: b, .. }) => {
                assert_ne!(a, b)
            }
            _ => panic!("next_generation must be Argon2id"),
        }
        assert!(second.validate().is_ok());
        assert_eq!(
            KdfParams::pbkdf2_sha512(SALT_A, 1).next_generation().gen(),
            1
        );
    }

    // The server's descriptor is stored verbatim on every device and compared
    // as a string by the sync engine, so a parse and re-serialize has to give
    // the exact bytes back — generation zero included, which Argon2id omits.
    #[test]
    fn external_round_trips_byte_for_byte_with_its_generation() {
        for (json, gen) in [
            (r#"{"algo":"external","kid":"0123abcd","gen":0}"#, 0),
            (r#"{"algo":"external","kid":"0123abcd","gen":4}"#, 4),
        ] {
            let parsed = KdfParams::from_json(json).unwrap();
            assert_eq!(parsed, KdfParams::external("0123abcd", gen));
            assert!(parsed.is_external());
            assert_eq!(parsed.algo(), "external");
            assert_eq!(parsed.gen(), gen);
            assert_eq!(parsed.to_json().unwrap(), json);
        }
        assert!(!KdfParams::argon2id(SALT_A, 256, 1, 1).is_external());
    }

    // No password derives a server-held master: asking is an error that says
    // where the key is, never a key.
    #[test]
    fn external_refuses_to_derive() {
        match derive(b"pw", &KdfParams::external("kid", 1)) {
            Err(Error::Crypto(why)) => assert!(why.contains("Rowel Server"), "{why}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    // A rotation at the server is a new key under a new name at the next
    // generation, as a password change is a new salt at the next one.
    #[test]
    fn external_next_generation_bumps_gen_and_changes_kid() {
        let first = KdfParams::external("k1", 2);
        let second = first.next_generation();
        match &second {
            KdfParams::External { kid, gen } => {
                assert_ne!(kid, "k1");
                assert_eq!(kid.len(), 32, "a fresh 16-byte hex id");
                assert_eq!(*gen, 3);
            }
            other => panic!("next_generation must stay external, got {other:?}"),
        }
        assert!(second.validate().is_ok());
        assert_ne!(second.next_generation(), second);
    }

    // The generation is not an input to the derivation: a descriptor that
    // differs only by `gen` derives the same key (older builds ignore it).
    #[test]
    fn gen_does_not_change_the_derived_key() {
        let base = KdfParams::argon2id(SALT_A, 256, 1, 1);
        let mut later = base.clone();
        if let KdfParams::Argon2id { gen, .. } = &mut later {
            *gen = 7;
        }
        assert_eq!(
            &*derive(b"pw", &base).unwrap(),
            &*derive(b"pw", &later).unwrap()
        );
    }

    #[test]
    fn default_argon2id_uses_owasp_params_and_fresh_salt() {
        let a = KdfParams::default_argon2id();
        let b = KdfParams::default_argon2id();
        match (&a, &b) {
            (
                KdfParams::Argon2id {
                    m_cost,
                    t_cost,
                    p_cost,
                    salt: salt_a,
                    gen,
                },
                KdfParams::Argon2id { salt: salt_b, .. },
            ) => {
                assert_eq!((*m_cost, *t_cost, *p_cost), (65_536, 3, 4));
                assert_eq!(*gen, 0, "a fresh vault starts at generation 0");
                assert_ne!(salt_a, salt_b, "each call must generate a fresh salt");
            }
            _ => panic!("default_argon2id must be Argon2id"),
        }
    }

    // A descriptor is untrusted input: a header asking for terabytes of memory
    // or a derivation that runs for hours must be refused, not attempted.
    #[test]
    fn costs_past_the_ceiling_are_refused() {
        let huge_memory = KdfParams::argon2id(SALT_A, u32::MAX, 1, 1);
        assert!(derive(b"pw", &huge_memory).is_err());
        let endless = KdfParams::argon2id(SALT_A, 256, u32::MAX, 1);
        assert!(derive(b"pw", &endless).is_err());
        let too_wide = KdfParams::argon2id(SALT_A, 256, 1, MAX_P_COST + 1);
        assert!(derive(b"pw", &too_wide).is_err());
        let forever = KdfParams::pbkdf2_sha512(SALT_A, u32::MAX);
        assert!(derive(b"pw", &forever).is_err());

        // Refused at parse time too, so a hostile file never reaches `derive`.
        let json = format!(
            r#"{{"algo":"argon2id","m_cost":{},"t_cost":3,"p_cost":4,"salt":"{}"}}"#,
            u32::MAX,
            STANDARD.encode(SALT_A)
        );
        assert!(KdfParams::from_json(&json).is_err());
    }

    // The ceilings sit well above the defaults, so no vault this app writes
    // can ever fail its own check.
    #[test]
    fn the_defaults_are_within_the_ceiling() {
        assert!(KdfParams::default_argon2id().validate().is_ok());
        assert!(
            KdfParams::argon2id(SALT_A, MAX_M_COST, MAX_T_COST, MAX_P_COST)
                .validate()
                .is_ok()
        );
        assert!(KdfParams::pbkdf2_sha512(SALT_A, MAX_PBKDF2_ITERATIONS)
            .validate()
            .is_ok());
    }

    #[test]
    fn random_salt_has_expected_len_and_varies() {
        let a = random_salt();
        let b = random_salt();
        assert_eq!(a.len(), SALT_LEN);
        assert_ne!(a, b);
    }

    #[test]
    fn algo_tag() {
        assert_eq!(KdfParams::default_argon2id().algo(), "argon2id");
        assert_eq!(KdfParams::pbkdf2_sha512(SALT_A, 1).algo(), "pbkdf2-sha512");
    }

    // Known-answer vector for Argon2id (RustCrypto `argon2` v0.5, Version::V0x13).
    // Pins output length 32 with the low-cost params above so a crate/param/
    // version regression is caught. Salt = SALT_A ("0123...").
    #[test]
    fn argon2id_known_answer() {
        let key = derive(b"password", &KdfParams::argon2id(SALT_A, 256, 1, 1)).unwrap();
        assert_eq!(hex::encode(&*key), KNOWN_ARGON2ID_HEX);
    }

    const KNOWN_ARGON2ID_HEX: &str =
        "5e08debc30c12c81d0d281980d6e5b29afead8f827714dd1d239f45996d415cb";
}
