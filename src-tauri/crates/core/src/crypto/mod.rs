//! Crypto core. Byte-compatible with the legacy `@swiftyapp/cryptor` vault format:
//! `hex( salt[64] ‖ iv[16] ‖ tag[16] ‖ ciphertext )` per value, wrapped in base64
//! for whole-object blobs. See `legacy/src/main/application/cryptor/index.js`.

use aes_gcm::aead::consts::U16;
use aes_gcm::aead::{Aead, AeadInOut, KeyInit, Payload};
use aes_gcm::aes::Aes256;
use aes_gcm::{AesGcm, Nonce, Tag};
use base64::{engine::general_purpose::STANDARD, Engine};
use rand::RngCore;
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha512};
use std::num::NonZeroU32;
use zeroize::Zeroizing;

use crate::error::{Error, Result};
use crate::models::Entry;

pub mod kdf;
pub use kdf::{derive, KdfParams};

mod vault;
pub use vault::{PayloadCipher, VaultKey};

#[cfg(test)]
mod tests;

// The vault uses a 16-byte GCM nonce, not the 12-byte Rust default. A 12-byte
// assumption silently fails to decrypt every existing vault.
type Aes256Gcm16 = AesGcm<Aes256, U16>;

const SALT_LEN: usize = 64;
const IV_LEN: usize = 16;
const TAG_LEN: usize = 16;
const ITERATIONS: u32 = 100_000;
pub const KEY_LEN: usize = 32;

/// Sixteen random bytes — the width of every id the app mints for itself
/// (entries, workspaces, share files, export rows). One source so they all
/// carry the same 128 bits of entropy; callers pick the encoding.
pub fn random_id_bytes() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}

/// [`random_id_bytes`] as lowercase hex — the shape of the frontend's own ids.
pub fn random_hex_id() -> String {
    hex::encode(random_id_bytes())
}

/// Borrow `bytes` as a 16-byte GCM nonce. aes-gcm 0.11 models nonces as
/// `hybrid_array::Array` and deprecates its `from_slice`, so the length check is
/// ours to make: the decrypt paths take the nonce off the wire as a slice.
fn to_nonce(bytes: &[u8]) -> Result<Nonce<U16>> {
    Nonce::<U16>::try_from(bytes).map_err(|_| Error::Crypto("invalid nonce length".into()))
}

/// `base64( SHA512(pw) )` — the "secret" fed to the cryptor.
pub fn hash_secret(password: &str) -> String {
    STANDARD.encode(Sha512::digest(password.as_bytes()))
}

/// Legacy deterministic SQLCipher key: HKDF of the `hash_secret` string, with no
/// per-vault salt. Retained only for sidecar-less interim/dev DBs so they still
/// open; Argon2id vaults derive their SQLCipher key via [`VaultKey`] instead.
pub fn sqlcipher_key(secret: &str) -> [u8; KEY_LEN] {
    const SALT: &[u8] = b"rowel-sqlcipher-v1";
    const INFO: &[u8] = b"sqlcipher-db-key";
    let prk = ring::hkdf::Salt::new(ring::hkdf::HKDF_SHA256, SALT).extract(secret.as_bytes());
    let okm = prk
        .expand(&[INFO], ring::hkdf::HKDF_SHA256)
        .expect("hkdf expand");
    let mut key = [0u8; KEY_LEN];
    okm.fill(&mut key).expect("hkdf fill");
    key
}

/// Derive a 32-byte subkey from `ikm` (an Argon2id master key) for the given
/// `info` label via HKDF-SHA256. One master yields the SQLCipher key and the
/// payload key as independent subkeys (distinct `info`), so a single KDF pass
/// covers both without either revealing the other.
pub fn hkdf_subkey(ikm: &[u8], info: &[u8]) -> [u8; KEY_LEN] {
    const SALT: &[u8] = b"rowel-kdf-hkdf-v1";
    let prk = ring::hkdf::Salt::new(ring::hkdf::HKDF_SHA256, SALT).extract(ikm);
    let info = [info];
    let okm = prk
        .expand(&info, ring::hkdf::HKDF_SHA256)
        .expect("hkdf expand");
    let mut key = [0u8; KEY_LEN];
    okm.fill(&mut key).expect("hkdf fill");
    key
}

/// Seal `plaintext` with AES-256-GCM under `key` and a fresh random 16-byte
/// nonce, returning `nonce ‖ ciphertext ‖ tag`. This is the per-payload AEAD for
/// Argon2id-keyed vaults — no per-payload KDF; the caller derives `key` once.
///
/// `aad` is authenticated but not stored: the caller has to present the same
/// bytes to [`unseal_aead`], which is what binds a ciphertext to its context
/// (a vault payload to its row id). Empty when there is nothing to bind to.
pub fn seal_aead(key: &[u8], aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm16::new_from_slice(key).map_err(err)?;
    let mut nonce = [0u8; IV_LEN];
    rand::thread_rng().fill_bytes(&mut nonce);
    let sealed = cipher
        .encrypt(
            &to_nonce(&nonce)?,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(err)?;
    let mut out = Vec::with_capacity(IV_LEN + sealed.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// Reverse of [`seal_aead`]: split off the 16-byte nonce and decrypt the rest,
/// under the same `aad` it was sealed with.
pub fn unseal_aead(key: &[u8], aad: &[u8], blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() < IV_LEN + TAG_LEN {
        return Err(Error::Crypto("payload too short".into()));
    }
    let (nonce, sealed) = blob.split_at(IV_LEN);
    let cipher = Aes256Gcm16::new_from_slice(key).map_err(err)?;
    cipher
        .decrypt(&to_nonce(nonce)?, Payload { msg: sealed, aad })
        .map_err(err)
}

// A subkey of the app key, so the bytes that seal the workspace keys are never
// the bytes that open the primary's database or its payloads.
const INFO_WORKSPACE_WRAP: &[u8] = b"workspace-key-wrap";

/// Seal workspace `id`'s key material under the app key — the primary
/// workspace's — as the text its `vault.key.sealed` sidecar holds
/// ([`crate::layout::WRAPPED_KEY_FILE`]). Bound to the workspace id, so a
/// sidecar copied under another workspace's directory does not open there.
pub fn seal_workspace_key(app_key: &[u8], id: &str, material: &[u8]) -> Result<String> {
    let key = Zeroizing::new(hkdf_subkey(app_key, INFO_WORKSPACE_WRAP));
    Ok(STANDARD.encode(seal_aead(&*key, id.as_bytes(), material)?))
}

/// Reverse of [`seal_workspace_key`]. Fails on a sidecar that will not decode,
/// and on one sealed under an app key other than `app_key` — which is what a
/// sidecar left behind by a deleted primary is.
pub fn open_workspace_key(app_key: &[u8], id: &str, sealed: &str) -> Result<Zeroizing<Vec<u8>>> {
    let key = Zeroizing::new(hkdf_subkey(app_key, INFO_WORKSPACE_WRAP));
    let blob = STANDARD.decode(sealed.trim()).map_err(err)?;
    unseal_aead(&*key, id.as_bytes(), &blob).map(Zeroizing::new)
}

fn err<E: std::fmt::Display>(e: E) -> Error {
    Error::Crypto(e.to_string())
}

pub struct Cryptor {
    // Zeroized on drop so the master secret doesn't linger on the heap.
    secret: Zeroizing<Vec<u8>>,
}

impl Cryptor {
    pub fn new(secret: &str) -> Self {
        Self {
            secret: Zeroizing::new(secret.as_bytes().to_vec()),
        }
    }

    fn cipher(&self, salt: &[u8]) -> Result<Aes256Gcm16> {
        // The derived AES key is scrubbed on drop too.
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        // ring's PBKDF2 is assembly-optimized; the pure-Rust one is ~10-20x slower
        // and froze the UI while deriving a key per field. Output is identical.
        let iters = NonZeroU32::new(ITERATIONS).unwrap();
        ring::pbkdf2::derive(
            ring::pbkdf2::PBKDF2_HMAC_SHA512,
            iters,
            salt,
            &self.secret,
            &mut *key,
        );
        Aes256Gcm16::new_from_slice(&*key).map_err(err)
    }

    /// Encrypt to `hex( salt ‖ iv ‖ tag ‖ ciphertext )`.
    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        let mut rng = rand::thread_rng();
        let mut salt = [0u8; SALT_LEN];
        let mut iv = [0u8; IV_LEN];
        rng.fill_bytes(&mut salt);
        rng.fill_bytes(&mut iv);

        let cipher = self.cipher(&salt)?;
        let payload = Payload {
            msg: plaintext.as_bytes(),
            aad: &[],
        };
        // aes-gcm returns `ciphertext ‖ tag`; the vault format wants the tag before it.
        let sealed = cipher.encrypt(&to_nonce(&iv)?, payload).map_err(err)?;
        let (ciphertext, tag) = sealed.split_at(sealed.len() - TAG_LEN);

        let mut out = Vec::with_capacity(SALT_LEN + IV_LEN + TAG_LEN + ciphertext.len());
        out.extend_from_slice(&salt);
        out.extend_from_slice(&iv);
        out.extend_from_slice(tag);
        out.extend_from_slice(ciphertext);
        Ok(hex::encode(out))
    }

    /// Reverse of [`encrypt`](Self::encrypt).
    pub fn decrypt(&self, hexstr: &str) -> Result<String> {
        self.decrypt_bytes(hex::decode(hexstr).map_err(err)?)
    }

    /// The AES half of [`decrypt`](Self::decrypt), taking `salt ‖ iv ‖ tag ‖
    /// ciphertext` by value and unsealing the ciphertext where it already lies.
    /// The plaintext is that same buffer with the header drained off its front,
    /// so nothing the size of the message is ever held twice — which is what
    /// keeps a whole-vault blob from costing a second copy of itself.
    fn decrypt_bytes(&self, mut bytes: Vec<u8>) -> Result<String> {
        const HEADER: usize = SALT_LEN + IV_LEN + TAG_LEN;
        if bytes.len() < HEADER {
            return Err(Error::Crypto("ciphertext too short".into()));
        }
        let cipher = self.cipher(&bytes[..SALT_LEN])?;
        let nonce = to_nonce(&bytes[SALT_LEN..SALT_LEN + IV_LEN])?;
        let tag = Tag::<U16>::try_from(&bytes[SALT_LEN + IV_LEN..HEADER])
            .map_err(|_| Error::Crypto("invalid tag length".into()))?;

        cipher
            .decrypt_inout_detached(&nonce, &[], (&mut bytes[HEADER..]).into(), &tag)
            .map_err(err)?;
        bytes.drain(..HEADER);
        String::from_utf8(bytes).map_err(err)
    }

    /// `base64( encrypt( JSON.stringify(data) ) )` — the on-disk blob (double-encoded).
    pub fn encrypt_data<T: Serialize>(&self, data: &T) -> Result<String> {
        let json = serde_json::to_string(data)?;
        Ok(STANDARD.encode(self.encrypt(&json)?))
    }

    /// Reverse of [`encrypt_data`](Self::encrypt_data).
    pub fn decrypt_data<T: DeserializeOwned>(&self, blob: &str) -> Result<T> {
        self.decrypt_hex_bytes(STANDARD.decode(blob).map_err(err)?)
    }

    /// [`decrypt_data`](Self::decrypt_data) for a blob the caller is done with:
    /// it takes the string by value and frees it the moment the base64 layer is
    /// off, so an import of a file the size of a whole vault holds the file and
    /// its decoding together only for that one step. Each layer after it is
    /// smaller than the one it came from and replaces it.
    pub fn decrypt_data_owned<T: DeserializeOwned>(&self, blob: String) -> Result<T> {
        let hexstr = STANDARD.decode(blob.as_bytes()).map_err(err)?;
        drop(blob);
        self.decrypt_hex_bytes(hexstr)
    }

    /// The shared tail of both: hex text in, parsed value out, dropping each
    /// encoding as the next one is built from it.
    fn decrypt_hex_bytes<T: DeserializeOwned>(&self, hexstr: Vec<u8>) -> Result<T> {
        let bytes = hex::decode(&hexstr).map_err(err)?;
        drop(hexstr);
        let json = self.decrypt_bytes(bytes)?;
        Ok(serde_json::from_str(&json)?)
    }

    /// Encrypt an entry's sensitive fields in place, each stored as raw hex.
    pub fn obscure(&self, entry: &Entry) -> Result<Entry> {
        self.transform(entry, |v| self.encrypt(v))
    }

    /// Decrypt an entry's sensitive fields in place.
    pub fn expose(&self, entry: &Entry) -> Result<Entry> {
        self.transform(entry, |v| self.decrypt(v))
    }

    /// Obscure `new`, re-encrypting only fields that differ from `old`'s stored
    /// ciphertext. Unchanged fields keep their ciphertext (no extra derivation),
    /// so saving one edited entry never re-encrypts the whole vault.
    pub fn obscure_changed(&self, new: &Entry, old: Option<&Entry>) -> Result<Entry> {
        // Old and new both go through `secret_slots`, so the two sides line up
        // by construction rather than by two tables kept in step by hand. The
        // clone of `old` exists only because the table hands out `&mut`.
        let mut old = old.cloned();
        let old_vals: Vec<Option<String>> = old
            .as_mut()
            .map(|o| secret_slots(o).into_iter().map(Option::take).collect())
            .unwrap_or_default();
        let mut e = new.clone();
        for (i, slot) in secret_slots(&mut e).into_iter().enumerate() {
            let value = slot.take().unwrap_or_default();
            *slot = Some(if value.is_empty() {
                String::new()
            } else if old_vals.get(i).and_then(|o| o.as_deref()) == Some(value.as_str()) {
                value
            } else {
                self.encrypt(&value)?
            });
        }
        Ok(e)
    }

    fn transform(&self, entry: &Entry, f: impl Fn(&str) -> Result<String>) -> Result<Entry> {
        let mut e = entry.clone();
        for slot in secret_slots(&mut e) {
            let value = slot.take().unwrap_or_default();
            // Empty/absent fields stay "".
            *slot = Some(if value.is_empty() {
                String::new()
            } else {
                f(&value)?
            });
        }
        Ok(e)
    }
}

// The per-kind secret fields, the one place that says which they are. Every
// caller walks the same list in the same order, so "changed since last save"
// can be decided slot by slot without a second table to keep in step.
fn secret_slots(e: &mut Entry) -> Vec<&mut Option<String>> {
    match e.kind.as_str() {
        "login" => vec![&mut e.password, &mut e.otp],
        "note" => vec![&mut e.note],
        "card" => vec![&mut e.pin],
        "identity" => vec![&mut e.number, &mut e.personal_number],
        "ssh" => vec![&mut e.private_key, &mut e.passphrase],
        // The whole file is the secret; its name is not.
        "env" => vec![&mut e.body],
        // The token alone: where it is used, for what, and until when are not.
        "apikey" => vec![&mut e.api_key],
        _ => vec![],
    }
}
