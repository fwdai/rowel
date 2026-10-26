//! The admin API: refused to anyone but an admin, and for an admin the
//! lockboxes, agents with their trust policies, rules, members and their
//! devices, the audit export and the tenant's events.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use reqwest::Method;
use serde_json::{json, Value};

use rowel_broker::grant::{Approval, CreatedBy, ItemSelector, Mode, Target};
use rowel_broker::policy::{AgentSelector, Rule};
use rowel_core::store::AuditRecord;

use super::harness::{json, Api};
use crate::store::{LockboxRole, Role};

const ADMIN: &str = "ada@acme.example";

/// A server with Ada as its admin, signed in; her access token.
async fn with_admin() -> (Api, String, String) {
    let api = Api::start().await;
    let ada = api.server.member(ADMIN, Role::Admin);
    let tokens = api.sign_in(ADMIN).await;
    (api, ada.id, tokens.access)
}

fn rule(id: &str) -> Rule {
    Rule {
        id: id.into(),
        agents: AgentSelector::Ids(vec!["release-bot".into()]),
        items: ItemSelector::Ids(vec!["stripe".into()]),
        targets: vec![Target::Http {
            host: "api.stripe.com".into(),
            path: "/v1/*".into(),
            methods: vec![],
        }],
        modes: vec![Mode::Header],
        approval: Approval::Ask,
        ttl_ms: 15 * 60 * 1000,
        max_uses: None,
        reason_required: false,
        approvers: None,
        sidecar_for: None,
        created_at_ms: 0,
        created_by: CreatedBy::User,
        expires_at_ms: None,
    }
}

fn record(writer: &str, seq: i64, ts_ms: i64, actor_id: &str) -> AuditRecord {
    AuditRecord {
        id: format!("{writer}-{seq}"),
        writer_id: writer.into(),
        seq,
        ts_ms,
        prev_hash: [seq as u8; 32],
        hash: [seq as u8 + 1; 32],
        actor_kind: "agent".into(),
        actor_id: Some(actor_id.into()),
        actor_name: Some("release-bot".into()),
        run_id: None,
        acting_for: None,
        grant_id: Some("g1".into()),
        item_id: Some("i1".into()),
        target: Some("api.stripe.com/v1/charges".into()),
        mode: Some("header".into()),
        approval: Some("ask".into()),
        approver: None,
        event: "used".into(),
        detail: None,
    }
}

#[tokio::test]
async fn every_admin_route_refuses_a_member_who_is_not_an_admin() {
    let api = Api::start().await;
    let pat = api.sign_in("pat@acme.example").await;
    let routes = [
        (Method::GET, "/v1/admin/members"),
        (Method::POST, "/v1/admin/members"),
        (Method::PATCH, "/v1/admin/members/m1"),
        (Method::GET, "/v1/admin/members/m1/devices"),
        (Method::POST, "/v1/admin/devices/d1/approve"),
        (Method::POST, "/v1/admin/devices/d1/revoke"),
        (Method::GET, "/v1/admin/lockboxes"),
        (Method::POST, "/v1/admin/lockboxes"),
        (Method::PATCH, "/v1/admin/lockboxes/lb1"),
        (Method::PUT, "/v1/admin/lockboxes/lb1/members/m1"),
        (Method::DELETE, "/v1/admin/lockboxes/lb1/members/m1"),
        (Method::GET, "/v1/admin/agents"),
        (Method::POST, "/v1/admin/agents"),
        (Method::PATCH, "/v1/admin/agents/a1"),
        (Method::DELETE, "/v1/admin/agents/a1"),
        (Method::POST, "/v1/admin/agents/a1/trust"),
        (Method::DELETE, "/v1/admin/trust/t1"),
        (Method::GET, "/v1/admin/rules"),
        (Method::POST, "/v1/admin/rules"),
        (Method::DELETE, "/v1/admin/rules/r1"),
        (Method::GET, "/v1/admin/audit/export"),
        (Method::GET, "/v1/admin/events"),
    ];
    for (method, path) in routes {
        let member = api
            .call(method.clone(), path, Some(&pat.access), Some(json!({})))
            .await;
        assert_eq!(member.status().as_u16(), 403, "{method} {path}");
        let nobody = api.call(method.clone(), path, None, None).await;
        assert_eq!(nobody.status().as_u16(), 401, "{method} {path}");
    }
}

#[tokio::test]
async fn an_admin_keeps_lockboxes_agents_trust_and_rules() {
    let (api, ada, access) = with_admin().await;
    let pat = api.server.member("pat@acme.example", Role::Member);
    let call = |method: Method, path: String, body: Option<Value>| {
        let access = access.clone();
        let api = &api;
        async move { api.call(method, &path, Some(&access), body).await }
    };

    // Lockboxes.
    let created = call(
        Method::POST,
        "/v1/admin/lockboxes".into(),
        Some(json!({ "name": "Deploy" })),
    )
    .await;
    assert_eq!(created.status().as_u16(), 201);
    let lockbox = json(created).await;
    let lockbox_id = lockbox["id"].as_str().unwrap().to_string();
    assert_eq!(lockbox["name"], "Deploy");
    assert_eq!(lockbox["gen"], 1);
    assert_eq!(lockbox["members"], json!([]));
    assert!(lockbox.get("wrappedMaster").is_none());
    let unnamed = call(
        Method::POST,
        "/v1/admin/lockboxes".into(),
        Some(json!({ "name": " " })),
    )
    .await;
    assert_eq!(unnamed.status().as_u16(), 400);

    let patched = json(
        call(
            Method::PATCH,
            format!("/v1/admin/lockboxes/{lockbox_id}"),
            Some(json!({ "frozen": true, "agents": true })),
        )
        .await,
    )
    .await;
    assert_eq!(
        (patched["frozen"].clone(), patched["agents"].clone()),
        (json!(true), json!(true))
    );
    let unknown = call(
        Method::PATCH,
        "/v1/admin/lockboxes/nope".into(),
        Some(json!({ "frozen": true })),
    )
    .await;
    assert_eq!(unknown.status().as_u16(), 404);

    let holder = format!("/v1/admin/lockboxes/{lockbox_id}/members/{}", pat.id);
    let held = call(
        Method::PUT,
        holder.clone(),
        Some(json!({ "role": "approver" })),
    )
    .await;
    assert_eq!(held.status().as_u16(), 204);
    let listed = json(call(Method::GET, "/v1/admin/lockboxes".into(), None).await).await;
    assert_eq!(
        listed[0]["members"],
        json!([{ "memberId": pat.id, "role": "approver" }])
    );
    assert_eq!(
        api.state.store.approvers_of(&lockbox_id).unwrap(),
        vec![pat.id.clone()]
    );
    let bad_role = call(
        Method::PUT,
        holder.clone(),
        Some(json!({ "role": "owner" })),
    )
    .await;
    assert_eq!(bad_role.status().as_u16(), 400);
    let nobody = call(
        Method::PUT,
        format!("/v1/admin/lockboxes/{lockbox_id}/members/nobody"),
        Some(json!({ "role": "reader" })),
    )
    .await;
    assert_eq!(nobody.status().as_u16(), 404);
    assert_eq!(
        call(Method::DELETE, holder, None).await.status().as_u16(),
        204
    );
    assert!(api
        .state
        .store
        .lockbox_members(&lockbox_id)
        .unwrap()
        .is_empty());

    // Agents and their trust policies.
    let agent = call(
        Method::POST,
        "/v1/admin/agents".into(),
        Some(json!({ "name": "release-bot", "lockboxId": lockbox_id })),
    )
    .await;
    assert_eq!(agent.status().as_u16(), 201);
    let agent = json(agent).await;
    let agent_id = agent["id"].as_str().unwrap().to_string();
    assert_eq!(agent["enabled"], true);
    assert_eq!(agent["trust"], json!([]));
    let orphan = call(
        Method::POST,
        "/v1/admin/agents".into(),
        Some(json!({ "name": "x", "lockboxId": "nope" })),
    )
    .await;
    assert_eq!(orphan.status().as_u16(), 400);

    let trust_path = format!("/v1/admin/agents/{agent_id}/trust");
    let trust = call(
        Method::POST,
        trust_path.clone(),
        Some(json!({
            "issuer": "https://token.actions.githubusercontent.com",
            "claims": { "repository": "acme/deploy", "ref": "refs/heads/main" },
            "fixedAudience": false,
        })),
    )
    .await;
    assert_eq!(trust.status().as_u16(), 201);
    let trust_id = json(trust).await["id"].as_str().unwrap().to_string();
    let everyone = call(
        Method::POST,
        trust_path.clone(),
        Some(json!({ "issuer": "https://token.actions.githubusercontent.com", "claims": {} })),
    )
    .await;
    assert_eq!(everyone.status().as_u16(), 400);

    // The sidecar's own identity, and an extension, which no workload
    // token becomes.
    let sidecar = call(
        Method::POST,
        trust_path.clone(),
        Some(json!({
            "issuer": "https://token.actions.githubusercontent.com",
            "claims": { "repository": "acme/deploy", "job_workflow_ref": "acme/proxy" },
            "clientKind": "sidecar",
        })),
    )
    .await;
    assert_eq!(sidecar.status().as_u16(), 201);
    assert_eq!(json(sidecar).await["clientKind"], "sidecar");
    let extension = call(
        Method::POST,
        trust_path,
        Some(json!({
            "issuer": "https://token.actions.githubusercontent.com",
            "claims": { "repository": "acme/deploy" },
            "clientKind": "extension",
        })),
    )
    .await;
    assert_eq!(extension.status().as_u16(), 400);

    let agents = json(call(Method::GET, "/v1/admin/agents".into(), None).await).await;
    assert_eq!(agents[0]["trust"][0]["id"], trust_id);
    assert_eq!(agents[0]["trust"][0]["claims"]["repository"], "acme/deploy");
    assert_eq!(agents[0]["trust"][0]["clientKind"], "agent");
    assert_eq!(agents[0]["trust"][1]["clientKind"], "sidecar");

    let renamed = json(
        call(
            Method::PATCH,
            format!("/v1/admin/agents/{agent_id}"),
            Some(json!({ "name": "deploy-bot", "enabled": false })),
        )
        .await,
    )
    .await;
    assert_eq!(renamed["name"], "deploy-bot");
    assert_eq!(renamed["enabled"], false);
    assert_eq!(renamed["trust"].as_array().unwrap().len(), 2);

    let untrust = format!("/v1/admin/trust/{trust_id}");
    assert_eq!(
        call(Method::DELETE, untrust.clone(), None)
            .await
            .status()
            .as_u16(),
        204
    );
    assert_eq!(
        call(Method::DELETE, untrust, None).await.status().as_u16(),
        404
    );

    // Rules.
    let added = call(
        Method::POST,
        "/v1/admin/rules".into(),
        Some(json!({ "lockboxId": lockbox_id, "rule": rule("stripe-ask") })),
    )
    .await;
    assert_eq!(added.status().as_u16(), 201);
    let added = json(added).await;
    assert_eq!(added["rule"]["createdBy"], json!({ "admin": "server" }));
    let rules = json(call(Method::GET, "/v1/admin/rules".into(), None).await).await;
    assert_eq!(rules.as_array().unwrap().len(), 1);
    assert_eq!(rules[0]["lockboxId"], lockbox_id);
    assert_eq!(rules[0]["rule"]["id"], "stripe-ask");
    assert_eq!(api.state.store.rules_for(&lockbox_id).unwrap().len(), 1);
    let twice = call(
        Method::POST,
        "/v1/admin/rules".into(),
        Some(json!({ "rule": rule("stripe-ask") })),
    )
    .await;
    assert_eq!(twice.status().as_u16(), 409);
    let tenant_wide = call(
        Method::POST,
        "/v1/admin/rules".into(),
        Some(json!({ "rule": rule("everywhere") })),
    )
    .await;
    assert_eq!(json(tenant_wide).await["lockboxId"], Value::Null);
    let unrule = "/v1/admin/rules/stripe-ask".to_string();
    assert_eq!(
        call(Method::DELETE, unrule.clone(), None)
            .await
            .status()
            .as_u16(),
        204
    );
    assert_eq!(
        call(Method::DELETE, unrule, None).await.status().as_u16(),
        404
    );

    let gone = format!("/v1/admin/agents/{agent_id}");
    assert_eq!(
        call(Method::DELETE, gone.clone(), None)
            .await
            .status()
            .as_u16(),
        204
    );
    let after = call(Method::PATCH, gone, Some(json!({ "enabled": true }))).await;
    assert_eq!(after.status().as_u16(), 404);

    // Every change is a tenant event naming Ada.
    let events = json(
        call(
            Method::GET,
            "/v1/admin/events?since=0&limit=1000".into(),
            None,
        )
        .await,
    )
    .await;
    let changes: Vec<&str> = events
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["event"] != "signed_in")
        .map(|e| {
            assert_eq!(e["memberId"], ada.as_str());
            e["event"].as_str().unwrap()
        })
        .collect();
    assert_eq!(
        changes,
        [
            "lockbox_created",
            "lockbox_updated",
            "lockbox_member_set",
            "lockbox_member_removed",
            "agent_added",
            "trust_added",
            "trust_added",
            "agent_updated",
            "trust_removed",
            "rule_added",
            "rule_added",
            "rule_removed",
            "agent_removed",
        ]
    );
    let limited = json(call(Method::GET, "/v1/admin/events?limit=2".into(), None).await).await;
    assert_eq!(limited.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn an_admin_adds_approves_and_offboards_members() {
    let (api, ada, access) = with_admin().await;
    let call = |method: Method, path: String, body: Option<Value>| {
        let access = access.clone();
        let api = &api;
        async move { api.call(method, &path, Some(&access), body).await }
    };

    let added = call(
        Method::POST,
        "/v1/admin/members".into(),
        Some(json!({ "email": "sam@acme.example", "name": "Sam", "role": "member" })),
    )
    .await;
    assert_eq!(added.status().as_u16(), 201);
    let sam_id = json(added).await["id"].as_str().unwrap().to_string();
    let twice = call(
        Method::POST,
        "/v1/admin/members".into(),
        Some(json!({ "email": "SAM@acme.example", "role": "member" })),
    )
    .await;
    assert_eq!(twice.status().as_u16(), 409);
    let not_an_address = call(
        Method::POST,
        "/v1/admin/members".into(),
        Some(json!({ "email": "sam", "role": "member" })),
    )
    .await;
    assert_eq!(not_an_address.status().as_u16(), 400);
    let members = json(call(Method::GET, "/v1/admin/members".into(), None).await).await;
    assert_eq!(members.as_array().unwrap().len(), 2);

    let sam_path = format!("/v1/admin/members/{sam_id}");
    let promoted = json(
        call(
            Method::PATCH,
            sam_path.clone(),
            Some(json!({ "role": "admin" })),
        )
        .await,
    )
    .await;
    assert_eq!(promoted["role"], "admin");
    call(
        Method::PATCH,
        sam_path.clone(),
        Some(json!({ "role": "member" })),
    )
    .await;

    // Sam's first device is let in by the admin.
    let sam = api.sign_in("sam@acme.example").await;
    let secret = crypto_box::SecretKey::generate(&mut rand::thread_rng());
    let public = STANDARD.encode(secret.public_key().as_bytes());
    let enrolled = json(
        api.call(
            Method::POST,
            "/v1/members/enroll",
            Some(&sam.access),
            Some(json!({ "devicePubkey": public, "deviceName": "Sam's laptop" })),
        )
        .await,
    )
    .await;
    let device_id = enrolled["deviceId"].as_str().unwrap().to_string();
    let devices = json(call(Method::GET, format!("{sam_path}/devices"), None).await).await;
    assert_eq!(devices[0]["state"], "pending");
    assert_eq!(devices[0]["memberId"], sam_id);
    let approved = call(
        Method::POST,
        format!("/v1/admin/devices/{device_id}/approve"),
        None,
    )
    .await;
    assert_eq!(approved.status().as_u16(), 200);
    let approved = json(approved).await;
    assert_eq!(approved["state"], "approved");
    assert_eq!(approved["approvedBy"], ada.as_str());

    let lockbox = api.server.lockbox("Deploy");
    api.state
        .store
        .add_lockbox_member(&lockbox.id, &sam_id, LockboxRole::Reader)
        .unwrap();
    let listing = json(
        api.call(
            Method::GET,
            "/v1/members/me/lockboxes",
            Some(&sam.access),
            None,
        )
        .await,
    )
    .await;
    assert!(listing[0]["wrappedMaster"].is_string());
    assert!(api
        .state
        .store
        .device_master(&lockbox.id, &device_id)
        .unwrap()
        .is_some());

    // An admin never takes themselves out.
    let ada_path = format!("/v1/admin/members/{ada}");
    for change in [json!({ "role": "member" }), json!({ "disabled": true })] {
        let refused = call(Method::PATCH, ada_path.clone(), Some(change)).await;
        assert_eq!(refused.status().as_u16(), 409);
    }

    // Offboarding: the sessions end, the devices are revoked and the
    // masters sealed to them go.
    let disabled = json(
        call(
            Method::PATCH,
            sam_path.clone(),
            Some(json!({ "disabled": true })),
        )
        .await,
    )
    .await;
    assert!(disabled["disabledAtMs"].is_i64());
    let me = api
        .call(Method::GET, "/v1/members/me", Some(&sam.access), None)
        .await;
    assert_eq!(me.status().as_u16(), 401);
    assert_eq!(api.refresh(&sam.refresh).await.status().as_u16(), 401);
    let device = api.state.store.device(&device_id).unwrap().unwrap();
    assert_eq!(device.state, crate::store::DeviceState::Revoked);
    assert_eq!(
        api.state
            .store
            .device_master(&lockbox.id, &device_id)
            .unwrap(),
        None
    );
    let back = call(Method::PATCH, sam_path, Some(json!({ "disabled": false }))).await;
    assert_eq!(back.status().as_u16(), 409);

    // A revoked device stays out; revoking is idempotent; unknown is 404.
    let pat = api.server.member("pat@acme.example", Role::Member);
    let (pending, _) = api.server.pending_device(&pat);
    let revoke = format!("/v1/admin/devices/{}/revoke", pending.id);
    assert_eq!(
        call(Method::POST, revoke.clone(), None)
            .await
            .status()
            .as_u16(),
        204
    );
    assert_eq!(
        call(Method::POST, revoke, None).await.status().as_u16(),
        204
    );
    let reapprove = call(
        Method::POST,
        format!("/v1/admin/devices/{}/approve", pending.id),
        None,
    )
    .await;
    assert_eq!(reapprove.status().as_u16(), 409);
    let unknown = call(Method::POST, "/v1/admin/devices/nope/revoke".into(), None).await;
    assert_eq!(unknown.status().as_u16(), 404);
    let events: Vec<String> = api
        .state
        .store
        .events_since(0, 1000)
        .unwrap()
        .into_iter()
        .map(|e| e.event)
        .collect();
    for expected in [
        "member_added",
        "member_updated",
        "device_approved",
        "device_revoked",
    ] {
        assert!(events.iter().any(|e| e == expected), "{expected}");
    }
}

#[tokio::test]
async fn the_audit_export_is_the_mirror_as_json_lines() {
    let (api, _ada, access) = with_admin().await;
    let store = &api.state.store;
    store
        .mirror_audit(
            "lb1",
            &[
                record("w1", 1, 1_000, "agent-a"),
                record("w1", 2, 2_000, "agent-b"),
            ],
        )
        .unwrap();
    store
        .mirror_audit("lb2", &[record("w2", 1, 3_000, "agent-a")])
        .unwrap();

    let export = |query: &'static str| {
        let access = access.clone();
        let api = &api;
        async move {
            let reply = api
                .call(
                    Method::GET,
                    &format!("/v1/admin/audit/export{query}"),
                    Some(&access),
                    None,
                )
                .await;
            assert_eq!(reply.status().as_u16(), 200);
            assert_eq!(reply.headers()["content-type"], "application/x-ndjson");
            reply
                .text()
                .await
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .collect::<Vec<_>>()
        }
    };

    let all = export("").await;
    assert_eq!(all.len(), 3);
    let first = &all[0];
    assert_eq!(first["lockboxId"], "lb1");
    assert_eq!(first["writerId"], "w1");
    assert_eq!(first["seq"], 1);
    assert_eq!(first["tsMs"], 1_000);
    assert_eq!(first["prevHash"], hex::encode([1u8; 32]));
    assert_eq!(first["hash"], hex::encode([2u8; 32]));
    assert_eq!(first["actorId"], "agent-a");
    assert_eq!(first["target"], "api.stripe.com/v1/charges");
    assert_eq!(first["event"], "used");
    assert_eq!(first["detail"], Value::Null);

    assert_eq!(export("?lockbox=lb2").await.len(), 1);
    assert_eq!(export("?actor=agent-a").await.len(), 2);
    let window = export("?since=2000&until=3000").await;
    assert_eq!(window.len(), 1);
    assert_eq!(window[0]["seq"], 2);

    let malformed = api
        .call(
            Method::GET,
            "/v1/admin/audit/export?since=yesterday",
            Some(&access),
            None,
        )
        .await;
    assert_eq!(malformed.status().as_u16(), 400);
}
