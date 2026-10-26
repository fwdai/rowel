use std::collections::BTreeMap;

use rowel_broker::actions::{Client, ClientKind};
use rowel_broker::grant::{Approval, CreatedBy, ItemSelector, Mode};
use rowel_broker::policy::{AgentSelector, Rule};
use rowel_core::store::AuditRecord;

use super::*;
use crate::testing::TempServer;

fn rule(id: &str) -> Rule {
    Rule {
        id: id.into(),
        agents: AgentSelector::Any,
        items: ItemSelector::Lockbox,
        targets: vec![],
        modes: vec![Mode::Env],
        approval: Approval::Ask,
        ttl_ms: 1_000,
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
        target: Some("stripe balance retrieve".into()),
        mode: Some("env".into()),
        approval: Some("ask".into()),
        approver: None,
        event: "used".into(),
        detail: None,
    }
}

#[test]
fn the_tenant_is_minted_once_and_renamed_by_the_config() {
    let server = TempServer::new();
    let tenant = server.store.tenant().unwrap();
    assert_eq!(tenant.name, crate::testing::TENANT);
    let path = server.data_dir().join(DB_FILE);
    drop(server.store);

    let reopened = Store::open(&path, "Acme Corp").unwrap();
    let again = reopened.tenant().unwrap();
    assert_eq!(again.id, tenant.id);
    assert_eq!(again.name, "Acme Corp");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn a_database_from_a_newer_build_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(DB_FILE);
    drop(Store::open(&path, "Acme").unwrap());
    rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", 99)
        .unwrap();
    assert!(matches!(
        Store::open(&path, "Acme"),
        Err(StoreError::SchemaNewer)
    ));
}

#[test]
fn members_round_trip_by_id_and_by_email_in_any_case() {
    let server = TempServer::new();
    let store = &server.store;
    let pat = store
        .add_member("Pat@Acme.example", Some("Pat"), Role::Member)
        .unwrap();
    assert_eq!(store.member(&pat.id).unwrap(), Some(pat.clone()));
    assert_eq!(
        store.member_by_email("pat@acme.example").unwrap(),
        Some(pat.clone())
    );
    // One address, one member, however it is cased.
    assert!(store
        .add_member("PAT@ACME.EXAMPLE", None, Role::Member)
        .is_err());

    store.set_member_role(&pat.id, Role::Admin).unwrap();
    store.disable_member(&pat.id).unwrap();
    let pat = store.member(&pat.id).unwrap().unwrap();
    assert_eq!(pat.role, Role::Admin);
    assert!(pat.disabled_at_ms.is_some());
    assert!(matches!(
        store.disable_member(&pat.id),
        Err(StoreError::NotFound(_))
    ));
    assert_eq!(store.list_members().unwrap(), vec![pat]);
    assert_eq!(store.member("nobody").unwrap(), None);
}

#[test]
fn a_device_is_pending_until_approved_and_the_same_key_is_the_same_device() {
    let server = TempServer::new();
    let store = &server.store;
    let pat = server.member("pat@acme.example", Role::Member);
    let device = store.enroll_device(&pat.id, "MacBook", "key-a").unwrap();
    assert_eq!(device.state, DeviceState::Pending);
    assert_eq!(
        store
            .enroll_device(&pat.id, "MacBook again", "key-a")
            .unwrap(),
        device
    );
    let other = store.enroll_device(&pat.id, "Phone", "key-b").unwrap();
    assert_eq!(
        store.devices_of(&pat.id).unwrap(),
        vec![device.clone(), other.clone()]
    );

    let approved = store.approve_device(&device.id, &other.id).unwrap();
    assert_eq!(approved.state, DeviceState::Approved);
    assert_eq!(approved.approved_by.as_deref(), Some(other.id.as_str()));
    assert!(approved.approved_at_ms.is_some());
    assert_eq!(store.device(&device.id).unwrap(), Some(approved.clone()));
    // Approving again changes nothing.
    assert_eq!(store.approve_device(&device.id, "admin").unwrap(), approved);

    store.revoke_device(&device.id).unwrap();
    assert_eq!(
        store.device(&device.id).unwrap().unwrap().state,
        DeviceState::Revoked
    );
    assert!(store.approve_device(&device.id, "admin").is_err());
    assert!(matches!(
        store.approve_device("nobody", "admin"),
        Err(StoreError::NotFound(_))
    ));
}

#[test]
fn a_revoked_device_loses_the_masters_sealed_to_it() {
    let server = TempServer::new();
    let store = &server.store;
    let pat = server.member("pat@acme.example", Role::Member);
    let (device, _) = server.approved_device(&pat);
    let lockbox = server.lockbox("Deploy");
    store
        .put_device_master(&lockbox.id, &device.id, 1, "sealed")
        .unwrap();
    let (_, access, _) = store
        .create_session(&pat.id, Some(&device.id), 60_000)
        .unwrap();
    store.revoke_device(&device.id).unwrap();
    assert_eq!(store.device_master(&lockbox.id, &device.id).unwrap(), None);
    assert!(
        store.session_by_access(&access).unwrap().is_none(),
        "a revoked device's session ends with it"
    );
}

#[test]
fn a_session_is_found_by_its_access_token_while_it_lives() {
    let server = TempServer::new();
    let store = &server.store;
    let pat = server.member("pat@acme.example", Role::Member);
    let (session, access, _refresh) = store
        .create_session_at(&pat.id, None, 1_000, 10_000)
        .unwrap();
    assert_eq!(session.member_id, pat.id);
    assert_eq!(session.device_id, None);
    assert_eq!(session.access_expires_at_ms, 11_000);
    assert_eq!(
        store.session_by_access_at(&access, 10_500).unwrap(),
        Some(session.clone())
    );
    assert_eq!(store.session_by_access_at(&access, 11_000).unwrap(), None);
    assert_eq!(store.session_by_access_at("other", 10_500).unwrap(), None);

    let (device, _) = server.approved_device(&pat);
    store.bind_session_device(&session.id, &device.id).unwrap();
    assert_eq!(
        store
            .session_by_access_at(&access, 10_500)
            .unwrap()
            .unwrap()
            .device_id
            .as_deref(),
        Some(device.id.as_str())
    );

    assert_eq!(store.revoke_sessions_of_member(&pat.id).unwrap(), 1);
    assert_eq!(store.session_by_access_at(&access, 10_500).unwrap(), None);
}

#[test]
fn a_refresh_token_is_single_use_and_its_reuse_ends_the_session() {
    let server = TempServer::new();
    let store = &server.store;
    let pat = server.member("pat@acme.example", Role::Member);
    let (device, _) = server.approved_device(&pat);
    let (session, access, refresh) = store
        .create_session_at(&pat.id, Some(&device.id), 1_000, 10_000)
        .unwrap();

    let (refreshed, access2, refresh2) = store
        .refresh_session_at(&refresh, 1_000, 12_000)
        .unwrap()
        .unwrap();
    // The same session, bound to the same device, with a new deadline.
    assert_eq!(refreshed.id, session.id);
    assert_eq!(refreshed.device_id.as_deref(), Some(device.id.as_str()));
    assert_eq!(refreshed.access_expires_at_ms, 13_000);
    assert_ne!(*access2, *access);
    assert_ne!(*refresh2, *refresh);
    // The old access token is gone with the refresh, the new one works.
    assert_eq!(store.session_by_access_at(&access, 12_500).unwrap(), None);
    assert_eq!(
        store.session_by_access_at(&access2, 12_500).unwrap(),
        Some(refreshed)
    );

    // The spent refresh token presented again: nothing, and the session —
    // the new tokens included — is revoked.
    assert_eq!(
        store.refresh_session_at(&refresh, 1_000, 12_600).unwrap(),
        None
    );
    assert_eq!(store.session_by_access_at(&access2, 12_700).unwrap(), None);
    assert_eq!(
        store.refresh_session_at(&refresh2, 1_000, 12_700).unwrap(),
        None
    );
    // A token the server never issued is simply unknown.
    assert_eq!(
        store.refresh_session_at("stranger", 1_000, 12_700).unwrap(),
        None
    );
}

#[test]
fn an_authorization_code_is_redeemed_once_with_its_verifier_within_its_time() {
    let server = TempServer::new();
    let store = &server.store;
    let pat = server.member("pat@acme.example", Role::Member);
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let challenge = sessions::pkce_challenge(verifier);
    let redirect = "http://127.0.0.1:4242/callback";

    let code = store
        .issue_code_at(&pat.id, &challenge, redirect, 10_000)
        .unwrap();
    // The wrong verifier burns the code.
    assert_eq!(
        store
            .redeem_code_at(&code, "not-the-verifier", redirect, 10_100)
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .redeem_code_at(&code, verifier, redirect, 10_100)
            .unwrap(),
        None
    );

    let code = store
        .issue_code_at(&pat.id, &challenge, redirect, 10_000)
        .unwrap();
    assert_eq!(
        store
            .redeem_code_at(&code, verifier, redirect, 10_100)
            .unwrap(),
        Some(pat.id.clone())
    );
    assert_eq!(
        store
            .redeem_code_at(&code, verifier, redirect, 10_100)
            .unwrap(),
        None
    );

    // Another redirect, or the deadline passed, is a no.
    let code = store
        .issue_code_at(&pat.id, &challenge, redirect, 10_000)
        .unwrap();
    assert_eq!(
        store
            .redeem_code_at(&code, verifier, "http://127.0.0.1:1/other", 10_100)
            .unwrap(),
        None
    );
    let code = store
        .issue_code_at(&pat.id, &challenge, redirect, 10_000)
        .unwrap();
    assert_eq!(
        store
            .redeem_code_at(&code, verifier, redirect, 10_000 + AUTH_CODE_TTL_MS)
            .unwrap(),
        None
    );
}

#[test]
fn lockbox_rows_hold_their_switches_members_and_revision() {
    let server = TempServer::new();
    let store = &server.store;
    let pat = server.member("pat@acme.example", Role::Member);
    let sam = server.member("sam@acme.example", Role::Member);
    let deploy = server.lockbox("Deploy");
    let billing = server.lockbox("Billing");
    assert_eq!(
        store.list_lockboxes().unwrap(),
        vec![deploy.clone(), billing.clone()]
    );

    store.set_frozen(&deploy.id, true).unwrap();
    store.set_agents(&deploy.id, true).unwrap();
    let frozen = store.lockbox(&deploy.id).unwrap().unwrap();
    assert!(frozen.frozen && frozen.agents);
    assert_eq!(store.bump_revision(&deploy.id).unwrap(), 1);
    assert_eq!(store.bump_revision(&deploy.id).unwrap(), 2);
    assert_eq!(store.revision(&deploy.id).unwrap(), Some(2));
    assert_eq!(store.revision("nothing").unwrap(), None);
    assert!(matches!(
        store.bump_revision("nothing"),
        Err(StoreError::NotFound(_))
    ));

    store
        .add_lockbox_member(&deploy.id, &pat.id, LockboxRole::Reader)
        .unwrap();
    store
        .add_lockbox_member(&deploy.id, &sam.id, LockboxRole::Approver)
        .unwrap();
    store
        .add_lockbox_member(&billing.id, &pat.id, LockboxRole::Admin)
        .unwrap();
    assert_eq!(
        store.lockboxes_of_member(&pat.id).unwrap(),
        vec![store.lockbox(&deploy.id).unwrap().unwrap(), billing.clone()]
    );
    assert_eq!(
        store.approvers_of(&deploy.id).unwrap(),
        vec![sam.id.clone()]
    );
    assert_eq!(
        store.approvers_of(&billing.id).unwrap(),
        vec![pat.id.clone()]
    );
    // A role given again replaces the one held.
    store
        .add_lockbox_member(&deploy.id, &pat.id, LockboxRole::Admin)
        .unwrap();
    let mut members = store.lockbox_members(&deploy.id).unwrap();
    members.sort_by(|a, b| a.member_id.cmp(&b.member_id));
    let mut expected = vec![
        LockboxMember {
            member_id: pat.id.clone(),
            role: LockboxRole::Admin,
        },
        LockboxMember {
            member_id: sam.id.clone(),
            role: LockboxRole::Approver,
        },
    ];
    expected.sort_by(|a, b| a.member_id.cmp(&b.member_id));
    assert_eq!(members, expected);
    store.remove_lockbox_member(&deploy.id, &pat.id).unwrap();
    assert_eq!(store.lockboxes_of_member(&pat.id).unwrap(), vec![billing]);
}

#[test]
fn device_masters_are_kept_per_lockbox_and_device_at_a_generation() {
    let server = TempServer::new();
    let store = &server.store;
    let pat = server.member("pat@acme.example", Role::Member);
    let (device, _) = server.approved_device(&pat);
    let lockbox = server.lockbox("Deploy");
    assert_eq!(store.device_master(&lockbox.id, &device.id).unwrap(), None);
    store
        .put_device_master(&lockbox.id, &device.id, 1, "one")
        .unwrap();
    store
        .put_device_master(&lockbox.id, &device.id, 2, "two")
        .unwrap();
    assert_eq!(
        store.device_master(&lockbox.id, &device.id).unwrap(),
        Some((2, "two".into()))
    );
    assert_eq!(
        store.delete_device_masters_of_device(&device.id).unwrap(),
        1
    );
    assert_eq!(store.device_master(&lockbox.id, &device.id).unwrap(), None);
}

#[test]
fn agents_their_trust_policies_and_clients_round_trip() {
    let server = TempServer::new();
    let store = &server.store;
    let lockbox = server.lockbox("Deploy");
    let agent = store.add_agent("release-bot", &lockbox.id).unwrap();
    assert!(agent.enabled);
    assert_eq!(store.agent(&agent.id).unwrap(), Some(agent.clone()));
    assert_eq!(store.list_agents().unwrap(), vec![agent.clone()]);
    store.rename_agent(&agent.id, "release").unwrap();
    store.set_agent_enabled(&agent.id, false).unwrap();
    let agent = store.agent(&agent.id).unwrap().unwrap();
    assert_eq!((agent.name.as_str(), agent.enabled), ("release", false));

    let claims: BTreeMap<String, String> =
        [("repository", "acme/deploy"), ("ref", "refs/heads/main")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
    let policy = store
        .add_trust_policy(
            &agent.id,
            "https://token.actions.githubusercontent.com",
            claims.clone(),
            false,
        )
        .unwrap();
    assert_eq!(policy.claims, claims);
    assert_eq!(
        store.trust_policies_of(&agent.id).unwrap(),
        vec![policy.clone()]
    );
    assert_eq!(
        store
            .trust_policies_for_issuer("https://token.actions.githubusercontent.com")
            .unwrap(),
        vec![policy.clone()]
    );
    assert_eq!(
        store
            .trust_policies_for_issuer("https://elsewhere")
            .unwrap(),
        vec![]
    );

    let client = Client {
        name: "release-bot".into(),
        key: "key-1".into(),
        kind: ClientKind::Agent,
        platform: Some("cli".into()),
        created_at_ms: 1_000,
        last_seen_at_ms: None,
        agent_id: Some(agent.id.clone()),
        driven_by_agent: false,
    };
    store.put_client(&client).unwrap();
    assert_eq!(store.client("key-1").unwrap(), Some(client.clone()));
    assert_eq!(store.clients().unwrap(), vec![client.clone()]);
    store.client_seen("key-1").unwrap();
    assert!(store
        .client("key-1")
        .unwrap()
        .unwrap()
        .last_seen_at_ms
        .is_some());
    // A client with no definition has no place here.
    assert!(store
        .put_client(&Client {
            agent_id: None,
            key: "key-2".into(),
            ..client.clone()
        })
        .is_err());

    // Pruning goes by the last sighting, or by creation for one never seen.
    store
        .put_client(&Client {
            key: "key-old".into(),
            created_at_ms: 10,
            ..client.clone()
        })
        .unwrap();
    assert_eq!(store.prune_clients(500).unwrap(), 1);
    assert_eq!(store.client("key-old").unwrap(), None);
    assert!(store.client("key-1").unwrap().is_some());

    store.remove_trust_policy(&policy.id).unwrap();
    assert_eq!(store.trust_policies_of(&agent.id).unwrap(), vec![]);
    // Removing the definition takes its clients with it.
    store.remove_agent(&agent.id).unwrap();
    assert_eq!(store.clients().unwrap(), vec![]);
    assert_eq!(store.list_agents().unwrap(), vec![]);
}

#[test]
fn disabling_an_agents_clients_by_hand_still_works() {
    let server = TempServer::new();
    let store = &server.store;
    let lockbox = server.lockbox("Deploy");
    let agent = store.add_agent("release-bot", &lockbox.id).unwrap();
    store
        .put_client(&Client {
            name: "release-bot".into(),
            key: "key-1".into(),
            kind: ClientKind::Agent,
            platform: None,
            created_at_ms: 0,
            last_seen_at_ms: None,
            agent_id: Some(agent.id.clone()),
            driven_by_agent: false,
        })
        .unwrap();
    assert_eq!(store.delete_clients_of_agent(&agent.id).unwrap(), 1);
    assert_eq!(store.clients().unwrap(), vec![]);
}

#[test]
fn rules_are_the_lockboxes_own_plus_the_tenants_and_are_the_servers() {
    let server = TempServer::new();
    let store = &server.store;
    let deploy = server.lockbox("Deploy");
    let billing = server.lockbox("Billing");
    store.add_rule(None, &rule("tenant-wide")).unwrap();
    store
        .add_rule(Some(&deploy.id), &rule("deploy-only"))
        .unwrap();
    store
        .add_rule(Some(&billing.id), &rule("billing-only"))
        .unwrap();

    let for_deploy = store.rules_for(&deploy.id).unwrap();
    assert_eq!(
        for_deploy.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["tenant-wide", "deploy-only"]
    );
    for rule in &for_deploy {
        assert_eq!(rule.created_by, CreatedBy::Admin(SERVER_ADMIN.into()));
    }
    let listed = store.list_rules().unwrap();
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0].0, None);
    assert_eq!(listed[1].0.as_deref(), Some(deploy.id.as_str()));

    store.remove_rule("deploy-only").unwrap();
    assert_eq!(store.rules_for(&deploy.id).unwrap().len(), 1);
    assert!(matches!(
        store.remove_rule("deploy-only"),
        Err(StoreError::NotFound(_))
    ));
}

#[test]
fn a_pending_approval_is_settled_once() {
    let server = TempServer::new();
    let store = &server.store;
    let deploy = server.lockbox("Deploy");
    let billing = server.lockbox("Billing");
    let ask = store
        .record_pending("ap1", &deploy.id, None, "key-1", r#"{"id":"ap1"}"#, 99_000)
        .unwrap();
    store
        .record_pending("ap2", &billing.id, None, "key-1", r#"{"id":"ap2"}"#, 99_000)
        .unwrap();
    assert_eq!(ask.state, PendingState::Pending);
    assert_eq!(
        store
            .list_pending(std::slice::from_ref(&deploy.id))
            .unwrap(),
        vec![ask.clone()]
    );
    assert_eq!(
        store
            .list_pending(&[deploy.id.clone(), billing.id.clone()])
            .unwrap()
            .len(),
        2
    );

    assert!(store
        .settle_pending("ap1", PendingState::Answered, Some("d1"))
        .unwrap());
    // The second answer is told it was late.
    assert!(!store
        .settle_pending("ap1", PendingState::Expired, None)
        .unwrap());
    assert_eq!(
        store
            .list_pending(std::slice::from_ref(&deploy.id))
            .unwrap(),
        vec![]
    );
    assert!(store
        .settle_pending("ap2", PendingState::Expired, None)
        .unwrap());
}

#[test]
fn the_audit_mirror_is_a_union_exported_in_chain_order() {
    let server = TempServer::new();
    let store = &server.store;
    let rows = vec![
        record("w1", 1, 100, "agent-a"),
        record("w1", 2, 200, "agent-b"),
        record("w2", 1, 150, "agent-a"),
    ];
    assert_eq!(store.mirror_audit("lb1", &rows).unwrap(), 3);
    // Mirroring again, with one new row, inserts only the new row.
    let mut again = rows.clone();
    again.push(record("w1", 3, 300, "agent-a"));
    assert_eq!(store.mirror_audit("lb1", &again).unwrap(), 1);
    assert_eq!(store.mirror_audit("lb2", &rows[..1]).unwrap(), 1);

    let all = store.audit_export(&AuditFilter::default()).unwrap();
    let order: Vec<(String, String, i64)> = all
        .iter()
        .map(|(lb, r)| (lb.clone(), r.writer_id.clone(), r.seq))
        .collect();
    assert_eq!(
        order,
        vec![
            ("lb1".into(), "w1".into(), 1),
            ("lb1".into(), "w1".into(), 2),
            ("lb1".into(), "w1".into(), 3),
            ("lb1".into(), "w2".into(), 1),
            ("lb2".into(), "w1".into(), 1),
        ]
    );
    assert_eq!(all[0].1, rows[0]);

    let filtered = store
        .audit_export(&AuditFilter {
            since_ms: Some(150),
            until_ms: Some(300),
            lockbox_id: Some("lb1".into()),
            actor_id: Some("agent-a".into()),
        })
        .unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(
        (filtered[0].1.writer_id.as_str(), filtered[0].1.seq),
        ("w2", 1)
    );

    assert_eq!(store.audit_high_water("lb1", "w1").unwrap(), 3);
    assert_eq!(store.audit_high_water("lb1", "w2").unwrap(), 1);
    assert_eq!(store.audit_high_water("lb1", "w9").unwrap(), 0);
}

#[test]
fn a_jti_is_noted_once_until_it_expires() {
    let server = TempServer::new();
    let store = &server.store;
    assert!(store.note_jti_at("j1", 2_000, 1_000).unwrap());
    assert!(!store.note_jti_at("j1", 2_000, 1_500).unwrap());
    // Past its time the same `jti` is a new token's as far as replay goes.
    assert!(store.note_jti_at("j1", 3_000, 2_000).unwrap());
    assert!(store.note_jti_at("j2", 2_500, 2_000).unwrap());
    assert_eq!(store.prune_replay(2_500).unwrap(), 1);
    assert_eq!(store.prune_replay(2_500).unwrap(), 0);
    assert!(!store.note_jti_at("j1", 3_000, 2_600).unwrap());
}
