//! The wire half of the KeePassXC-Browser protocol: the key exchange, the
//! sealed envelope every later message travels in, and the refusals.
//!
//! The extension makes a fresh X25519 key pair per browser session and opens
//! with `change-public-keys`, the one message sent in the clear. Everything
//! after it is a NaCl box — XSalsa20-Poly1305 under the X25519 shared secret —
//! with a fresh 24-byte nonce per request; the reply is sealed under that
//! nonce plus one, which is also how the extension matches a reply to the
//! request it answers. A refusal is sent in the clear, as KeePassXC sends it:
//! `errorCode` a string of digits, `error` the English text.

use base64::{engine::general_purpose::STANDARD, Engine};
use crypto_box::{aead::Aead, Nonce, PublicKey, SalsaBox, SecretKey};
use serde_json::{json, Map, Value};

/// The KeePassXC release this host answers as. There is no capability
/// negotiation in the protocol: the extension turns features on by this
/// string alone (`keepass.updateFeaturesList` in `background/keepass.js`), so
/// it must claim no more than the actions this host answers. The gates, and
/// what each needs from the host:
///
/// | claims   | the extension turns on             | needs                                   |
/// |----------|------------------------------------|-----------------------------------------|
/// | ≥ 2.6.0  | association at all                 | the core actions in `actions`           |
/// | ≥ 2.6.1  | TOTP from the host (`get-totp`)    | `get-totp`                              |
/// | ≥ 2.7.0  | `generate-password`, favicons      | `generate-password`; favicons are ours  |
/// | ≥ 2.7.7  | passkeys                           | `passkeys-get`, `passkeys-register`     |
/// | ≥ 2.7.10 | a default passkey *group* setting  | groups, which the vault does not have   |
///
/// The highest row this host has every action for, and no higher: raise it in
/// the change that adds the next row's actions, and hold it there with a test.
pub const VERSION: &str = "2.7.6";

pub const NONCE_LEN: usize = 24;

/// KeePassXC's error codes, by their numbers, so the extension's own copy for
/// each applies. Only the ones this host raises are listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Code {
    DatabaseNotOpened = 1,
    ClientPublicKeyNotReceived = 3,
    CannotDecryptMessage = 4,
    ActionCancelledOrDenied = 6,
    CannotEncryptMessage = 7,
    AssociationFailed = 8,
    IncorrectAction = 12,
    EmptyMessageReceived = 13,
    NoUrlProvided = 14,
    NoLoginsFound = 15,
    NoGroupsFound = 16,
    NoValidUuidProvided = 18,
}

impl Code {
    pub fn message(self) -> &'static str {
        match self {
            Code::DatabaseNotOpened => "Database not opened",
            Code::ClientPublicKeyNotReceived => "Client public key not received",
            Code::CannotDecryptMessage => "Cannot decrypt message",
            Code::ActionCancelledOrDenied => "Action cancelled or denied",
            Code::CannotEncryptMessage => "Message encryption failed.",
            Code::AssociationFailed => "Association failed",
            Code::IncorrectAction => "Incorrect action",
            Code::EmptyMessageReceived => "Empty message received",
            Code::NoUrlProvided => "No URL provided",
            Code::NoLoginsFound => "No logins found",
            Code::NoGroupsFound => "No groups found",
            Code::NoValidUuidProvided => "No valid UUID provided",
        }
    }
}

/// A refusal, in the clear. The extension matches one to its request by
/// `action`, since there is no nonce to match on.
pub fn error_reply(action: &str, code: Code) -> Value {
    json!({
        "action": action,
        "errorCode": (code as u8).to_string(),
        "error": code.message(),
    })
}

/// The nonce plus one, little-endian across the whole 24 bytes — libsodium's
/// `sodium_increment`, which is what both KeePassXC and the extension run.
pub fn increment(nonce: &mut [u8; NONCE_LEN]) {
    let mut carry = 1u16;
    for byte in nonce.iter_mut() {
        carry += u16::from(*byte);
        *byte = carry as u8;
        carry >>= 8;
    }
}

fn decode_nonce(text: &str) -> Option<[u8; NONCE_LEN]> {
    STANDARD.decode(text).ok()?.try_into().ok()
}

/// One extension's session: this side's key pair, and once the extension has
/// sent its public key, the box the two share.
pub struct Session {
    secret: SecretKey,
    /// The extension's session public key as it sent it, and the box under it.
    peer: Option<(String, SalsaBox)>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            secret: SecretKey::generate(&mut rand::rngs::OsRng),
            peer: None,
        }
    }

    /// The extension's session public key, base64, once exchanged. `associate`
    /// repeats it inside the sealed message, which is what proves that message
    /// came over this very session.
    pub fn client_key(&self) -> Option<&str> {
        self.peer.as_ref().map(|(key, _)| key.as_str())
    }

    /// `change-public-keys`: adopt the extension's key and answer with ours.
    pub fn exchange(&mut self, request: &Map<String, Value>) -> Value {
        let action = "change-public-keys";
        let key = str_of(request, "publicKey");
        let Some(nonce) = decode_nonce(str_of(request, "nonce")) else {
            return error_reply(action, Code::ClientPublicKeyNotReceived);
        };
        let public = STANDARD
            .decode(key)
            .ok()
            .and_then(|bytes| PublicKey::from_slice(&bytes).ok());
        let Some(public) = public else {
            return error_reply(action, Code::ClientPublicKeyNotReceived);
        };
        self.peer = Some((key.to_string(), SalsaBox::new(&public, &self.secret)));
        let mut next = nonce;
        increment(&mut next);
        json!({
            "action": action,
            "version": VERSION,
            "success": "true",
            "nonce": STANDARD.encode(next),
            "publicKey": STANDARD.encode(self.secret.public_key().as_bytes()),
        })
    }

    /// Open a sealed request: the message inside, and the nonce it came under.
    pub fn open(
        &self,
        request: &Map<String, Value>,
    ) -> Result<(Map<String, Value>, [u8; NONCE_LEN]), Code> {
        let (_, sealed) = self.peer.as_ref().ok_or(Code::ClientPublicKeyNotReceived)?;
        let nonce = decode_nonce(str_of(request, "nonce")).ok_or(Code::CannotDecryptMessage)?;
        let cipher = STANDARD
            .decode(str_of(request, "message"))
            .map_err(|_| Code::CannotDecryptMessage)?;
        let plain = sealed
            .decrypt(Nonce::from_slice(&nonce), cipher.as_slice())
            .map_err(|_| Code::CannotDecryptMessage)?;
        match serde_json::from_slice::<Value>(&plain) {
            Ok(Value::Object(message)) => Ok((message, nonce)),
            _ => Err(Code::CannotDecryptMessage),
        }
    }

    /// Seal a reply under the request's nonce plus one. The message carries the
    /// same nonce, `version` and `success`, which is what the extension checks
    /// before it reads anything else.
    pub fn seal(&self, action: &str, nonce: &[u8; NONCE_LEN], params: Map<String, Value>) -> Value {
        let Some((_, sealed)) = self.peer.as_ref() else {
            return error_reply(action, Code::CannotEncryptMessage);
        };
        let mut next = *nonce;
        increment(&mut next);
        let next_b64 = STANDARD.encode(next);
        let mut message = params;
        message.insert("action".into(), action.into());
        message.insert("version".into(), VERSION.into());
        message.insert("success".into(), "true".into());
        message.insert("nonce".into(), next_b64.clone().into());
        let plain = match serde_json::to_vec(&Value::Object(message)) {
            Ok(plain) => plain,
            Err(_) => return error_reply(action, Code::CannotEncryptMessage),
        };
        match sealed.encrypt(Nonce::from_slice(&next), plain.as_slice()) {
            Ok(cipher) => json!({
                "action": action,
                "message": STANDARD.encode(cipher),
                "nonce": next_b64,
            }),
            Err(_) => error_reply(action, Code::CannotEncryptMessage),
        }
    }
}

/// A string field, or `""` for one that is absent or not a string.
pub fn str_of<'a>(map: &'a Map<String, Value>, key: &str) -> &'a str {
    map.get(key).and_then(Value::as_str).unwrap_or("")
}
