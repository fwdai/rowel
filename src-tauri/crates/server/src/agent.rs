//! Where agents reach the server (`docs/server-api.md`, "Agent API"): the
//! unauthenticated challenge an agent binds its workload token to, and the
//! WebSocket the sealed channel runs over — `rowel serve`'s serving loop
//! (`rowel_serve::listen::serve_connection`) with an `OrgHost` per
//! connection, under the same ceiling on connections and the broker's frame
//! cap on a message. Association is by proof (`host`); everything after it
//! is the broker's protocol unchanged.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures_util::{future, SinkExt, StreamExt};
use tokio::runtime::Handle;
use tokio::sync::OwnedSemaphorePermit;

use rowel_broker::frame;
use rowel_serve::listen::{serve_connection, Inbound};

use crate::host::{self, OrgHost};
use crate::identity::workload::Challenge;
use crate::state::{now_ms, AppState};

/// `GET /v1/agent/challenge` and `GET /v1/agent` (the WebSocket).
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/agent/challenge", get(challenge))
        .route("/v1/agent", get(upgrade))
}

async fn challenge(State(state): State<Arc<AppState>>) -> Result<Json<Challenge>, StatusCode> {
    let audience = state.tenant_audience().map_err(|e| {
        log::warn!("rowel server: the tenant row is unreadable: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(state.challenges.mint(&audience, now_ms())))
}

// Past the ceiling the upgrade is refused before it happens and the agent
// retries; the slot goes to the connection's thread, which may outlive the
// socket by one request (`listen::MAX_CONNECTIONS`).
async fn upgrade(State(state): State<Arc<AppState>>, ws: WebSocketUpgrade) -> Response {
    let Ok(slot) = state.agent_slots.clone().try_acquire_owned() else {
        log::warn!("rowel server: every agent connection slot is taken; one was turned away");
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    ws.max_message_size(frame::MAX_FRAME)
        .max_frame_size(frame::MAX_FRAME)
        .on_upgrade(move |socket| connection(socket, state, slot))
}

/// One agent connection: axum's messages as the serving loop reads them —
/// a binary message is a frame, text and the control messages are not ours,
/// a close or an error ends it.
async fn connection(socket: WebSocket, state: Arc<AppState>, slot: OwnedSemaphorePermit) {
    let host = OrgHost::new(state, Handle::current());
    let (sink, stream) = socket.split();
    let sink =
        sink.with(|body: Vec<u8>| future::ready(Ok::<_, axum::Error>(Message::binary(body))));
    let stream = stream.map(|message| match message {
        Ok(Message::Binary(body)) => Inbound::Frame(body.to_vec()),
        Ok(Message::Text(_)) | Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => Inbound::Ignored,
        Ok(Message::Close(_)) | Err(_) => Inbound::End,
    });
    serve_connection(stream, sink, host, Some(slot)).await;
}

/// The agents' timer, once a second for the life of the process (`host::
/// tick`), off the runtime's workers: it takes the vaults' locks and the
/// store's connection.
pub async fn run_ticker(state: Arc<AppState>) {
    let mut every = tokio::time::interval(Duration::from_secs(1));
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        every.tick().await;
        let state = state.clone();
        if let Err(e) = tokio::task::spawn_blocking(move || host::tick(&state, now_ms())).await {
            log::warn!("rowel server: the agents' tick failed: {e}");
        }
    }
}
