//! The way to a broker, as the extension's proxy reaches the app: one
//! request out, one reply in, the sealed channel on top unchanged.
//!
//! There are two transports. With `ROWEL_BROKER` unset the broker is the app
//! on this machine, reached over the data directory's socket (a named pipe on
//! Windows) in native messaging frames — a length, then the JSON. With
//! `ROWEL_BROKER=ws://host:port[/path]` or `wss://…` the broker is a `rowel
//! serve` on another box (a CI runner, a dev VM), reached over a WebSocket.
//! There a binary message carries exactly one frame's JSON body and nothing
//! else: the WebSocket already frames for us, so the length prefix would only
//! say what the message's own length says. `wss://` verifies the broker's
//! certificate against the platform's roots, plus the PEM `ROWEL_BROKER_CA`
//! names for a broker signed by a CA of one's own.
//!
//! Either way the host pushes clear, unsolicited signals — `database-locked`,
//! `database-unlocked`, `rowel-signal` — to every connection, so a reply is
//! whatever arrives that is not one of those ([`is_signal`]).

use std::env;
use std::fmt;
use std::io;
use std::net::TcpStream;
use std::sync::Arc;

use http::Uri;
use interprocess::local_socket::{prelude::*, Stream};
use rowel_broker::client::Transport;
use rowel_broker::frame;
use rowel_broker::socket::{root_dir, socket_name};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::CertificateDer;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Connector, Message, WebSocket};

/// Names the broker to reach instead of the app's socket.
pub const BROKER_VAR: &str = "ROWEL_BROKER";
/// A PEM file of CA certificates to trust for `wss://` beside the platform's.
pub const BROKER_CA_VAR: &str = "ROWEL_BROKER_CA";

/// A connected channel to a broker.
pub enum Socket {
    /// The app on this machine, in native messaging frames.
    Local(Stream),
    /// A `rowel serve` elsewhere, one frame per binary message. Boxed: the
    /// WebSocket carries its buffers inline, and the local socket is a
    /// handle, so the enum would otherwise be the size of the larger for
    /// every connection the app ever sees.
    Web(Box<WebSocket<MaybeTlsStream<TcpStream>>>),
}

/// Connect to the broker `ROWEL_BROKER` names, or with it unset to the app:
/// the way the extension's proxy does. The error says why not: the variable
/// is not a broker URL, there is no data directory for this user, or nothing
/// is listening — which is the app not running, or the broker not reachable.
pub fn connect() -> io::Result<Socket> {
    match broker()? {
        None => connect_local(),
        Some(broker) => connect_to(&broker),
    }
}

/// Where `ROWEL_BROKER` points, if anywhere, parsed once so `status` can say
/// where it tried. `Display` is the scheme, host, port and path — never the
/// query, which is where a token would travel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Broker {
    uri: Uri,
}

impl Broker {
    /// `value` as a broker URL, or the one error every bad value gets, which
    /// names the variable and the forms it takes and not the value: a value
    /// with a secret in it is a mistake, not something to print.
    pub fn parse(value: &str) -> io::Result<Self> {
        let uri: Option<Uri> = value.parse().ok();
        match uri {
            Some(uri) if matches!(uri.scheme_str(), Some("ws" | "wss")) && uri.host().is_some() => {
                Ok(Self { uri })
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{BROKER_VAR} must be unset, ws://host:port[/path] or wss://host:port[/path]"
                ),
            )),
        }
    }

    fn tls(&self) -> bool {
        self.uri.scheme_str() == Some("wss")
    }

    fn host(&self) -> &str {
        self.uri.host().expect("checked by parse")
    }

    fn port(&self) -> u16 {
        self.uri
            .port_u16()
            .unwrap_or(if self.tls() { 443 } else { 80 })
    }

    /// The broker's HTTP origin — `https://host[:port]` for `wss://`,
    /// `http://` for `ws://` — where a Rowel Server answers what is not a
    /// sealed action, such as the workload identity challenge (`workload`).
    /// The WebSocket's path is the channel's, not the origin's.
    pub fn origin(&self) -> String {
        let scheme = if self.tls() { "https" } else { "http" };
        match self.uri.port_u16() {
            Some(port) => format!("{scheme}://{}:{port}", self.host()),
            None => format!("{scheme}://{}", self.host()),
        }
    }
}

impl fmt::Display for Broker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}://{}",
            self.uri.scheme_str().unwrap_or("ws"),
            self.host()
        )?;
        if let Some(port) = self.uri.port_u16() {
            write!(f, ":{port}")?;
        }
        f.write_str(self.uri.path())
    }
}

/// The broker the environment names: `None` with `ROWEL_BROKER` unset or
/// empty, which is the app.
pub fn broker() -> io::Result<Option<Broker>> {
    broker_from(env::var_os(BROKER_VAR).map(|value| value.to_string_lossy().into_owned()))
}

fn broker_from(value: Option<String>) -> io::Result<Option<Broker>> {
    match value.as_deref() {
        None | Some("") => Ok(None),
        Some(value) => Broker::parse(value).map(Some),
    }
}

/// The app's socket.
pub fn connect_local() -> io::Result<Socket> {
    let root = root_dir().ok_or_else(|| io::Error::other("no data directory for this user"))?;
    socket_name(&root)
        .and_then(Stream::connect)
        .map(Socket::Local)
}

/// A WebSocket to `broker`: TCP to its host and port, TLS over that for
/// `wss://`, then the WebSocket handshake at its path.
pub fn connect_to(broker: &Broker) -> io::Result<Socket> {
    let stream = TcpStream::connect((broker.host(), broker.port()))?;
    let connector = if broker.tls() {
        Connector::Rustls(tls_config()?)
    } else {
        Connector::Plain
    };
    let (socket, _response) =
        tungstenite::client_tls_with_config(broker.uri.clone(), stream, None, Some(connector))
            .map_err(|error| match error {
                tungstenite::HandshakeError::Failure(error) => failed(error),
                // The stream is blocking, so a handshake never has to be
                // resumed; this is the library's shape, not a case.
                tungstenite::HandshakeError::Interrupted(_) => {
                    io::Error::new(io::ErrorKind::Interrupted, "the handshake was interrupted")
                }
            })?;
    Ok(Socket::Web(Box::new(socket)))
}

/// The TLS client for `wss://`: the platform's roots, and the CA
/// `ROWEL_BROKER_CA` names, over the ring provider the proxy installs, so the
/// CLI links one TLS stack whichever of its commands runs.
fn tls_config() -> io::Result<Arc<rustls::ClientConfig>> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut roots = rustls::RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    roots.add_parsable_certificates(native.certs);
    if let Some(path) = env::var_os(BROKER_CA_VAR) {
        let shown = path.to_string_lossy().into_owned();
        let certificates = CertificateDer::pem_file_iter(&path)
            .map_err(|error| io::Error::other(format!("{BROKER_CA_VAR} {shown}: {error}")))?;
        for certificate in certificates {
            let certificate = certificate
                .map_err(|error| io::Error::other(format!("{BROKER_CA_VAR} {shown}: {error}")))?;
            roots.add(certificate).map_err(|error| {
                io::Error::other(format!("{BROKER_CA_VAR} {shown}: not a CA: {error}"))
            })?;
        }
    } else if roots.is_empty() {
        // Nothing to verify against is a configuration to fix, not a
        // handshake to fail: say which store could not be read.
        let reason = native
            .errors
            .first()
            .map(|error| error.to_string())
            .unwrap_or_else(|| "none found".to_string());
        return Err(io::Error::other(format!(
            "no root certificates to verify the broker with ({reason}); set {BROKER_CA_VAR}"
        )));
    }
    Ok(Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

impl Transport for Socket {
    fn roundtrip(&mut self, request: &[u8]) -> io::Result<Vec<u8>> {
        match self {
            Socket::Local(stream) => {
                frame::write(stream, request)?;
                loop {
                    let reply = frame::read(stream)?.ok_or_else(closed)?;
                    if !is_signal(&reply) {
                        return Ok(reply);
                    }
                }
            }
            Socket::Web(socket) => {
                socket
                    .send(Message::binary(request.to_vec()))
                    .map_err(failed)?;
                loop {
                    // Pings are answered by the library on the next read or
                    // write; a pong is one we never asked for. Both are the
                    // transport's business, not a reply.
                    match socket.read().map_err(failed)? {
                        Message::Binary(body) if is_signal(&body) => continue,
                        Message::Binary(body) => return Ok(body.into()),
                        Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                        // A text message is not a frame of ours: whatever is
                        // on the other end is not a broker, and the peer
                        // dropping us reads the same as a closed socket, so
                        // `Agent::call` reconnects once as it does for the app.
                        Message::Text(_) | Message::Close(_) => return Err(closed()),
                    }
                }
            }
        }
    }
}

/// Whether `frame` is one of the host's clear, unsolicited signals rather
/// than a reply: the three signal actions, and no nonce, which every reply
/// to a sealed request carries.
pub fn is_signal(frame: &[u8]) -> bool {
    let Ok(serde_json::Value::Object(message)) = serde_json::from_slice(frame) else {
        return false;
    };
    matches!(
        message.get("action").and_then(|action| action.as_str()),
        Some("database-locked" | "database-unlocked" | "rowel-signal")
    ) && !message.contains_key("nonce")
}

fn closed() -> io::Error {
    io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "the broker closed the connection",
    )
}

/// A WebSocket failure as an `io::Error`: the peer going away is the same
/// end-of-stream the socket reports, an I/O error is itself, a certificate
/// the roots do not vouch for names the variable that would.
fn failed(error: tungstenite::Error) -> io::Error {
    match error {
        tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => closed(),
        tungstenite::Error::Io(error) => error,
        tungstenite::Error::Tls(error) => io::Error::other(format!(
            "TLS to the broker failed ({error}); a CA of your own goes in {BROKER_CA_VAR}"
        )),
        other => io::Error::other(other),
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::thread;

    use rowel_broker::actions::Connection;
    use rowel_broker::client::{Peer, PeerError};
    use rowel_broker::signal::ROWEL_SIGNAL;
    use rowel_broker::testing::Mock;
    use serde_json::json;

    use super::*;

    const LOCKED: &[u8] = br#"{"action":"database-locked"}"#;

    /// A broker on the loopback address: one connection, a `Connection<Mock>`
    /// answering each binary message, and before the reply to each the noise
    /// `noise` pushes — the clear signals a live host sends, and a ping.
    fn serve(noise: bool) -> Broker {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let mut connection = Connection::new(Mock::unlocked());
            loop {
                let request = match socket.read() {
                    Ok(Message::Binary(body)) => body,
                    Ok(_) => continue,
                    Err(_) => break,
                };
                if noise {
                    socket.send(Message::Ping(vec![1].into())).unwrap();
                    socket.send(Message::binary(ROWEL_SIGNAL)).unwrap();
                    socket.send(Message::binary(LOCKED)).unwrap();
                }
                let reply = connection.handle(&request);
                socket.send(Message::binary(reply)).unwrap();
            }
        });
        Broker::parse(&format!("ws://{address}/broker")).unwrap()
    }

    #[test]
    fn a_peer_exchanges_keys_and_calls_over_a_websocket() {
        let broker = serve(false);
        let socket = connect_to(&broker).unwrap();
        let mut peer = Peer::connect(socket, "rowel-mcp").unwrap();
        let reply = peer.call("get-databasehash", json!({})).unwrap();
        assert_eq!(reply["hash"], "abc123");
    }

    #[test]
    fn signals_and_pings_before_the_reply_are_skipped() {
        let broker = serve(true);
        let socket = connect_to(&broker).unwrap();
        let mut peer = Peer::connect(socket, "rowel-mcp").unwrap();
        let reply = peer.call("get-databasehash", json!({})).unwrap();
        assert_eq!(reply["hash"], "abc123");
    }

    #[test]
    fn a_broker_that_hangs_up_reads_as_end_of_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let _ = socket.read();
            let _ = socket.close(None);
            while socket.read().is_ok() {}
        });
        let broker = Broker::parse(&format!("ws://{address}")).unwrap();
        let socket = connect_to(&broker).unwrap();
        match Peer::connect(socket, "rowel-mcp") {
            Err(PeerError::Io(error)) => assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof),
            Err(other) => panic!("expected end of stream, got {other:?}"),
            Ok(_) => panic!("expected end of stream, got a connection"),
        }
    }

    #[test]
    fn nothing_listening_is_an_io_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let broker = Broker::parse(&format!("ws://{address}")).unwrap();
        assert!(connect_to(&broker).is_err());
    }

    #[test]
    fn the_variable_unset_or_empty_is_the_app() {
        assert_eq!(broker_from(None).unwrap(), None);
        assert_eq!(broker_from(Some(String::new())).unwrap(), None);
    }

    #[test]
    fn the_origin_is_the_brokers_host_over_http_without_the_channels_path() {
        let origin = |value: &str| Broker::parse(value).unwrap().origin();
        assert_eq!(
            origin("wss://broker.example:8443/v1/agent"),
            "https://broker.example:8443"
        );
        assert_eq!(
            origin("wss://broker.example/v1/agent"),
            "https://broker.example"
        );
        assert_eq!(origin("ws://127.0.0.1:7401"), "http://127.0.0.1:7401");
    }

    #[test]
    fn a_value_that_is_not_a_broker_url_is_a_clear_error() {
        for value in [
            "nonsense",
            "http://host:1",
            "ws://",
            "host:7401",
            "wss:///path",
        ] {
            let error = broker_from(Some(value.into())).unwrap_err();
            let text = error.to_string();
            assert!(text.contains("ROWEL_BROKER"), "{value}: {text}");
            assert!(text.contains("ws://host:port[/path]"), "{value}: {text}");
        }
        // The value is never repeated: it may be a URL with a secret in it.
        let value = "http://user:hunter2@host:1";
        let text = broker_from(Some(value.into())).unwrap_err().to_string();
        assert!(!text.contains("hunter2"), "the value is echoed: {text}");
    }

    #[test]
    fn a_broker_is_shown_without_its_query() {
        let broker = Broker::parse("wss://vm.example:7401/rowel?token=secret").unwrap();
        assert_eq!(broker.to_string(), "wss://vm.example:7401/rowel");
        assert!(broker.tls());
        assert_eq!(broker.port(), 7401);
        let broker = Broker::parse("ws://vm.example").unwrap();
        assert_eq!(broker.to_string(), "ws://vm.example/");
        assert!(!broker.tls());
        assert_eq!(broker.port(), 80);
        assert_eq!(Broker::parse("wss://vm.example").unwrap().port(), 443);
    }

    #[test]
    fn a_signal_is_told_from_a_reply() {
        assert!(is_signal(ROWEL_SIGNAL));
        assert!(is_signal(LOCKED));
        assert!(is_signal(br#"{"action":"database-unlocked"}"#));
        assert!(!is_signal(
            br#"{"action":"change-public-keys","nonce":"n","publicKey":"k","success":"true"}"#
        ));
        assert!(!is_signal(
            br#"{"action":"get-databasehash","errorCode":"1","error":"Database not opened"}"#
        ));
        assert!(!is_signal(b"not json"));
    }
}
