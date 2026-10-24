//! The client half of the sealed channel: what the extension does in
//! `keepass.js`, as Rust, so the CLI (and later the remote transports) speak
//! to a host with the same code the protocol tests drive one with.
//!
//! A [`Channel`] is the key pair and, once exchanged, the box shared with the
//! host; it builds the clear `change-public-keys`, seals a request and opens
//! the reply the way the extension checks it — nonce plus one, `success`, the
//! action it asked — or reads the refusal the host sent in the clear. A
//! [`Peer`] runs a channel over a [`Transport`], one frame out and one in,
//! whatever carries the frames.

use std::fmt;
use std::io;

use base64::{engine::general_purpose::STANDARD, Engine};
use crypto_box::{aead::Aead, Nonce, PublicKey, SalsaBox, SecretKey};
use serde_json::{json, Map, Value};

use crate::protocol::{increment, str_of, Code, NONCE_LEN};

/// Our X25519 key pair, and the host's box once the keys are exchanged.
pub struct Channel {
    secret: SecretKey,
    host: Option<SalsaBox>,
}

impl Default for Channel {
    fn default() -> Self {
        Self::new()
    }
}

impl Channel {
    pub fn new() -> Self {
        Self {
            secret: SecretKey::generate(&mut rand::rngs::OsRng),
            host: None,
        }
    }

    /// Our session public key, base64, as `change-public-keys` sends it and
    /// as `associate` repeats it inside the sealed message.
    pub fn public_key(&self) -> String {
        STANDARD.encode(self.secret.public_key().as_bytes())
    }

    /// The clear `change-public-keys` request and the nonce it went out
    /// under, which `accept_exchange` checks the reply against.
    pub fn exchange_request(&self, client_id: &str) -> (Value, [u8; NONCE_LEN]) {
        let nonce = fresh_nonce();
        let request = json!({
            "action": "change-public-keys",
            "publicKey": self.public_key(),
            "nonce": STANDARD.encode(nonce),
            "clientID": client_id,
        });
        (request, nonce)
    }

    /// Adopt the host's key from its reply to the request sent under `nonce`;
    /// the host's `version` is returned. A refusal is the host's own
    /// (`ClientPublicKeyNotReceived` for a key it could not read); anything
    /// else that is not a well-formed exchange is a bad reply.
    pub fn accept_exchange(
        &mut self,
        nonce: &[u8; NONCE_LEN],
        reply: &Value,
    ) -> Result<String, ChannelError> {
        let reply = reply.as_object().ok_or(ChannelError::BadReply)?;
        if let Some(refused) = Refused::read(reply) {
            return Err(ChannelError::Refused(refused));
        }
        if str_of(reply, "nonce") != STANDARD.encode(next(nonce))
            || str_of(reply, "success") != "true"
        {
            return Err(ChannelError::BadReply);
        }
        let public = STANDARD
            .decode(str_of(reply, "publicKey"))
            .ok()
            .and_then(|bytes| PublicKey::from_slice(&bytes).ok())
            .ok_or(ChannelError::BadReply)?;
        self.host = Some(SalsaBox::new(&public, &self.secret));
        Ok(str_of(reply, "version").to_string())
    }

    /// A sealed request for `action` carrying `message` (`action` is set
    /// inside, as the host requires it to match the envelope), and its nonce.
    pub fn seal(
        &self,
        client_id: &str,
        action: &str,
        message: Value,
    ) -> Result<(Value, [u8; NONCE_LEN]), ChannelError> {
        let sealed = self.host.as_ref().ok_or(ChannelError::NotExchanged)?;
        let mut inner = match message {
            Value::Object(map) => map,
            Value::Null => Map::new(),
            _ => return Err(ChannelError::Sealing),
        };
        inner.insert("action".into(), action.into());
        let nonce = fresh_nonce();
        let plain = serde_json::to_vec(&Value::Object(inner)).map_err(|_| ChannelError::Sealing)?;
        let cipher = sealed
            .encrypt(Nonce::from_slice(&nonce), plain.as_slice())
            .map_err(|_| ChannelError::Sealing)?;
        let request = json!({
            "action": action,
            "message": STANDARD.encode(cipher),
            "nonce": STANDARD.encode(nonce),
            "clientID": client_id,
        });
        Ok((request, nonce))
    }

    /// The message inside the host's reply to the request for `action` sent
    /// under `nonce` — checked the way the extension checks it: the envelope's
    /// nonce is ours plus one, the message repeats it, says `success` and
    /// answers the action we asked — or the refusal it carries in the clear.
    /// The envelope fields (`action`, `nonce`, `success`, `version`) stay in
    /// the map, as the extension sees them.
    pub fn open(
        &self,
        action: &str,
        nonce: &[u8; NONCE_LEN],
        reply: &Value,
    ) -> Result<Map<String, Value>, ChannelError> {
        let sealed = self.host.as_ref().ok_or(ChannelError::NotExchanged)?;
        let reply = reply.as_object().ok_or(ChannelError::BadReply)?;
        if let Some(refused) = Refused::read(reply) {
            return Err(ChannelError::Refused(refused));
        }
        let expected = next(nonce);
        let reply_nonce = str_of(reply, "nonce");
        if reply_nonce != STANDARD.encode(expected) {
            return Err(ChannelError::BadReply);
        }
        let cipher = STANDARD
            .decode(str_of(reply, "message"))
            .map_err(|_| ChannelError::BadReply)?;
        let plain = sealed
            .decrypt(Nonce::from_slice(&expected), cipher.as_slice())
            .map_err(|_| ChannelError::BadReply)?;
        let Ok(Value::Object(message)) = serde_json::from_slice::<Value>(&plain) else {
            return Err(ChannelError::BadReply);
        };
        if str_of(&message, "success") != "true"
            || str_of(&message, "nonce") != reply_nonce
            || str_of(&message, "action") != action
        {
            return Err(ChannelError::BadReply);
        }
        Ok(message)
    }
}

fn fresh_nonce() -> [u8; NONCE_LEN] {
    rand::random()
}

fn next(nonce: &[u8; NONCE_LEN]) -> [u8; NONCE_LEN] {
    let mut next = *nonce;
    increment(&mut next);
    next
}

/// What can go wrong on the channel itself, apart from the host saying no.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelError {
    /// A request before `accept_exchange`: there is no box to seal it in.
    NotExchanged,
    /// The host's reply is not one this protocol produces: a nonce that is
    /// not ours plus one, a box that does not open, an answer to another
    /// action.
    BadReply,
    /// The message could not be sealed, which is a request that is not an
    /// object or one JSON cannot carry.
    Sealing,
    /// The host refused, in the clear.
    Refused(Refused),
}

impl fmt::Display for ChannelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChannelError::NotExchanged => write!(f, "keys not exchanged"),
            ChannelError::BadReply => write!(f, "malformed reply from the host"),
            ChannelError::Sealing => write!(f, "request could not be sealed"),
            ChannelError::Refused(refused) => write!(f, "{refused}"),
        }
    }
}

impl std::error::Error for ChannelError {}

/// A clear refusal: the code (a known [`Code`], or only the raw number for one
/// this client does not know), the host's text, and `retryAfterMs` when the
/// host said when to ask again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub code: Option<Code>,
    pub raw: u8,
    pub message: String,
    pub retry_after_ms: Option<u64>,
}

impl Refused {
    /// The refusal `reply` carries, if it is one: `errorCode` is a string of
    /// digits, as KeePassXC writes it. A reply with an `errorCode` that is not
    /// a number is not a refusal this protocol produces, and reads as none.
    fn read(reply: &Map<String, Value>) -> Option<Self> {
        let raw: u8 = str_of(reply, "errorCode").parse().ok()?;
        Some(Self {
            code: Code::from_number(raw),
            raw,
            message: str_of(reply, "error").to_string(),
            retry_after_ms: reply.get("retryAfterMs").and_then(Value::as_u64),
        })
    }
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (code {})", self.message, self.raw)
    }
}

/// One frame out, one frame in — a socket, a pipe, or an in-process
/// `Connection` in tests.
pub trait Transport {
    fn roundtrip(&mut self, request: &[u8]) -> io::Result<Vec<u8>>;
}

/// A [`Channel`] over a [`Transport`]: `call` seals, sends and opens.
pub struct Peer<T: Transport> {
    transport: T,
    channel: Channel,
    client_id: String,
    version: String,
}

impl<T: Transport> Peer<T> {
    /// Run the key exchange over `transport` under `client_id`, the name the
    /// envelope carries beside every request.
    pub fn connect(mut transport: T, client_id: &str) -> Result<Self, PeerError> {
        let mut channel = Channel::new();
        let (request, nonce) = channel.exchange_request(client_id);
        let reply = roundtrip(&mut transport, &request)?;
        let version = channel.accept_exchange(&nonce, &reply)?;
        Ok(Self {
            transport,
            channel,
            client_id: client_id.to_string(),
            version,
        })
    }

    /// The KeePassXC release the host answers as.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Our session public key, which `associate` repeats inside its message.
    pub fn public_key(&self) -> String {
        self.channel.public_key()
    }

    /// One sealed request and its opened reply, or why there is none: the
    /// transport failed, the channel could not make sense of the reply, or
    /// the host refused.
    pub fn call(&mut self, action: &str, message: Value) -> Result<Map<String, Value>, PeerError> {
        let (request, nonce) = self.channel.seal(&self.client_id, action, message)?;
        let reply = roundtrip(&mut self.transport, &request)?;
        Ok(self.channel.open(action, &nonce, &reply)?)
    }
}

fn roundtrip<T: Transport>(transport: &mut T, request: &Value) -> Result<Value, PeerError> {
    let raw = serde_json::to_vec(request).map_err(|_| ChannelError::Sealing)?;
    let reply = transport.roundtrip(&raw)?;
    serde_json::from_slice(&reply).map_err(|_| ChannelError::BadReply.into())
}

#[derive(Debug)]
pub enum PeerError {
    Io(io::Error),
    Channel(ChannelError),
    Refused(Refused),
}

impl From<io::Error> for PeerError {
    fn from(error: io::Error) -> Self {
        PeerError::Io(error)
    }
}

// A refusal is the host's answer, not a fault of the channel, so a caller
// matches it at the top rather than inside `Channel`.
impl From<ChannelError> for PeerError {
    fn from(error: ChannelError) -> Self {
        match error {
            ChannelError::Refused(refused) => PeerError::Refused(refused),
            other => PeerError::Channel(other),
        }
    }
}

impl fmt::Display for PeerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PeerError::Io(error) => write!(f, "{error}"),
            PeerError::Channel(error) => write!(f, "{error}"),
            PeerError::Refused(refused) => write!(f, "{refused}"),
        }
    }
}

impl std::error::Error for PeerError {}
