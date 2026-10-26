//! The agent surface end to end on a loopback port: the challenge, the
//! WebSocket with an agent associating through the CLI's peer, and an ask
//! reaching a member's approvals stream and answered over HTTP.

use std::io;
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

use axum::Router;
use serde_json::{json, Value};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use rowel_broker::client::{Peer, Transport};
use rowel_broker::grant::Approval as Gate;

use super::org::{header_request, hello, Org, STRIPE};
use crate::identity::workload::expected_audience;
use crate::{agent, approvals};

const ASK_TIMEOUT_MS: u64 = 120_000;

/// The CLI's WebSocket transport, as `transport::Socket::Web` speaks it:
/// one frame per binary message, anything else skipped.
struct Ws(WebSocket<MaybeTlsStream<TcpStream>>);

impl Transport for Ws {
    fn roundtrip(&mut self, request: &[u8]) -> io::Result<Vec<u8>> {
        self.0
            .send(Message::binary(request.to_vec()))
            .map_err(io::Error::other)?;
        loop {
            if let Message::Binary(body) = self.0.read().map_err(io::Error::other)? {
                return Ok(body.into());
            }
        }
    }
}

/// The agent and approval routes on a loopback port, on the tenant's
/// runtime; the address.
fn serve(org: &Org) -> String {
    let listener = org
        .runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let router = Router::new()
        .merge(agent::router())
        .merge(approvals::router())
        .with_state(org.state.clone());
    org.runtime.spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    address
}

#[test]
fn an_agent_takes_the_challenge_and_associates_over_the_websocket() {
    let org = Org::new(ASK_TIMEOUT_MS);
    org.stripe_rule(Gate::Auto);
    let address = serve(&org);

    let challenge: Value = org.runtime.block_on(async {
        reqwest::get(format!("http://{address}/v1/agent/challenge"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    });
    let audience = challenge["audience"].as_str().unwrap();
    assert_eq!(audience, org.state.tenant_audience().unwrap());
    assert!(!challenge["nonce"].as_str().unwrap().is_empty());

    let (socket, _) = tungstenite::connect(format!("ws://{address}/v1/agent")).unwrap();
    let mut peer = Peer::connect(Ws(socket), "rowel-cli").unwrap();
    let token = org.token_for(&expected_audience(audience, &peer.public_key()).unwrap());
    let key = peer.public_key();
    let reply = peer
        .call(
            "associate",
            json!({ "key": key, "idKey": "run-key-1", "hello": hello("agent", Some(&token)) }),
        )
        .unwrap();
    assert_eq!(reply["clientId"], "run-key-1");
    let granted = peer.call("request-grant", header_request("ship")).unwrap();
    assert_eq!(granted["state"], "active");
    assert_eq!(granted["items"][0]["id"], STRIPE);
}

/// The first `event: approval` frame's data off an SSE response.
async fn next_approval(response: &mut reqwest::Response) -> Value {
    let mut buffer = String::new();
    loop {
        let chunk = tokio::time::timeout(Duration::from_secs(10), response.chunk())
            .await
            .expect("an approval within ten seconds")
            .unwrap()
            .expect("the stream stays open");
        buffer.push_str(std::str::from_utf8(&chunk).unwrap());
        if let Some(at) = buffer.find("event: approval\n") {
            let frame = &buffer[at..];
            if let Some(end) = frame.find("\n\n") {
                let data = frame[..end]
                    .lines()
                    .find_map(|line| line.strip_prefix("data: "))
                    .unwrap();
                return serde_json::from_str(data).unwrap();
            }
        }
    }
}

#[test]
fn an_ask_reaches_the_approvers_stream_and_its_answer_lands_once() {
    let org = Org::new(ASK_TIMEOUT_MS);
    let address = serve(&org);
    let store = &org.state.store;
    let session_of = |member: &str, device: &str| {
        store
            .create_session(member, Some(device), 3_600_000)
            .unwrap()
            .1
            .to_string()
    };
    let approver = session_of(&org.approver.0.id, &org.approver.1.id);
    let reader = session_of(&org.reader.0.id, &org.reader.1.id);
    let (pending_device, _) = org.server.pending_device(&org.approver.0);
    let unapproved = session_of(&org.approver.0.id, &pending_device.id);

    let asked = {
        let mut agent = org.associated("run-key-1", "agent");
        thread::spawn(move || agent.call("request-grant", header_request("deploy 1.2")))
    };
    let http = reqwest::Client::new();
    let url = |path: &str| format!("http://{address}{path}");
    org.runtime.block_on(async {
        let stream = |access: &str| {
            http.get(url("/v1/members/me/approvals/stream"))
                .bearer_auth(access)
                .send()
        };
        assert_eq!(stream(&unapproved).await.unwrap().status(), 403);
        let mut open = stream(&approver).await.unwrap();
        assert_eq!(open.status(), 200);
        assert!(open.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream"));
        let approval = next_approval(&mut open).await;
        assert_eq!(approval["agentName"], "release-bot");
        assert_eq!(approval["lockboxName"], "Deploy");
        assert_eq!(approval["targets"][0]["kind"], "http");
        let id = approval["id"].as_str().unwrap();

        let answer = |access: String, id: String, body: Value| {
            http.post(url(&format!("/v1/approvals/{id}")))
                .bearer_auth(access)
                .json(&body)
                .send()
        };
        let pick = json!({ "answer": { "itemId": STRIPE, "standingDays": null } });
        let status = |response: reqwest::Response| response.status().as_u16();
        assert_eq!(
            status(
                answer(reader.clone(), id.into(), pick.clone())
                    .await
                    .unwrap()
            ),
            403
        );
        assert_eq!(
            status(
                answer(approver.clone(), "nope".into(), pick.clone())
                    .await
                    .unwrap()
            ),
            404
        );
        assert_eq!(
            status(
                answer(approver.clone(), id.into(), pick.clone())
                    .await
                    .unwrap()
            ),
            204
        );
        assert_eq!(
            status(
                answer(approver.clone(), id.into(), json!({ "answer": null }))
                    .await
                    .unwrap()
            ),
            409
        );
    });
    let reply = asked.join().unwrap().unwrap();
    assert_eq!(reply["state"], "active");
}

#[test]
fn a_session_without_a_token_hears_nothing() {
    let org = Org::new(ASK_TIMEOUT_MS);
    let address = serve(&org);
    let status = org.runtime.block_on(async {
        reqwest::get(format!("http://{address}/v1/members/me/approvals/stream"))
            .await
            .unwrap()
            .status()
    });
    assert_eq!(status, 401);
}
