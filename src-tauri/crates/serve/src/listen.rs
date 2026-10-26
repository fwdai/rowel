//! Where agents and extensions reach the broker: the data directory's socket
//! — the same path the app would listen on, so the extension and the CLI on
//! this box find it unchanged; the broker lock keeps the two from serving
//! it at once — and a WebSocket, plain behind a reverse proxy or with TLS
//! from PEM files, carrying the same frames (spec `04`: a binary message is
//! exactly one frame's JSON body).
//!
//! Over the WebSocket each connection is three things: a tokio task reading
//! messages, a std thread owning the `Connection` and serving requests one
//! at a time — `request_grant` blocks for up to the ask timeout, which must
//! not stall the runtime — and a writer task that sends what the thread
//! replies and what the host pushes (the lock signals, registered through
//! `server::register` with a sink into the same channel). No frame is ever
//! logged.
//!
//! The port may face anyone the operator lets reach it, and a client is let
//! in only after an association ask is answered, so everything before that
//! answer is bounded: a ceiling on open connections, a deadline on the
//! handshake, the broker's frame cap on a message, and a short queue of
//! requests per connection that stops the reader when it is full. The same
//! person who could fill the queue could also hold an association ask open
//! for its timeout; what they cannot do is run the process out of threads or
//! memory by trying.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use futures_util::{future, Sink, SinkExt, Stream, StreamExt};
use interprocess::local_socket::{prelude::*, Listener};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc as async_mpsc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use rowel_broker::actions::{Connection, Host};
use rowel_broker::frame;
use rowel_broker::server;

use crate::config::Tls;
use crate::host::HeadlessHost;

/// Serve the data directory's socket (bound with `server::bind`, behind the
/// broker lock) for the life of the process, each connection on a thread of
/// its own through the broker's loop.
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

/// How many WebSocket connections are served at once. Each one is a thread
/// and two tasks, and an unknown client's first request holds its thread for
/// the whole ask timeout, so without a ceiling whoever can reach the port
/// could run the process out of threads. Past it a connection is closed
/// before the handshake and the agent retries. The slot belongs to the
/// thread, not the socket: a client that sends a request and hangs up still
/// holds its slot until that request is answered, so hanging up is not a
/// way around the ceiling.
pub const MAX_CONNECTIONS: usize = 256;

/// How long a connection has to finish the TLS and WebSocket handshakes. An
/// idle TCP connection must not hold one of the slots.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// How many requests a connection may have waiting on its thread. When the
/// queue is full the reader stops taking messages off the socket, so a
/// client that floods frames while its ask waits fills its own send buffer,
/// not the broker's memory.
const REQUEST_BACKLOG: usize = 4;

/// Accept WebSocket connections on `listener` for the life of the process,
/// up to `MAX_CONNECTIONS` at a time.
pub async fn serve_ws(listener: TcpListener, tls: Option<TlsAcceptor>, host: HeadlessHost) {
    serve_ws_limited(listener, tls, host, MAX_CONNECTIONS).await
}

/// `serve_ws` with the ceiling as a parameter, so a test can reach it.
pub async fn serve_ws_limited(
    listener: TcpListener,
    tls: Option<TlsAcceptor>,
    host: HeadlessHost,
    max_connections: usize,
) {
    let slots = Arc::new(Semaphore::new(max_connections));
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                log::warn!("rowel serve: WebSocket accept failed: {e}");
                continue;
            }
        };
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            log::warn!(
                "rowel serve: {max_connections} WebSocket connections are open; {peer} was turned away"
            );
            drop(stream);
            continue;
        };
        let host = host.clone();
        let tls = tls.clone();
        tokio::spawn(async move {
            match tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(stream, tls)).await {
                Ok(Ok(socket)) => connection(socket, host, slot).await,
                Ok(Err(e)) => {
                    log::debug!("rowel serve: WebSocket handshake with {peer} failed: {e}")
                }
                Err(_) => log::debug!("rowel serve: WebSocket handshake with {peer} timed out"),
            }
        });
    }
}

/// The byte stream under a WebSocket, TLS or not, once accepted.
type Transport = Box<dyn ByteStream>;

/// What the two kinds of accepted stream have in common, as one trait so it
/// can be boxed.
trait ByteStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> ByteStream for S {}

/// The TLS handshake when there is one, then the WebSocket's, with the
/// broker's frame cap as the most a message may carry: a larger one ends
/// the connection in the library instead of being buffered whole.
async fn handshake(
    stream: TcpStream,
    tls: Option<TlsAcceptor>,
) -> Result<WebSocketStream<Transport>, String> {
    let transport: Transport = match tls {
        Some(acceptor) => Box::new(acceptor.accept(stream).await.map_err(|e| e.to_string())?),
        None => Box::new(stream),
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(frame::MAX_FRAME))
        .max_frame_size(Some(frame::MAX_FRAME));
    tokio_tungstenite::accept_async_with_config(transport, Some(config))
        .await
        .map_err(|e| e.to_string())
}

/// One tokio-tungstenite connection, served through [`serve_connection`]:
/// a binary message is a frame, text and the control messages are the
/// library's or nobody's, and a close, a raw frame or an error ends it.
async fn connection<S>(socket: WebSocketStream<S>, host: HeadlessHost, slot: OwnedSemaphorePermit)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (sink, stream) = socket.split();
    let sink = sink.with(|body: Vec<u8>| {
        future::ready(Ok::<_, tokio_tungstenite::tungstenite::Error>(
            Message::binary(body),
        ))
    });
    let stream = stream.map(|message| match message {
        Ok(Message::Binary(body)) => Inbound::Frame(body.to_vec()),
        Ok(Message::Text(_)) | Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => Inbound::Ignored,
        Ok(Message::Close(_)) | Ok(Message::Frame(_)) | Err(_) => Inbound::End,
    });
    serve_connection(stream, sink, host, Some(slot)).await
}

/// A message off a WebSocket, as the serving loop reads it, whichever
/// library read it: one frame's JSON body, something that is not a frame of
/// ours (text, a ping, a pong), or the end of the connection.
pub enum Inbound {
    Frame(Vec<u8>),
    Ignored,
    End,
}

/// One WebSocket connection to `host`, served until it closes or the host is
/// switched off: `incoming` is the socket's reading half as [`Inbound`]s,
/// `outgoing` its writing half taking each frame as one binary message. The
/// same loop serves `rowel serve`'s listener and a Rowel Server's agent
/// endpoint, so the bounds this module states hold for both. `slot` is the
/// connection's place under a ceiling, when it is under one; it goes to the
/// thread, which may outlive the socket by one request.
pub async fn serve_connection<H, R, W>(
    mut incoming: R,
    mut outgoing: W,
    host: H,
    slot: Option<OwnedSemaphorePermit>,
) where
    H: Host + Send + 'static,
    R: Stream<Item = Inbound> + Unpin,
    W: Sink<Vec<u8>> + Unpin + Send + 'static,
{
    // Replies and signals, to the one writer.
    let (out, mut queued) = async_mpsc::channel::<Vec<u8>>(64);
    // Requests, to the one thread, as many as the backlog allows.
    let (requests_in, mut requests) = async_mpsc::channel::<Vec<u8>>(REQUEST_BACKLOG);

    let signals = out.clone();
    let registered = server::register(Box::new(move |message| {
        signals
            .blocking_send(message.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the connection ended"))
    }));

    let replies = out;
    thread::spawn(move || {
        let _slot = slot;
        let mut connection = Connection::new(host);
        while let Some(request) = requests.blocking_recv() {
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
        while let Some(body) = queued.recv().await {
            if outgoing.send(body).await.is_err() {
                break;
            }
        }
    });

    while let Some(message) = incoming.next().await {
        match message {
            // Awaited: with the backlog full this is where the reader waits,
            // and the socket goes unread until the thread catches up.
            Inbound::Frame(body) => {
                if requests_in.send(body).await.is_err() {
                    break;
                }
            }
            Inbound::Ignored => {}
            Inbound::End => break,
        }
    }
    // Dropping the request sender ends the thread; forgetting the sink ends
    // the signals; the writer ends when the last sender is gone.
    drop(requests_in);
    server::forget(registered);
    writer.abort();
}
