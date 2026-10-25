//! Where agents and extensions reach the broker: the data directory's socket
//! — the same path the app would listen on, so the extension and the CLI on
//! this box find it unchanged — and a WebSocket, plain behind a reverse
//! proxy or with TLS from PEM files, carrying the same frames (spec `04`:
//! a binary message is exactly one frame's JSON body).
//!
//! Over the WebSocket each connection is three things: a tokio task reading
//! messages, a std thread owning the `Connection` and serving requests one
//! at a time — `request_grant` blocks for up to the ask timeout, which must
//! not stall the runtime — and a writer task that sends what the thread
//! replies and what the host pushes (the lock signals, registered through
//! `server::register` with a sink into the same channel). No frame is ever
//! logged.

use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use interprocess::local_socket::{prelude::*, Listener, Stream};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpListener;
use tokio::sync::mpsc as async_mpsc;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use rowel_broker::actions::{Connection, Host};
use rowel_broker::client::Channel;
use rowel_broker::frame;
use rowel_broker::server;
use rowel_broker::socket::socket_name;

use crate::config::Tls;
use crate::host::HeadlessHost;

/// Why the socket cannot be taken.
#[derive(Debug)]
pub enum SocketError {
    /// Something answers on the socket already: the app serving this data
    /// directory. `server::bind` would overwrite it, and stealing the app's
    /// socket must not be possible by accident.
    Occupied,
    Io(io::Error),
}

impl std::fmt::Display for SocketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Occupied => f.write_str(
                "the Rowel app is serving this data directory; stop it or point rowel serve at another --data-dir",
            ),
            Self::Io(e) => write!(f, "could not listen on the socket: {e}"),
        }
    }
}

impl std::error::Error for SocketError {}

/// How long the probe waits for the app to answer a key exchange.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Whether something is serving `root`'s socket: a connection that is
/// accepted and answers a `change-public-keys`, or accepts and stays silent
/// — a hung app is still the app. A stale socket file refuses the
/// connection, which is the one case the file may be replaced.
pub fn socket_occupied(root: &Path) -> io::Result<bool> {
    let name = socket_name(root)?;
    let mut stream = match Stream::connect(name) {
        Ok(stream) => stream,
        Err(_) => return Ok(false),
    };
    let (request, _nonce) = Channel::new().exchange_request("rowel-serve-probe");
    if frame::write(&mut stream, &serde_json::to_vec(&request)?).is_err() {
        return Ok(true);
    }
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let answered = frame::read(&mut stream).is_ok();
        let _ = sender.send(answered);
    });
    // Answered, silent, or hung up: a live listener took the connection
    // either way. The reader thread is left to end with the stream.
    let _ = receiver.recv_timeout(PROBE_TIMEOUT);
    Ok(true)
}

/// The data directory's socket, after the probe.
pub fn bind_socket(root: &Path) -> Result<Listener, SocketError> {
    if socket_occupied(root).map_err(SocketError::Io)? {
        return Err(SocketError::Occupied);
    }
    server::bind(root).map_err(SocketError::Io)
}

/// Serve `listener` for the life of the process, each connection on a
/// thread of its own through the broker's loop.
pub fn serve_socket(listener: Listener, host: HeadlessHost) {
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => server::serve_stream(stream, host.clone()),
            Err(e) => log::warn!("rowel serve: socket accept failed: {e}"),
        }
    }
}

/// A TLS acceptor from the PEM files `tls` names, over the ring provider.
pub fn tls_acceptor(tls: &Tls) -> io::Result<TlsAcceptor> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certs = CertificateDer::pem_file_iter(&tls.cert)
        .map_err(|e| io::Error::other(format!("{}: {e}", tls.cert.display())))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| io::Error::other(format!("{}: {e}", tls.cert.display())))?;
    let key = PrivateKeyDer::from_pem_file(&tls.key)
        .map_err(|e| io::Error::other(format!("{}: {e}", tls.key.display())))?;
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| io::Error::other(format!("TLS: {e}")))?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// Bind the WebSocket listener at `address`.
pub async fn bind_ws(address: &str) -> io::Result<TcpListener> {
    let address: SocketAddr = address
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "listen.ws must be host:port"))?;
    TcpListener::bind(address).await
}

/// Accept WebSocket connections on `listener` for the life of the process.
pub async fn serve_ws(listener: TcpListener, tls: Option<TlsAcceptor>, host: HeadlessHost) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                log::warn!("rowel serve: WebSocket accept failed: {e}");
                continue;
            }
        };
        let host = host.clone();
        let tls = tls.clone();
        tokio::spawn(async move {
            let outcome = match tls {
                Some(acceptor) => match acceptor.accept(stream).await {
                    Ok(stream) => match tokio_tungstenite::accept_async(stream).await {
                        Ok(socket) => {
                            connection(socket, host).await;
                            Ok(())
                        }
                        Err(e) => Err(e.to_string()),
                    },
                    Err(e) => Err(e.to_string()),
                },
                None => match tokio_tungstenite::accept_async(stream).await {
                    Ok(socket) => {
                        connection(socket, host).await;
                        Ok(())
                    }
                    Err(e) => Err(e.to_string()),
                },
            };
            if let Err(e) = outcome {
                log::debug!("rowel serve: WebSocket handshake with {peer} failed: {e}");
            }
        });
    }
}

/// One WebSocket connection, served until it closes or the host is
/// switched off.
async fn connection<S>(socket: WebSocketStream<S>, host: HeadlessHost)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sink, mut stream) = socket.split();
    // Replies and signals, to the one writer.
    let (out, mut outgoing) = async_mpsc::channel::<Vec<u8>>(64);
    // Requests, to the one thread.
    let (incoming, requests) = mpsc::channel::<Vec<u8>>();

    let signals = out.clone();
    let registered = server::register(Box::new(move |message| {
        signals
            .blocking_send(message.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the connection ended"))
    }));

    let replies = out;
    thread::spawn(move || {
        let mut connection = Connection::new(host);
        for request in requests {
            if !connection.host().enabled() {
                break;
            }
            let reply = connection.handle(&request);
            if replies.blocking_send(reply).is_err() {
                break;
            }
        }
    });

    let writer = tokio::spawn(async move {
        while let Some(body) = outgoing.recv().await {
            if sink.send(Message::binary(body)).await.is_err() {
                break;
            }
        }
    });

    while let Some(message) = stream.next().await {
        match message {
            Ok(Message::Binary(body)) => {
                if incoming.send(body.to_vec()).is_err() {
                    break;
                }
            }
            // A text message is not a frame of ours; pings and pongs are the
            // library's business.
            Ok(Message::Text(_)) | Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
            Ok(Message::Close(_)) | Ok(Message::Frame(_)) | Err(_) => break,
        }
    }
    // Dropping the request sender ends the thread; forgetting the sink ends
    // the signals; the writer ends when the last sender is gone.
    drop(incoming);
    server::forget(registered);
    writer.abort();
}
