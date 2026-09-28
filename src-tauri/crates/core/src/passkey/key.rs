//! Key and record translation between the vault's [`models::Passkey`] (base64url
//! strings, PKCS#8 private key) and the shapes `passkey-rs` speaks (COSE keys,
//! raw bytes). P-256 / ES256 only — the same single algorithm the data model and
//! the Bitwarden importer assume.
//!
//! Base64url is decoded with `passkey_types::encoding`, which accepts padded and
//! unpadded input alike: credential ids and key material are carried verbatim
//! from whatever exporter produced them (Bitwarden writes unpadded), so a
//! stricter engine would reject perfectly good imports.

use coset::iana::EnumI64;
use coset::{iana, CoseKey, CoseKeyBuilder, Label, RegisteredLabel, RegisteredLabelWithPrivate};
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use p256::{ecdsa::SigningKey, SecretKey};
use passkey_types::ctap2::make_credential::PublicKeyCredentialUserEntity;
use passkey_types::encoding;

use crate::error::{Error, Result};
use crate::models;

/// Decode a base64url PKCS#8 DER private key into the COSE key `passkey-rs`
/// stores on a credential.
pub fn pkcs8_to_cose(private_key_b64url: &str) -> Result<CoseKey> {
    let der = decode(private_key_b64url, "private key")?;
    let secret = SecretKey::from_pkcs8_der(&der)
        .map_err(|e| Error::Crypto(format!("passkey private key is not P-256 PKCS#8: {e}")))?;
    Ok(cose_from_secret(&secret))
}

/// Encode a COSE private key back to the base64url PKCS#8 DER the vault stores.
/// Rejects anything that is not an ES256 EC2 P-256 key with its `d` parameter.
pub fn cose_to_pkcs8(key: &CoseKey) -> Result<String> {
    let der = secret_from_cose(key)?
        .to_pkcs8_der()
        .map_err(|e| Error::Crypto(format!("cannot encode passkey private key: {e}")))?;
    Ok(encoding::base64url(der.as_bytes()))
}

/// The vault record as the authenticator wants it. The counter is the reason
/// this cannot be a plain `From`: `0` means "no counter" to `passkey-rs`, which
/// then reports the constant zero the WebAuthn spec recommends for synced
/// credentials, and never asks the store for an update.
pub fn to_passkey_types(passkey: &models::Passkey) -> Result<passkey_types::Passkey> {
    Ok(passkey_types::Passkey {
        key: pkcs8_to_cose(&passkey.private_key)?,
        credential_id: decode_credential_id(&passkey.credential_id)?.into(),
        rp_id: passkey.rp_id.clone(),
        // Ours are always discoverable: the vault is the credential list, so a
        // sign-in with no allowCredentials must still find them.
        user_handle: Some(decode(&passkey.user_handle, "user handle")?.into()),
        // Carried so the authenticator can name the account in a UI hint; the
        // relying party never sees either (see `get_assertion`'s response).
        username: Some(passkey.user_name.clone()),
        user_display_name: Some(passkey.user_display_name.clone()),
        counter: (passkey.counter != 0).then_some(passkey.counter),
        extensions: Default::default(),
    })
}

/// The inverse, for a credential the authenticator just created. The user's name
/// and display name live on the request's user entity rather than on the
/// credential, and `rp_name` on the request's rp entity, so they come in
/// alongside; `created_at` is stamped here because this is where the record is
/// born.
pub fn from_passkey_types(
    passkey: &passkey_types::Passkey,
    user: &PublicKeyCredentialUserEntity,
    rp_name: Option<&str>,
) -> Result<models::Passkey> {
    Ok(models::Passkey {
        credential_id: encoding::base64url(&passkey.credential_id),
        rp_id: passkey.rp_id.clone(),
        rp_name: rp_name.map(str::to_owned),
        user_handle: encoding::base64url(passkey.user_handle.as_deref().unwrap_or(&user.id)),
        user_name: user.name.clone().unwrap_or_default(),
        user_display_name: user.display_name.clone().unwrap_or_default(),
        private_key: cose_to_pkcs8(&passkey.key)?,
        counter: passkey.counter.unwrap_or(0),
        created_at: Some(chrono::Utc::now().to_rfc3339()),
    })
}

fn decode(b64url: &str, what: &str) -> Result<Vec<u8>> {
    encoding::try_from_base64url(b64url)
        .ok_or_else(|| Error::Crypto(format!("passkey {what} is not valid base64url")))
}

// A credential id is base64url — except in a Bitwarden export, where it is the
// GUID Bitwarden mints for every passkey, whose 16 raw bytes are what it sends
// to the site. Import keeps the string verbatim, so the GUID form is resolved
// here, at the moment the bytes are needed.
pub fn decode_credential_id(id: &str) -> Result<Vec<u8>> {
    if is_guid(id) {
        return hex::decode(id.replace('-', ""))
            .map_err(|_| Error::Crypto("passkey credential id is not a valid GUID".into()));
    }
    decode(id, "credential id")
}

fn is_guid(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, len)| g.len() == len && g.chars().all(|c| c.is_ascii_hexdigit()))
}

// The public half is derived rather than stored: the vault only keeps the
// private key, and COSE wants the affine coordinates alongside `d`.
fn cose_from_secret(secret: &SecretKey) -> CoseKey {
    let point = SigningKey::from(secret)
        .verifying_key()
        .to_encoded_point(false);
    // SAFETY: an uncompressed point always carries both coordinates.
    let (x, y) = (point.x().unwrap(), point.y().unwrap());
    CoseKeyBuilder::new_ec2_priv_key(
        iana::EllipticCurve::P_256,
        x.to_vec(),
        y.to_vec(),
        secret.to_bytes().to_vec(),
    )
    .algorithm(iana::Algorithm::ES256)
    .build()
}

fn secret_from_cose(key: &CoseKey) -> Result<SecretKey> {
    if !matches!(key.kty, RegisteredLabel::Assigned(iana::KeyType::EC2)) {
        return Err(unsupported("only EC2 keys are supported"));
    }
    if !matches!(
        key.alg,
        Some(RegisteredLabelWithPrivate::Assigned(iana::Algorithm::ES256)) | None
    ) {
        return Err(unsupported("only ES256 keys are supported"));
    }
    if param(key, iana::Ec2KeyParameter::Crv)
        .and_then(|v| v.as_integer())
        .map(i128::from)
        != Some(i128::from(iana::EllipticCurve::P_256.to_i64()))
    {
        return Err(unsupported("only the P-256 curve is supported"));
    }
    let d = param(key, iana::Ec2KeyParameter::D)
        .and_then(|v| v.as_bytes())
        .ok_or_else(|| unsupported("COSE key has no private part"))?;
    SecretKey::from_slice(d).map_err(|e| Error::Crypto(format!("invalid P-256 private key: {e}")))
}

fn param(key: &CoseKey, want: iana::Ec2KeyParameter) -> Option<&coset::cbor::value::Value> {
    key.params
        .iter()
        .find(|(label, _)| label == &Label::Int(want.to_i64()))
        .map(|(_, value)| value)
}

fn unsupported(why: &str) -> Error {
    Error::Crypto(format!("unsupported passkey key: {why}"))
}
