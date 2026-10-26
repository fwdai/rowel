//! `OrgHost` driven as the broker's connection drives it: association by
//! workload proof, grants decided by the admin's rules or by a member's
//! answer, the sidecar's `resolve`, revocation, freezing, idle sealing and
//! the audit mirror.

use std::collections::BTreeSet;
use std::thread;

use serde_json::json;

use rowel_broker::agents::GrantAnswer;
use rowel_broker::grant::Approval as Gate;
use rowel_broker::protocol::Code;

use super::org::{
    associate, header_request, hello, refused, resolve_request, row, Org, KEY_IDLE_MS, STRIPE,
    STRIPE_KEY,
};
use crate::approvals::{AnswerError, AskTarget};
use crate::host;
use crate::state::now_ms;
use crate::store::{AuditFilter, PendingState};

const ASK_TIMEOUT_MS: u64 = 120_000;

// --- association --------------------------------------------------------------------

#[test]
fn a_proof_for_this_channel_lets_the_agent_in_as_its_definition() {
    let org = Org::new(ASK_TIMEOUT_MS);
    let mut peer = org.connect();
    let token = org.token(&peer.public_key());
    let reply = associate(&mut peer, "run-key-1", &hello("agent", Some(&token))).unwrap();
    assert_eq!(reply["id"], "release-bot");
    assert_eq!(reply["clientId"], "run-key-1");

    let stored = org.state.store.client("run-key-1").unwrap().unwrap();
    assert_eq!(stored.agent_id.as_deref(), Some(org.agent.id.as_str()));
    assert_eq!(stored.name, "release-bot");
    let rows = org.rows();
    let events: Vec<&str> = rows.iter().map(|r| r.event.as_str()).collect();
    assert_eq!(events, ["key_unwrapped", "associated"]);
    assert_eq!(rows[0].actor_kind, "server");
    assert_eq!(rows[1].actor_kind, "agent");
    assert_eq!(rows[1].actor_id.as_deref(), Some("run-key-1"));
}

#[test]
fn a_token_is_good_once_and_on_its_own_channel_only() {
    let org = Org::new(ASK_TIMEOUT_MS);
    let mut first = org.connect();
    let token = org.token(&first.public_key());
    associate(&mut first, "run-key-1", &hello("agent", Some(&token))).unwrap();
    // The same token again on the same channel: the replay cache.
    let again = associate(&mut first, "run-key-2", &hello("agent", Some(&token)));
    assert_eq!(refused(again.unwrap_err()), Code::AssociationFailed);
    // On another channel: the audience names the first channel's key.
    let mut second = org.connect();
    let lifted = associate(&mut second, "run-key-3", &hello("agent", Some(&token)));
    assert_eq!(refused(lifted.unwrap_err()), Code::AssociationFailed);
    assert!(org.state.store.client("run-key-3").unwrap().is_none());
}

#[test]
fn a_token_for_another_audience_or_no_proof_at_all_is_refused() {
    let org = Org::new(ASK_TIMEOUT_MS);
    let mut peer = org.connect();
    // The tenant's bare audience, which only a fixed-audience policy takes.
    let bare = org.token_for(&org.state.tenant_audience().unwrap());
    let outcome = associate(&mut peer, "run-key-1", &hello("agent", Some(&bare)));
    assert_eq!(refused(outcome.unwrap_err()), Code::AssociationFailed);
    let elsewhere = org.token_for("rowel:another-tenant");
    let outcome = associate(&mut peer, "run-key-1", &hello("agent", Some(&elsewhere)));
    assert_eq!(refused(outcome.unwrap_err()), Code::AssociationFailed);
    // No proof is never a question for a person.
    let outcome = associate(&mut peer, "run-key-1", &hello("agent", None));
    assert_eq!(refused(outcome.unwrap_err()), Code::AssociationFailed);
    assert!(org.state.store.clients().unwrap().is_empty());
    assert!(org
        .state
        .approvals
        .pending_for(&org.state.store, &org.approver.0.id)
        .is_empty());
}

// --- grants -------------------------------------------------------------------------

#[test]
fn an_admin_auto_rule_grants_and_the_sidecar_resolves_by_id_and_by_name() {
    let org = Org::new(ASK_TIMEOUT_MS);
    org.stripe_rule(Gate::Auto);
    let mut agent = org.associated("run-key-1", "agent");
    let reply = agent
        .call("request-grant", header_request("check the balance"))
        .unwrap();
    assert_eq!(reply["state"], "active");
    assert_eq!(reply["items"][0]["id"], STRIPE);

    let mut sidecar = org.associated("sidecar-key-1", "sidecar");
    let by_id = sidecar
        .call(
            "resolve",
            resolve_request(&reply["grantId"], &org.lockbox.id),
        )
        .unwrap();
    assert_eq!(by_id["value"], STRIPE_KEY);
    let by_name = sidecar
        .call("resolve", resolve_request(&reply["grantId"], "deploy"))
        .unwrap();
    assert_eq!(by_name["value"], STRIPE_KEY);
    let other = sidecar.call("resolve", resolve_request(&reply["grantId"], "billing"));
    assert_eq!(refused(other.unwrap_err()), Code::GrantDenied);

    let events = org.events();
    let names: Vec<&str> = events.iter().map(|(event, _)| event.as_str()).collect();
    assert_eq!(
        names,
        [
            "associated",
            "requested",
            "granted",
            "associated",
            "used",
            "used"
        ]
    );
    assert_eq!(
        events[4].1.as_deref(),
        Some(r#"{"via":"resolve","field":"apiKey"}"#)
    );
}

#[test]
fn an_ask_goes_to_the_approvers_and_the_first_answer_wins() {
    let org = Org::new(ASK_TIMEOUT_MS);
    let asked = {
        let mut agent = org.associated("run-key-1", "agent");
        thread::spawn(move || agent.call("request-grant", header_request("deploy 1.2")))
    };
    let id = org.routed_ask();
    let store = &org.state.store;
    let approvals = &org.state.approvals;
    let pending = approvals.pending_for(store, &org.approver.0.id);
    let ask = &pending[0];
    assert_eq!(ask.lockbox_id, org.lockbox.id);
    assert_eq!(ask.lockbox_name, "Deploy");
    assert_eq!(ask.agent_name, "release-bot");
    assert_eq!(ask.reason.as_deref(), Some("deploy 1.2"));
    assert_eq!(ask.candidates[0].title, "Stripe (live)");
    assert_eq!(
        ask.targets,
        [AskTarget::Http {
            host: "api.stripe.com".into(),
            path: "/v1/balance".into(),
            methods: vec!["GET".into()],
        }]
    );
    let shape = serde_json::to_value(ask).unwrap();
    assert_eq!(shape["mode"], "header");
    assert_eq!(shape["stepUp"], false);
    assert_eq!(shape["targets"][0]["kind"], "http");
    assert!(shape["expiresAtMs"].as_i64().unwrap() > now_ms());
    assert!(!shape.to_string().contains(STRIPE_KEY));

    // A reader of the lockbox is not asked, and may not answer.
    assert!(approvals.pending_for(store, &org.reader.0.id).is_empty());
    let pick = || {
        Some(GrantAnswer {
            item_id: STRIPE.into(),
            standing_days: None,
        })
    };
    assert_eq!(
        approvals.answer(store, &id, &org.reader.1, pick()),
        Err(AnswerError::Forbidden)
    );
    assert_eq!(
        approvals.answer(store, "no-such-ask", &org.approver.1, pick()),
        Err(AnswerError::NotFound)
    );

    approvals
        .answer(store, &id, &org.approver.1, pick())
        .unwrap();
    let reply = asked.join().unwrap().unwrap();
    assert_eq!(reply["state"], "active");
    assert_eq!(
        approvals.answer(store, &id, &org.approver.1, None),
        Err(AnswerError::Settled)
    );
    assert_eq!(org.pending_state(&id), PendingState::Answered.as_str());
    assert!(approvals.pending_for(store, &org.approver.0.id).is_empty());
    assert_eq!(
        org.events()[1..],
        [row("requested", None), row("granted", None)]
    );
}

#[test]
fn a_refusal_is_denied_on_record() {
    let org = Org::new(ASK_TIMEOUT_MS);
    let asked = {
        let mut agent = org.associated("run-key-1", "agent");
        thread::spawn(move || agent.call("request-grant", header_request("deploy")))
    };
    let id = org.routed_ask();
    org.state
        .approvals
        .answer(&org.state.store, &id, &org.approver.1, None)
        .unwrap();
    assert_eq!(
        refused(asked.join().unwrap().unwrap_err()),
        Code::GrantDenied
    );
    assert_eq!(
        org.events()[1..],
        [
            row("requested", None),
            row("denied", Some(r#"{"reason":"user"}"#))
        ]
    );
}

#[test]
fn an_unanswered_ask_times_out_and_its_row_expires() {
    let org = Org::new(300);
    let mut agent = org.associated("run-key-1", "agent");
    let outcome = agent.call("request-grant", header_request("nobody home"));
    assert_eq!(refused(outcome.unwrap_err()), Code::GrantDenied);
    assert_eq!(
        org.events().last().unwrap(),
        &row("denied", Some(r#"{"reason":"timeout"}"#))
    );
    let db = rusqlite::Connection::open(org.server.data_dir().join(crate::store::DB_FILE)).unwrap();
    let states: Vec<String> = db
        .prepare("SELECT state FROM pending_approvals")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(states, [PendingState::Expired.as_str()]);
}

// --- what ends access ---------------------------------------------------------------

#[test]
fn a_disabled_definition_ends_its_sessions_at_the_next_request() {
    let org = Org::new(ASK_TIMEOUT_MS);
    org.stripe_rule(Gate::Auto);
    let mut agent = org.associated("run-key-1", "agent");
    agent.call("request-grant", header_request("a")).unwrap();
    org.state
        .store
        .set_agent_enabled(&org.agent.id, false)
        .unwrap();
    let outcome = agent.call("request-grant", header_request("b"));
    assert_eq!(refused(outcome.unwrap_err()), Code::AssociationFailed);
    // And no new proof lets it back in while it stays disabled.
    let mut peer = org.connect();
    let token = org.token(&peer.public_key());
    let again = associate(&mut peer, "run-key-2", &hello("agent", Some(&token)));
    assert_eq!(refused(again.unwrap_err()), Code::AssociationFailed);
}

#[test]
fn a_frozen_lockbox_is_sealed_and_refused_before_policy() {
    let org = Org::new(ASK_TIMEOUT_MS);
    org.stripe_rule(Gate::Auto);
    let mut agent = org.associated("run-key-1", "agent");
    let before = org.rows().len();
    org.state.store.set_frozen(&org.lockbox.id, true).unwrap();
    let outcome = agent.call("request-grant", header_request("frozen"));
    assert_eq!(refused(outcome.unwrap_err()), Code::GrantDenied);
    assert!(!org.state.vaults.get(&org.lockbox.id).unwrap().is_open());
    // Its key is not unwrapped to say so: the lockbox's log is as it was.
    assert_eq!(org.rows().len(), before);
}

#[test]
fn the_lockbox_switch_off_is_a_refusal_on_record_and_revokes_what_was_granted() {
    let org = Org::new(ASK_TIMEOUT_MS);
    org.stripe_rule(Gate::Auto);
    let mut agent = org.associated("run-key-1", "agent");
    let granted = agent.call("request-grant", header_request("a")).unwrap();
    org.state.store.set_agents(&org.lockbox.id, false).unwrap();
    let outcome = agent.call("request-grant", header_request("b"));
    assert_eq!(refused(outcome.unwrap_err()), Code::GrantDenied);
    let events = org.events();
    assert_eq!(
        events[events.len() - 2..],
        [
            row("revoked", Some(r#"{"reason":"lockbox"}"#)),
            row("denied", Some(r#"{"reason":"lockbox"}"#))
        ]
    );
    let status = agent
        .call("grant-status", json!({ "grantId": granted["grantId"] }))
        .unwrap();
    assert_eq!(status["state"], "revoked");
}

#[test]
fn an_idle_lockbox_is_sealed_and_unwrapped_again_by_the_next_request() {
    let org = Org::new(ASK_TIMEOUT_MS);
    org.stripe_rule(Gate::Auto);
    let mut agent = org.associated("run-key-1", "agent");
    let granted = agent.call("request-grant", header_request("a")).unwrap();
    let vault = org.state.vaults.get(&org.lockbox.id).unwrap();

    host::tick(&org.state, now_ms());
    assert!(vault.is_open(), "not idle yet");
    host::tick(&org.state, now_ms() + KEY_IDLE_MS as i64 + 1);
    assert!(!vault.is_open(), "idle: sealed");

    // The seal ended the grant with the session; the next request unwraps.
    let status = agent.call("grant-status", json!({ "grantId": granted["grantId"] }));
    assert_eq!(refused(status.unwrap_err()), Code::GrantDenied);
    assert!(vault.is_open());
    let reply = agent.call("request-grant", header_request("b")).unwrap();
    assert_eq!(reply["state"], "active");
    let unwraps = org
        .rows()
        .iter()
        .filter(|row| row.event == "key_unwrapped")
        .count();
    assert_eq!(unwraps, 2);
}

#[test]
fn the_mirror_holds_every_row_the_lockbox_holds() {
    let org = Org::new(ASK_TIMEOUT_MS);
    org.stripe_rule(Gate::Auto);
    let mut agent = org.associated("run-key-1", "agent");
    let reply = agent.call("request-grant", header_request("a")).unwrap();
    let mut sidecar = org.associated("sidecar-key-1", "sidecar");
    sidecar
        .call(
            "resolve",
            resolve_request(&reply["grantId"], &org.lockbox.id),
        )
        .unwrap();
    agent
        .call("end-grant", json!({ "grantId": reply["grantId"] }))
        .unwrap();
    host::tick(&org.state, now_ms() + KEY_IDLE_MS as i64 + 1);

    let held: BTreeSet<_> = org
        .rows()
        .into_iter()
        .map(|r| (r.writer_id, r.seq, r.hash))
        .collect();
    let mirrored: BTreeSet<_> = org
        .state
        .store
        .audit_export(&AuditFilter {
            lockbox_id: Some(org.lockbox.id.clone()),
            ..AuditFilter::default()
        })
        .unwrap()
        .into_iter()
        .map(|(_, r)| (r.writer_id, r.seq, r.hash))
        .collect();
    assert!(held.len() >= 7, "{held:?}");
    assert_eq!(mirrored, held);
}
