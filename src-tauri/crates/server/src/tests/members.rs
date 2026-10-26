//! The member API as the app speaks it: the sign-in through the company's
//! identity provider, the token endpoint's rules, enrollment, approval from
//! another device and the listing that seals a master to an approved one.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use crypto_box::SecretKey;
use reqwest::Method;
use serde_json::json;

use super::harness::{
    challenge, code_of, json, param, public_key_of, tokens_of, Api, APP_STATE, REDIRECT, VERIFIER,
};
use crate::rootkey::RootKey;
use crate::store::{DeviceState, LockboxRole, Role, ENROLL_CHALLENGE_TTL_MS};

#[tokio::test]
async fn a_member_signs_in_through_the_idp_and_the_bearer_reaches_me() {
    let api = Api::start().await;
    api.idp.set_identity("pat@acme.example", "Pat", true);

    let back = api.sign_in_redirect(&challenge(VERIFIER)).await;
    assert_eq!(
        format!("{}://{}{}", back.scheme(), back.authority(), back.path()),
        REDIRECT
    );
    let code = code_of(&back);
    let reply = api.redeem(&code, VERIFIER).await;
    assert_eq!(reply.status().as_u16(), 200);
    assert_eq!(reply.headers()["cache-control"], "no-store");
    let body = json(reply).await;
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 3600);
    let access = body["access_token"].as_str().unwrap();
    assert!(body["refresh_token"].is_string());

    let me = api
        .call(Method::GET, "/v1/members/me", Some(access), None)
        .await;
    assert_eq!(me.status().as_u16(), 200);
    let me = json(me).await;
    assert_eq!(me["member"]["email"], "pat@acme.example");
    assert_eq!(me["member"]["name"], "Pat");
    assert_eq!(me["tenant"]["name"], "Acme");

    // Provisioned at the first sign-in, as a plain member, and written down.
    let member = api
        .state
        .store
        .member_by_email("pat@acme.example")
        .unwrap()
        .unwrap();
    assert_eq!(member.role, Role::Member);
    assert_eq!(me["member"]["id"], member.id);
    let events = api.state.store.events_since(0, 10).unwrap();
    assert!(events
        .iter()
        .any(|e| e.event == "signed_in" && e.member_id.as_deref() == Some(&member.id)));

    // The code was single use.
    let again = api.redeem(&code, VERIFIER).await;
    assert_eq!(again.status().as_u16(), 400);
    assert_eq!(json(again).await["error"], "invalid_grant");
}

#[tokio::test]
async fn every_member_call_wants_a_live_bearer() {
    let api = Api::start().await;
    for access in [None, Some("not-a-token")] {
        let reply = api.call(Method::GET, "/v1/members/me", access, None).await;
        assert_eq!(reply.status().as_u16(), 401);
        let reply = api
            .call(Method::GET, "/v1/members/me/lockboxes", access, None)
            .await;
        assert_eq!(reply.status().as_u16(), 401);
    }
}

#[tokio::test]
async fn authorize_refuses_what_the_app_never_sends() {
    let api = Api::start().await;
    let challenge = challenge(VERIFIER);
    for overrides in [
        &[("client_id", "someone-else")][..],
        &[("response_type", "token")],
        &[("code_challenge_method", "plain")],
        &[("code_challenge", "short")],
        &[("redirect_uri", "https://evil.example/cb")],
        &[("redirect_uri", "http://127.0.0.1/no-port")],
        &[("redirect_uri", "http://10.0.0.1:4242/cb")],
        &[("state", "")],
    ] {
        let reply = api.get(&api.authorize_url(&challenge, overrides)).await;
        assert_eq!(reply.status().as_u16(), 400, "{overrides:?}");
        assert!(!reply.text().await.unwrap().is_empty());
    }
    let localhost = api
        .get(&api.authorize_url(&challenge, &[("redirect_uri", "http://localhost:4242/cb")]))
        .await;
    assert_eq!(localhost.status().as_u16(), 302);

    // A callback for a login the server never started.
    let stray = api
        .get(&format!(
            "{}/v1/members/auth/callback?state=made-up&code=x",
            api.url
        ))
        .await;
    assert_eq!(stray.status().as_u16(), 400);

    let unconfigured = Api::with(None).await;
    let reply = unconfigured
        .get(&unconfigured.authorize_url(&challenge, &[]))
        .await;
    assert_eq!(reply.status().as_u16(), 503);
    assert_eq!(
        reply.text().await.unwrap(),
        "member sign-in is not configured on this server"
    );
}

#[tokio::test]
async fn a_wrong_verifier_is_invalid_grant_and_burns_the_code() {
    let api = Api::start().await;
    let back = api.sign_in_redirect(&challenge(VERIFIER)).await;
    let code = code_of(&back);

    let wrong = api
        .redeem(&code, "another-verifier-of-at-least-forty-three-chars")
        .await;
    assert_eq!(wrong.status().as_u16(), 400);
    assert_eq!(json(wrong).await["error"], "invalid_grant");
    let right_too_late = api.redeem(&code, VERIFIER).await;
    assert_eq!(right_too_late.status().as_u16(), 400);

    for (form, error) in [
        (
            &[
                ("grant_type", "authorization_code"),
                ("client_id", "rowel-app"),
            ][..],
            "invalid_request",
        ),
        (
            &[("grant_type", "password"), ("client_id", "rowel-app")],
            "unsupported_grant_type",
        ),
        (
            &[("grant_type", "refresh_token"), ("client_id", "other")],
            "invalid_request",
        ),
    ] {
        let reply = api.token(form).await;
        assert_eq!(reply.status().as_u16(), 400);
        assert_eq!(json(reply).await["error"], error);
    }
}

#[tokio::test]
async fn a_refresh_rotates_and_a_replayed_refresh_ends_the_session() {
    let api = Api::start().await;
    let first = api.sign_in("pat@acme.example").await;

    let rotated = api.refresh(&first.refresh).await;
    assert_eq!(rotated.status().as_u16(), 200);
    let second = tokens_of(rotated).await;
    assert_ne!(second.refresh, first.refresh);
    let me = api
        .call(Method::GET, "/v1/members/me", Some(&second.access), None)
        .await;
    assert_eq!(me.status().as_u16(), 200);

    // The spent token again: a 401, and the session it belonged to is over.
    let replayed = api.refresh(&first.refresh).await;
    assert_eq!(replayed.status().as_u16(), 401);
    assert_eq!(json(replayed).await["error"], "invalid_grant");
    let me = api
        .call(Method::GET, "/v1/members/me", Some(&second.access), None)
        .await;
    assert_eq!(me.status().as_u16(), 401);
    assert_eq!(api.refresh(&second.refresh).await.status().as_u16(), 401);
}

#[tokio::test]
async fn a_refused_sign_in_goes_back_to_the_app_with_an_error_and_no_code() {
    let api = Api::start().await;
    let refused = |back: &url::Url| {
        assert_eq!(param(back, "state").as_deref(), Some(APP_STATE));
        assert_eq!(param(back, "error").as_deref(), Some("access_denied"));
        assert_eq!(param(back, "code"), None);
    };

    api.idp.set_identity("eve@elsewhere.example", "Eve", true);
    refused(&api.sign_in_redirect(&challenge(VERIFIER)).await);
    assert_eq!(
        api.state
            .store
            .member_by_email("eve@elsewhere.example")
            .unwrap(),
        None
    );

    let sam = api.server.member("sam@acme.example", Role::Member);
    api.state.store.disable_member(&sam.id).unwrap();
    api.idp.set_identity("sam@acme.example", "Sam", true);
    refused(&api.sign_in_redirect(&challenge(VERIFIER)).await);

    // With no domains named, only a member an admin added gets in.
    let open = Api::with(Some(vec![])).await;
    open.idp.set_identity("pat@acme.example", "Pat", true);
    refused(&open.sign_in_redirect(&challenge(VERIFIER)).await);
    open.server.member("pat@acme.example", Role::Member);
    code_of(&open.sign_in_redirect(&challenge(VERIFIER)).await);
}

#[tokio::test]
async fn a_pending_device_gets_no_master_until_an_approved_device_approves_it() {
    let api = Api::start().await;
    let pat = api.server.member("pat@acme.example", Role::Member);
    let lockbox = api.server.lockbox("Deploy");
    api.server
        .store
        .add_lockbox_member(&lockbox.id, &pat.id, LockboxRole::Reader)
        .unwrap();

    // The new device enrolls and comes back pending.
    let new = api.sign_in("pat@acme.example").await;
    let new_secret = SecretKey::generate(&mut rand::thread_rng());
    let new_public = public_key_of(&new_secret);
    let enrolled = api.enroll(&new.access, &new_secret).await;
    assert_eq!(enrolled.status().as_u16(), 200);
    let enrolled = json(enrolled).await;
    assert_eq!(enrolled["state"], "pending");
    let new_id = enrolled["deviceId"].as_str().unwrap().to_string();

    let missing = api
        .call(
            Method::POST,
            "/v1/members/enroll",
            Some(&new.access),
            Some(json!({ "deviceName": "Pat's laptop" })),
        )
        .await;
    assert_eq!(missing.status().as_u16(), 400);

    let devices = json(
        api.call(
            Method::GET,
            "/v1/members/me/devices",
            Some(&new.access),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(devices.as_array().unwrap().len(), 1);
    assert_eq!(devices[0]["id"], new_id);
    assert_eq!(devices[0]["publicKey"], new_public);
    assert_eq!(devices[0]["state"], "pending");
    assert!(devices[0]["createdAtMs"].is_i64());

    let listing = json(
        api.call(
            Method::GET,
            "/v1/members/me/lockboxes",
            Some(&new.access),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(listing[0]["id"], lockbox.id);
    assert_eq!(listing[0]["kdf"], lockbox.kdf);
    assert!(listing[0].get("wrappedMaster").is_none());

    // A pending device approves nothing, itself included.
    let approve_path = format!("/v1/members/devices/{new_id}/approve");
    let from_pending = api
        .call(Method::POST, &approve_path, Some(&new.access), None)
        .await;
    assert_eq!(from_pending.status().as_u16(), 403);

    // The member's other device, already approved, signs in as itself (the
    // same key enrolled again is the same device) and approves the new one.
    let (old_device, old_secret) = api.server.approved_device(&pat);
    let old = api.sign_in("pat@acme.example").await;
    let again = json(api.enroll(&old.access, &old_secret).await).await;
    assert_eq!(again["deviceId"], old_device.id);
    assert_eq!(again["state"], "approved");

    let unknown = api
        .call(
            Method::POST,
            "/v1/members/devices/nope/approve",
            Some(&old.access),
            None,
        )
        .await;
    assert_eq!(unknown.status().as_u16(), 404);
    let sam = api.server.member("sam@acme.example", Role::Member);
    let (sams, _) = api.server.pending_device(&sam);
    let someone_elses = api
        .call(
            Method::POST,
            &format!("/v1/members/devices/{}/approve", sams.id),
            Some(&old.access),
            None,
        )
        .await;
    assert_eq!(someone_elses.status().as_u16(), 404);

    let approved = api
        .call(Method::POST, &approve_path, Some(&old.access), None)
        .await;
    assert_eq!(approved.status().as_u16(), 204);
    let row = api.state.store.device(&new_id).unwrap().unwrap();
    assert_eq!(row.approved_by.as_deref(), Some(old_device.id.as_str()));

    // Now the new device's listing carries the master, sealed to its key.
    let listing = json(
        api.call(
            Method::GET,
            "/v1/members/me/lockboxes",
            Some(&new.access),
            None,
        )
        .await,
    )
    .await;
    let sealed = STANDARD
        .decode(listing[0]["wrappedMaster"].as_str().unwrap())
        .unwrap();
    let opened = new_secret.unseal(&sealed).unwrap();
    let store = &api.state.store;
    let wrapped = store.wrapped_master(&lockbox.id).unwrap().unwrap();
    let master = api
        .server
        .root
        .unwrap(&store.tenant().unwrap().id, &lockbox.id, &wrapped)
        .unwrap();
    assert_eq!(opened, *master);

    let events: Vec<_> = store
        .events_since(0, 100)
        .unwrap()
        .into_iter()
        .map(|e| (e.event, e.device_id))
        .collect();
    assert!(events.contains(&("device_enrolled".into(), Some(new_id.clone()))));
    assert!(events.contains(&("device_approved".into(), Some(new_id))));
}

/// The attack the proof is for: an install of the member's that nobody
/// approved reads an approved device's public key off the list and enrolls
/// under it, to be handed that device's masters and approve itself.
#[tokio::test]
async fn naming_an_approved_devices_key_without_its_secret_binds_nothing() {
    let api = Api::start().await;
    let pat = api.server.member("pat@acme.example", Role::Member);
    let lockbox = api.server.lockbox("Deploy");
    api.server
        .store
        .add_lockbox_member(&lockbox.id, &pat.id, LockboxRole::Reader)
        .unwrap();
    let (approved, _) = api.server.approved_device(&pat);

    let intruder = api.sign_in("pat@acme.example").await;
    let own = SecretKey::generate(&mut rand::thread_rng());
    let enrolled = json(api.enroll(&intruder.access, &own).await).await;
    assert_eq!(enrolled["state"], "pending");
    let own_id = enrolled["deviceId"].as_str().unwrap().to_string();

    let devices = json(
        api.call(
            Method::GET,
            "/v1/members/me/devices",
            Some(&intruder.access),
            None,
        )
        .await,
    )
    .await;
    let listed = devices
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["state"] == "approved")
        .unwrap()["publicKey"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(listed, approved.public_key);

    // A challenge for the listed key is handed out, but it opens only under
    // the approved device's secret. Whatever the intruder sends instead —
    // nothing, the proof for its own key, bytes of its own making — is no
    // answer.
    let own_proof = api.open_challenge(
        &own,
        &api.enroll_challenge(&intruder.access, &public_key_of(&own))
            .await,
    );
    let zeros = STANDARD.encode([0u8; 32]);
    for proof in [None, Some(own_proof.as_str()), Some(zeros.as_str())] {
        api.enroll_challenge(&intruder.access, &listed).await;
        let refused = api.enroll_with(&intruder.access, &listed, proof).await;
        assert_eq!(refused.status().as_u16(), 403, "{proof:?}");
    }

    // Nothing moved: the session is still its own pending device's, the
    // listing carries no master, and that device approves nothing.
    let session = api
        .state
        .store
        .session_by_access(&intruder.access)
        .unwrap()
        .unwrap();
    assert_eq!(session.device_id.as_deref(), Some(own_id.as_str()));
    let listing = json(
        api.call(
            Method::GET,
            "/v1/members/me/lockboxes",
            Some(&intruder.access),
            None,
        )
        .await,
    )
    .await;
    assert!(listing[0].get("wrappedMaster").is_none());
    let approve = api
        .call(
            Method::POST,
            &format!("/v1/members/devices/{own_id}/approve"),
            Some(&intruder.access),
            None,
        )
        .await;
    assert_eq!(approve.status().as_u16(), 403);
    let own_row = api.state.store.device(&own_id).unwrap().unwrap();
    assert_eq!(own_row.state, DeviceState::Pending);
}

#[tokio::test]
async fn only_a_live_challenge_of_this_session_for_this_key_is_an_answer() {
    let api = Api::start().await;
    let pat = api.server.member("pat@acme.example", Role::Member);
    let first = api.sign_in("pat@acme.example").await;
    let other = api.sign_in("pat@acme.example").await;
    let key = SecretKey::generate(&mut rand::thread_rng());
    let public = public_key_of(&key);
    let store = &api.state.store;
    let refused = |reply: reqwest::Response| assert_eq!(reply.status().as_u16(), 403);

    // The proof for another key.
    let elsewhere = SecretKey::generate(&mut rand::thread_rng());
    let elsewhere_public = public_key_of(&elsewhere);
    let for_elsewhere = api.open_challenge(
        &elsewhere,
        &api.enroll_challenge(&first.access, &elsewhere_public).await,
    );
    api.enroll_challenge(&first.access, &public).await;
    refused(
        api.enroll_with(&first.access, &public, Some(&for_elsewhere))
            .await,
    );

    // The proof for a challenge issued to the member's other session.
    let to_other = api.open_challenge(&key, &api.enroll_challenge(&other.access, &public).await);
    api.enroll_challenge(&first.access, &public).await;
    refused(
        api.enroll_with(&first.access, &public, Some(&to_other))
            .await,
    );

    // A challenge past its time.
    let session = store.session_by_access(&first.access).unwrap().unwrap();
    let stale = store
        .issue_enroll_challenge_at(
            &session.id,
            &public,
            crate::state::now_ms() - ENROLL_CHALLENGE_TTL_MS - 1,
        )
        .unwrap();
    refused(
        api.enroll_with(&first.access, &public, Some(&STANDARD.encode(*stale)))
            .await,
    );

    assert_eq!(store.devices_of(&pat.id).unwrap(), vec![]);
    let session = store.session_by_access(&first.access).unwrap().unwrap();
    assert_eq!(session.device_id, None);

    // The right proof enrolls, once.
    let proof = api.open_challenge(&key, &api.enroll_challenge(&first.access, &public).await);
    let enrolled = api.enroll_with(&first.access, &public, Some(&proof)).await;
    assert_eq!(enrolled.status().as_u16(), 200);
    refused(api.enroll_with(&first.access, &public, Some(&proof)).await);
    // The other session's challenge was its own all along, and still answers
    // for it.
    let theirs = api
        .enroll_with(&other.access, &public, Some(&to_other))
        .await;
    assert_eq!(theirs.status().as_u16(), 200);
    assert_eq!(store.devices_of(&pat.id).unwrap().len(), 1);

    // A challenge is for a key, asked for under a bearer.
    for body in [json!({}), json!({ "devicePubkey": "c2hvcnQ=" })] {
        let reply = api
            .call(
                Method::POST,
                "/v1/members/enroll/challenge",
                Some(&first.access),
                Some(body),
            )
            .await;
        assert_eq!(reply.status().as_u16(), 400);
    }
    let anonymous = api
        .call(
            Method::POST,
            "/v1/members/enroll/challenge",
            None,
            Some(json!({ "devicePubkey": public })),
        )
        .await;
    assert_eq!(anonymous.status().as_u16(), 401);
}

#[tokio::test]
async fn an_approved_device_signing_in_again_proves_its_key_and_is_itself_again() {
    let api = Api::start().await;
    let pat = api.server.member("pat@acme.example", Role::Member);
    let lockbox = api.server.lockbox("Deploy");
    api.server
        .store
        .add_lockbox_member(&lockbox.id, &pat.id, LockboxRole::Reader)
        .unwrap();
    let (device, secret) = api.server.approved_device(&pat);

    let fresh = api.sign_in("pat@acme.example").await;
    let again = json(api.enroll(&fresh.access, &secret).await).await;
    assert_eq!(again["deviceId"], device.id);
    assert_eq!(again["state"], "approved");
    let session = api
        .state
        .store
        .session_by_access(&fresh.access)
        .unwrap()
        .unwrap();
    assert_eq!(session.device_id.as_deref(), Some(device.id.as_str()));
    let listing = json(
        api.call(
            Method::GET,
            "/v1/members/me/lockboxes",
            Some(&fresh.access),
            None,
        )
        .await,
    )
    .await;
    let sealed = STANDARD
        .decode(listing[0]["wrappedMaster"].as_str().unwrap())
        .unwrap();
    assert_eq!(secret.unseal(&sealed).unwrap().len(), 32);
}
