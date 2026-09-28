//! Passkeys through the extension: `passkeys-register` and `passkeys-get`.
//!
//! The extension overrides `navigator.credentials` on every page and hands the
//! page's options to the host. The host plays the WebAuthn *client* — checks
//! the origin and the rpId, writes `clientDataJSON` — and drives
//! `passkey::Authenticator` for the CTAP2 half, against the open vault and with
//! the user's say over each ceremony (`passkey::UserConsent`).
//!
//! ## The wire, as the reference implementations have it
//! Read from `keepassxreboot/keepassxc-browser` (develop) and
//! `keepassxreboot/keepassxc` (develop), so nobody has to fetch them again:
//!
//! - Request (`background/keepass.js`, `passkeysRegister` / `passkeysGet`):
//!   `{ action, publicKey, origin, relatedOrigins, keys }`, plus `groupName`
//!   on a registration. `origin` is `window.location.origin`. Sent without the
//!   extension's 500 ms reply timeout, so the host may take as long as the
//!   user does. `relatedOrigins` and `groupName` are not used here.
//! - `publicKey` (`content/passkeys-utils.js`): the page's options with every
//!   binary field as unpadded base64url — `challenge`, `user.id`, and the `id`
//!   of each `excludeCredentials` / `allowCredentials` entry (`'internal'` is
//!   appended to every allow entry's `transports`). `pubKeyCredParams[].alg`
//!   is a number; `rp`, `rpId`, `authenticatorSelection`, `userVerification`,
//!   `attestation`, `extensions` and `timeout` are passed through.
//! - Reply (`BrowserAction.cpp`): the usual sealed reply with one param,
//!   `response`. On success it is the credential; on failure
//!   `{ errorCode: N }`, a JSON *number* (`BrowserService::getPasskeyError`).
//!   The page script (`content/passkeys-inject.js`) reads
//!   `ret.response.errorCode`; a refusal in the clear instead reaches the page
//!   as `ret === null`, i.e. code 22, so everything past the association
//!   check answers inside `response`.
//! - Credential (`BrowserPasskeys.cpp`, `buildRegisterPublicKeyCredential` /
//!   `buildGetPublicKeyCredential`), rebuilt into a `PublicKeyCredential` by
//!   `content/passkeys.js` `createPublicKeyCredential`, which base64-decodes
//!   `id` into `rawId` and every `response` field it reads, tolerating either
//!   alphabet and no padding:
//!   `{ authenticatorAttachment, id, type: "public-key", response }`, where a
//!   registration's `response` is `{ attestationObject, clientDataJSON,
//!   authenticatorData, publicKey (SPKI DER), publicKeyAlgorithm,
//!   clientExtensionResults }` and an assertion's is `{ authenticatorData,
//!   clientDataJSON, signature, userHandle, clientExtensionResults }`. Which
//!   one the page gets is decided by `response.attestationObject` being
//!   there. `getTransports()` is fixed to `['internal']` on the page side.
//! - `clientDataJSON` (`PasskeyUtils::buildClientDataJson`), in this order:
//!   `{"type":"webauthn.create"|"webauthn.get","challenge":…,"origin":…,
//!   "crossOrigin":false}`, the challenge as the extension sent it.
//! - Checks (`BrowserAction.cpp`, `BrowserPasskeysClient.cpp`,
//!   `PasskeyUtils.cpp`), in order: an empty `publicKey` is 24; an origin that
//!   is not `https://` is 25 (`isOriginAllowedWithLocalhost`: localhost only
//!   behind a setting there, always here); a challenge under 16 characters is
//!   32 and a `user.id` outside 1..=64 bytes is 33 (`checkLimits`, on a
//!   registration); an origin host that is not a domain — an IP address — is
//!   27; an `rpId` that is neither the host nor a registrable suffix of it
//!   (`isRegistrableDomainSuffix`, which refuses a public suffix) is 28; no
//!   supported algorithm is 29. A missing `rpId` is the host.
//! - Outcomes (`BrowserService.cpp`, `showPasskeysRegisterPrompt` /
//!   `showPasskeysAuthenticationPrompt`): an `excludeCredentials` id the
//!   vault holds for the rpId is 21, before the user is asked; a sign-in the
//!   allow list leaves nothing for — or an allow list naming only ids the
//!   vault does not hold — is 15 (`ERROR_KEEPASS_NO_LOGINS_FOUND`), before the
//!   user is asked; a dialog the user cancels is 22; anything else is 31.
//! - Codes (`BrowserMessageBuilder.h`): 20 attestation not supported, 21
//!   credential excluded, 22 request canceled, 23 invalid user verification,
//!   24 empty public key, 25 invalid URL, 26 origin not allowed, 27 domain not
//!   valid, 28 rpId mismatch, 29 no supported algorithms, 30 wait for the
//!   lifetimer, 31 unknown, 32 invalid challenge, 33 invalid user id. The page
//!   turns 27/28 into `SecurityError`, 32/33 into `TypeError`, the rest into
//!   `NotAllowedError`; for 15, 21 and 22 it waits out the request's timeout
//!   first, so a site cannot tell "no passkey" from "the user said no" by
//!   timing.
//!
//! ## What is done by hand, and why
//! `passkey-client` would build `clientDataJSON` and check the rpId, but its
//! check is a bare `ends_with` — `notexample.com` could claim `example.com` —
//! and it leaves `crossOrigin` out. The client role is small, so it is written
//! here with KeePassXC's rules and the public suffix list `passkey-client`
//! itself uses; the CTAP2 half is the vault's `passkey::Authenticator`.
//! Extensions are not supported beyond answering `credProps` (ours are always
//! discoverable); BE/BS are whatever the authenticator sets.

use std::future::Future;

use passkey_types::ctap2::{get_assertion, make_credential, Ctap2Error, StatusCode};
use passkey_types::encoding::{base64url, try_from_base64url};
use passkey_types::webauthn::{
    PublicKeyCredentialDescriptor, PublicKeyCredentialParameters, PublicKeyCredentialType,
    PublicKeyCredentialUserEntity,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use url::{Host, Url};

use super::protocol::{str_of, Code};
use crate::passkey::store::PasskeyVault;
use crate::passkey::{Authenticator, UserConsent};

/// ES256, the one algorithm the vault's keys are.
const ES256: i64 = -7;

/// A registration the page asked for, checked and ready for the authenticator.
pub struct Registration {
    /// The page's origin, serialized: what the user is shown alongside the rpId.
    pub origin: String,
    client_data_json: String,
    request: make_credential::Request,
    /// The page asked whether the credential is discoverable (`credProps`).
    cred_props: bool,
}

/// A sign-in the page asked for, checked and ready for the authenticator.
pub struct Assertion {
    pub origin: String,
    client_data_json: String,
    request: get_assertion::Request,
}

/// `passkeys-register`'s message, read the way KeePassXC reads it.
pub fn registration(message: &Map<String, Value>) -> Result<Registration, Code> {
    let (options, origin) = options_and_origin(message)?;
    let challenge = challenge(options)?;
    let user = options.get("user").and_then(Value::as_object);
    let user_id = user
        .and_then(|user| try_from_base64url(str_of(user, "id")))
        .filter(|id| (1..=64).contains(&id.len()))
        .ok_or(Code::PasskeyInvalidUserId)?;
    let rp = options.get("rp").and_then(Value::as_object);
    let rp_id = rp_id(&origin, rp.and_then(|rp| rp.get("id")))?;

    // An empty list is "anything"; a list without ES256 is nothing we make.
    let params = options.get("pubKeyCredParams").and_then(Value::as_array);
    let es256 = params.is_none_or(|params| {
        params.is_empty()
            || params.iter().any(|p| {
                p.get("alg").and_then(Value::as_i64) == Some(ES256)
                    && p.get("type").and_then(Value::as_str) == Some("public-key")
            })
    });
    if !es256 {
        return Err(Code::PasskeyNoSupportedAlgorithms);
    }

    let client_data_json = client_data("webauthn.create", &challenge, &origin);
    let exclude = descriptors(options.get("excludeCredentials"));
    let cred_props = options
        .get("extensions")
        .and_then(|e| e.get("credProps"))
        .and_then(Value::as_bool)
        == Some(true);
    let name = |key| user.map(|user| str_of(user, key)).unwrap_or_default();
    Ok(Registration {
        origin: origin_of(&origin),
        request: make_credential::Request {
            client_data_hash: Sha256::digest(&client_data_json).to_vec().into(),
            rp: make_credential::PublicKeyCredentialRpEntity {
                id: rp_id,
                name: rp.map(|rp| str_of(rp, "name").to_string()),
            },
            user: PublicKeyCredentialUserEntity {
                id: user_id.into(),
                name: name("name").to_string(),
                display_name: name("displayName").to_string(),
            },
            pub_key_cred_params: vec![PublicKeyCredentialParameters {
                ty: PublicKeyCredentialType::PublicKey,
                alg: coset::iana::Algorithm::ES256,
            }],
            exclude_list: (!exclude.is_empty()).then_some(exclude),
            extensions: None,
            // Not `rk`: the vault makes every credential discoverable anyway
            // (`DiscoverabilitySupport::ForcedDiscoverable`). Verified: an
            // unlocked vault and the user's yes, see `passkey`'s module docs.
            options: make_credential::Options {
                rk: false,
                up: true,
                uv: true,
            },
            pin_auth: None,
            pin_protocol: None,
        },
        client_data_json,
        cred_props,
    })
}

/// `passkeys-get`'s message, read the way KeePassXC reads it.
pub fn assertion(message: &Map<String, Value>) -> Result<Assertion, Code> {
    let (options, origin) = options_and_origin(message)?;
    let challenge = challenge(options)?;
    let rp_id = rp_id(&origin, options.get("rpId"))?;
    // A list the site gave that names nothing readable must not turn into no
    // list at all, which the authenticator reads as "any credential".
    let listed = options
        .get("allowCredentials")
        .and_then(Value::as_array)
        .is_some_and(|list| !list.is_empty());
    let allow = descriptors(options.get("allowCredentials"));
    if listed && allow.is_empty() {
        return Err(Code::NoLoginsFound);
    }
    let client_data_json = client_data("webauthn.get", &challenge, &origin);
    Ok(Assertion {
        origin: origin_of(&origin),
        request: get_assertion::Request {
            rp_id,
            client_data_hash: Sha256::digest(&client_data_json).to_vec().into(),
            allow_list: (!allow.is_empty()).then_some(allow),
            extensions: None,
            options: get_assertion::Options {
                rk: false,
                up: true,
                uv: true,
            },
            pin_auth: None,
            pin_protocol: None,
        },
        client_data_json,
    })
}

/// Create a credential in `vault`, asking through `consent`: the credential as
/// the extension hands it to the page.
pub fn register<V: PasskeyVault>(
    vault: V,
    consent: impl UserConsent + 'static,
    registration: Registration,
) -> Result<Value, Code> {
    let mut authenticator = Authenticator::new(vault, consent);
    let created = run(authenticator.make_credential(registration.request))?;
    let attested = created
        .auth_data
        .attested_credential_data
        .as_ref()
        .ok_or(Code::PasskeyUnknownError)?;
    let public_key = passkey_authenticator::public_key_der_from_cose_key(&attested.key)
        .map_err(|e| unknown(format!("{e:?}")))?;
    let extensions = if registration.cred_props {
        json!({ "credProps": { "rk": true } })
    } else {
        json!({})
    };
    Ok(json!({
        "authenticatorAttachment": "platform",
        "id": base64url(attested.credential_id()),
        "type": "public-key",
        "response": {
            "attestationObject": base64url(&created.as_webauthn_bytes()),
            "authenticatorData": base64url(&created.auth_data.to_vec()),
            "clientDataJSON": base64url(registration.client_data_json.as_bytes()),
            "clientExtensionResults": extensions,
            "publicKey": base64url(&public_key),
            "publicKeyAlgorithm": ES256,
        },
    }))
}

/// Sign in with a credential from `vault`, asking through `consent`: the
/// assertion as the extension hands it to the page.
pub fn assert<V: PasskeyVault>(
    vault: V,
    consent: impl UserConsent + 'static,
    assertion: Assertion,
) -> Result<Value, Code> {
    let mut authenticator = Authenticator::new(vault, consent);
    let signed = run(authenticator.get_assertion(assertion.request))?;
    let credential = signed.credential.ok_or(Code::PasskeyUnknownError)?;
    Ok(json!({
        "authenticatorAttachment": "platform",
        "id": base64url(&credential.id),
        "type": "public-key",
        "response": {
            "authenticatorData": base64url(&signed.auth_data.to_vec()),
            "clientDataJSON": base64url(assertion.client_data_json.as_bytes()),
            "clientExtensionResults": {},
            "signature": base64url(&signed.signature),
            "userHandle": signed.user.map(|user| base64url(&user.id)),
        },
    }))
}

/// The sealed reply's params: the credential under `response`, or the
/// ceremony's error code there, as a number.
pub fn reply(outcome: Result<Value, Code>) -> Map<String, Value> {
    let response = match outcome {
        Ok(credential) => credential,
        Err(code) => json!({ "errorCode": code as u8 }),
    };
    let mut params = Map::new();
    params.insert("response".into(), response);
    params
}

// The authenticator's traits are async, but nothing under them waits on
// anything but the user's answer, which blocks this connection's thread by
// design. A current-thread runtime is all the driving it needs.
fn run<T>(ceremony: impl Future<Output = Result<T, StatusCode>>) -> Result<T, Code> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|e| unknown(e.to_string()))?;
    runtime.block_on(ceremony).map_err(code_of)
}

fn code_of(status: StatusCode) -> Code {
    let is = |error: Ctap2Error| status == StatusCode::from(error);
    if is(Ctap2Error::OperationDenied) {
        Code::PasskeyRequestCanceled
    } else if is(Ctap2Error::CredentialExcluded) {
        Code::PasskeyCredentialExcluded
    } else if is(Ctap2Error::NoCredentials) {
        Code::NoLoginsFound
    } else if is(Ctap2Error::UnsupportedAlgorithm) {
        Code::PasskeyNoSupportedAlgorithms
    } else {
        unknown(format!("{status:?}"))
    }
}

fn unknown(why: String) -> Code {
    log::warn!("browser host: a passkey ceremony failed: {why}");
    Code::PasskeyUnknownError
}

// `publicKey`, and the page's origin if it is one a passkey may be used from:
// HTTPS, or plain HTTP on localhost — the one insecure origin browsers
// themselves treat as secure.
fn options_and_origin(message: &Map<String, Value>) -> Result<(&Map<String, Value>, Url), Code> {
    let options = message
        .get("publicKey")
        .and_then(Value::as_object)
        .filter(|options| !options.is_empty())
        .ok_or(Code::PasskeyEmptyPublicKey)?;
    let origin = Url::parse(str_of(message, "origin")).map_err(|_| Code::PasskeyInvalidUrl)?;
    let local = matches!(origin.host(), Some(Host::Domain(host))
        if host == "localhost" || host.ends_with(".localhost"));
    match origin.scheme() {
        "https" => Ok((options, origin)),
        "http" if local => Ok((options, origin)),
        _ => Err(Code::PasskeyInvalidUrl),
    }
}

// KeePassXC's floor is on the string: 16 characters of base64url.
fn challenge(options: &Map<String, Value>) -> Result<Vec<u8>, Code> {
    let text = str_of(options, "challenge");
    if text.len() < 16 {
        return Err(Code::PasskeyInvalidChallenge);
    }
    try_from_base64url(text).ok_or(Code::PasskeyInvalidChallenge)
}

/// The rpId the page may use from `origin`: the one it `claimed`, if that is
/// the origin's host or a registrable suffix of it, or the host when it
/// claimed none. `login.example.com` may claim `example.com`; it may not
/// claim `co.uk`, `notexample.com` or `other.example.com`.
pub fn rp_id(origin: &Url, claimed: Option<&Value>) -> Result<String, Code> {
    // The url crate has already lowercased the host and put it in punycode.
    let Some(Host::Domain(host)) = origin.host() else {
        return Err(Code::PasskeyDomainNotValid);
    };
    let claimed = match claimed {
        None | Some(Value::Null) => return Ok(host.to_string()),
        Some(Value::String(claimed)) => claimed,
        Some(_) => return Err(Code::PasskeyRpIdMismatch),
    };
    let Ok(Host::Domain(rp_id)) = Host::parse(claimed) else {
        return Err(Code::PasskeyRpIdMismatch);
    };
    if rp_id == host {
        return Ok(rp_id);
    }
    if !host.ends_with(&format!(".{rp_id}")) || super::actions::is_public_suffix(&rp_id) {
        return Err(Code::PasskeyRpIdMismatch);
    }
    Ok(rp_id)
}

// `{"type":…,"challenge":…,"origin":…,"crossOrigin":false}`, in the order
// browsers write it and KeePassXC does — some relying parties compare the
// prefix rather than parse the JSON.
fn client_data(kind: &str, challenge: &[u8], origin: &Url) -> String {
    format!(
        r#"{{"type":"{kind}","challenge":"{}","origin":{},"crossOrigin":false}}"#,
        base64url(challenge),
        Value::from(origin_of(origin)),
    )
}

// `window.location.origin`'s form: no path, no trailing slash, the host in
// punycode.
fn origin_of(url: &Url) -> String {
    url.origin().ascii_serialization()
}

// The credential ids in an allow or exclude list. An entry of another type,
// or with an id that is not base64url, cannot name one of ours.
fn descriptors(list: Option<&Value>) -> Vec<PublicKeyCredentialDescriptor> {
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter(|d| {
            matches!(
                d.get("type").and_then(Value::as_str),
                None | Some("public-key")
            )
        })
        .filter_map(|d| try_from_base64url(str_of(d, "id")))
        .map(|id| PublicKeyCredentialDescriptor {
            ty: PublicKeyCredentialType::PublicKey,
            id: id.into(),
            transports: None,
        })
        .collect()
}
