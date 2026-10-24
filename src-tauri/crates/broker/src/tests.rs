use std::io::Cursor;

use base64::{engine::general_purpose::STANDARD, Engine};
use crypto_box::{aead::Aead, Nonce};
use serde_json::{json, Map, Value};

use crate::actions::{Client, Connection, Login};
use crate::frame;
use crate::grant::{GrantRequest, ItemSelector, Mode, Target};
use crate::lockbox::save_login_in;
use crate::protocol::{increment, Code, NONCE_LEN, VERSION};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::socket::{root_dir, socket_name, IDENTIFIER};
use crate::testing::{
    error_code, login, nonce, raw, ready, seed, stored, vault, Extension, GrantAnswer, Mock,
    GRANT_ID,
};
use rowel_core::models::{Entry, PasswordHistoryItem};
use rowel_core::store::VaultStore;

// Passkeys through the extension, end to end.
mod ceremonies;

// --- what the host claims to be --------------------------------------------------

// The extension turns features on by the version alone (see `VERSION`): this
// host answers the passkey actions, so it claims the release that turns them
// on — and not the one after, whose default passkey *group* the vault has no
// groups to honour.
#[test]
fn the_version_claimed_turns_on_nothing_this_host_lacks() {
    assert_eq!(VERSION, "2.7.7");
}

// --- the exchange ------------------------------------------------------------

#[test]
fn increment_carries_across_bytes() {
    let mut nonce = [0xff; NONCE_LEN];
    nonce[2] = 0x01;
    increment(&mut nonce);
    assert_eq!(&nonce[..4], &[0x00, 0x00, 0x02, 0xff]);
}

#[test]
fn the_exchange_answers_with_the_hosts_key_and_the_next_nonce() {
    let (extension, _) = ready();
    assert!(extension.host.is_some());
}

#[test]
fn an_exchange_without_a_key_is_refused() {
    let mut connection = Connection::new(Mock::unlocked());
    let response = raw(
        &mut connection,
        &json!({ "action": "change-public-keys", "nonce": STANDARD.encode(nonce()) }),
    );
    assert_eq!(
        error_code(&response),
        Code::ClientPublicKeyNotReceived as u8
    );
}

#[test]
fn a_sealed_request_before_the_exchange_is_refused() {
    let mut connection = Connection::new(Mock::unlocked());
    let response = raw(
        &mut connection,
        &json!({ "action": "get-databasehash", "message": "x", "nonce": "y" }),
    );
    assert_eq!(
        error_code(&response),
        Code::ClientPublicKeyNotReceived as u8
    );
}

#[test]
fn a_tampered_message_is_refused() {
    let (_extension, mut connection) = ready();
    let mut request = Map::new();
    request.insert("action".into(), "get-databasehash".into());
    request.insert("message".into(), STANDARD.encode(b"not a box").into());
    request.insert("nonce".into(), STANDARD.encode(nonce()).into());
    let response = raw(&mut connection, &Value::Object(request));
    assert_eq!(error_code(&response), Code::CannotDecryptMessage as u8);
}

#[test]
fn a_message_whose_action_differs_from_the_envelope_is_refused() {
    let (extension, mut connection) = ready();
    let sealed = extension.host.as_ref().unwrap();
    let nonce = nonce();
    let inner = json!({ "action": "lock-database" });
    let cipher = sealed
        .encrypt(
            Nonce::from_slice(&nonce),
            serde_json::to_vec(&inner).unwrap().as_slice(),
        )
        .unwrap();
    let response = raw(
        &mut connection,
        &json!({ "action": "get-databasehash", "message": STANDARD.encode(cipher), "nonce": STANDARD.encode(nonce) }),
    );
    assert_eq!(error_code(&response), Code::IncorrectAction as u8);
}

#[test]
fn junk_is_refused_as_an_empty_message() {
    let mut connection = Connection::new(Mock::unlocked());
    let reply = connection.handle(b"[1, 2");
    let response: Value = serde_json::from_slice(&reply).unwrap();
    assert_eq!(error_code(&response), Code::EmptyMessageReceived as u8);
}

// --- the locked vault --------------------------------------------------------

#[test]
fn a_locked_vault_refuses_everything_but_the_exchange() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock::unlocked());
    connection_mock(&connection).locked.set(true);
    let exchanged = extension.exchange(&mut connection);
    assert_eq!(exchanged["success"], "true");

    let response = extension.send(&mut connection, "get-databasehash", json!({}));
    assert_eq!(error_code(&response), Code::DatabaseNotOpened as u8);
    assert_eq!(response["action"], "get-databasehash");
    assert_eq!(connection_mock(&connection).raises.get(), 0);
}

#[test]
fn trigger_unlock_on_a_locked_vault_brings_the_app_forward() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock::unlocked());
    connection_mock(&connection).locked.set(true);
    extension.exchange(&mut connection);

    let mut outer = Map::new();
    outer.insert("triggerUnlock".into(), "true".into());
    let response = extension.send_with(&mut connection, "get-databasehash", json!({}), outer);
    assert_eq!(error_code(&response), Code::DatabaseNotOpened as u8);
    assert_eq!(connection_mock(&connection).raises.get(), 1);
}

fn connection_mock(connection: &Connection<Mock>) -> &Mock {
    connection.host()
}

// --- association -------------------------------------------------------------

#[test]
fn get_databasehash_answers_with_the_hash() {
    let (extension, mut connection) = ready();
    let response = extension.send(&mut connection, "get-databasehash", json!({}));
    assert_eq!(response["hash"], "abc123");
}

#[test]
fn associate_needs_the_session_key_repeated_inside() {
    let (extension, mut connection) = ready();
    let response = extension.send(
        &mut connection,
        "associate",
        json!({ "key": "someone else's", "idKey": "id-key" }),
    );
    assert_eq!(error_code(&response), Code::AssociationFailed as u8);
    assert!(connection.host().clients.borrow().is_empty());
}

#[test]
fn associate_asks_the_user_and_remembers_the_answer() {
    let (extension, mut connection) = ready();
    let response = extension.send(
        &mut connection,
        "associate",
        json!({ "key": extension.public_key(), "idKey": "id-key" }),
    );
    assert_eq!(response["id"], "Chrome");
    assert_eq!(response["hash"], "abc123");
    assert_eq!(
        connection.host().clients.borrow().as_slice(),
        &[Client {
            name: "Chrome".into(),
            key: "id-key".into()
        }]
    );

    // And the connection is associated from here on: no keys needed.
    let logins = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com/login" }),
    );
    assert_eq!(logins["count"], 1);
}

#[test]
fn a_refused_associate_is_denied_and_forgotten() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock {
        consent: None,
        ..Mock::unlocked()
    });
    extension.exchange(&mut connection);
    let response = extension.send(
        &mut connection,
        "associate",
        json!({ "key": extension.public_key(), "idKey": "id-key" }),
    );
    assert_eq!(error_code(&response), Code::ActionCancelledOrDenied as u8);
    assert!(connection.host().clients.borrow().is_empty());
    let logins = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com" }),
    );
    assert_eq!(error_code(&logins), Code::AssociationFailed as u8);
}

#[test]
fn a_yes_that_could_not_be_kept_is_refused_not_reported() {
    // The write failed, or the vault the user was asked for is no longer the
    // one open: the extension must not hear "in" when its next request
    // will find it is not.
    let (extension, mut connection) = ready();
    connection.host().remember_fails.set(true);
    let response = extension.send(
        &mut connection,
        "associate",
        json!({ "key": extension.public_key(), "idKey": "id-key" }),
    );
    assert_eq!(error_code(&response), Code::AssociationFailed as u8);
    assert!(connection.host().clients.borrow().is_empty());
    let logins = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com" }),
    );
    assert_eq!(error_code(&logins), Code::AssociationFailed as u8);
}

#[test]
fn test_associate_proves_a_remembered_key_under_its_name() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock::unlocked().known("Chrome", "id-key"));
    extension.exchange(&mut connection);

    let wrong = extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "other" }),
    );
    assert_eq!(error_code(&wrong), Code::AssociationFailed as u8);
    let renamed = extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Firefox", "key": "id-key" }),
    );
    assert_eq!(error_code(&renamed), Code::AssociationFailed as u8);

    let right = extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "id-key" }),
    );
    assert_eq!(right["id"], "Chrome");
    assert_eq!(right["hash"], "abc123");
}

#[test]
fn a_request_carrying_a_remembered_key_counts_as_associated() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock::unlocked().known("Chrome", "id-key"));
    extension.exchange(&mut connection);
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com", "keys": [{ "id": "Chrome", "key": "id-key" }] }),
    );
    assert_eq!(response["count"], 1);
}

// --- what the extension gets -------------------------------------------------

fn associated() -> (Extension, Connection<Mock>) {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock::unlocked().known("Chrome", "id-key"));
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "id-key" }),
    );
    (extension, connection)
}

#[test]
fn get_logins_answers_with_the_logins_for_the_site() {
    let (extension, mut connection) = associated();
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com/login", "submitUrl": "https://github.com/session" }),
    );
    assert_eq!(response["count"], 1);
    assert_eq!(response["hash"], "abc123");
    assert_eq!(
        response["entries"],
        json!([{ "login": "octocat", "password": "hunter2", "name": "GitHub", "uuid": "gh", "totp": "123456" }])
    );
}

#[test]
fn get_logins_for_a_site_without_any_is_the_no_logins_refusal() {
    let (extension, mut connection) = associated();
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://example.com" }),
    );
    assert_eq!(error_code(&response), Code::NoLoginsFound as u8);
}

#[test]
fn get_logins_without_a_usable_url_is_refused() {
    let (extension, mut connection) = associated();
    let missing = extension.send(&mut connection, "get-logins", json!({}));
    assert_eq!(error_code(&missing), Code::NoUrlProvided as u8);
    let junk = extension.send(&mut connection, "get-logins", json!({ "url": "not a url" }));
    assert_eq!(error_code(&junk), Code::NoUrlProvided as u8);
}

#[test]
fn get_totp_answers_with_the_current_code() {
    let (extension, mut connection) = associated();
    let response = extension.send(&mut connection, "get-totp", json!({ "uuid": "gh" }));
    assert_eq!(response["totp"], "654321");
    let unknown = extension.send(&mut connection, "get-totp", json!({ "uuid": "nope" }));
    assert_eq!(error_code(&unknown), Code::NoValidUuidProvided as u8);
    let blank = extension.send(&mut connection, "get-totp", json!({}));
    assert_eq!(error_code(&blank), Code::NoValidUuidProvided as u8);
}

// --- fills in agent mode: every fill is a grant ----------------------------------

const GH_LOGIN: &str = "https://github.com/login";

fn two_logins() -> Vec<Login> {
    vec![
        login("gh", "GitHub", "octocat", "hunter2", Some("123456")),
        login("gh2", "GitHub (work)", "octocorp", "s3cret", None),
    ]
}

/// Associated, with two logins for GitHub and the host scripted to `answer`.
fn agent_connection(answer: GrantAnswer) -> (Extension, Connection<Mock>) {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock {
        logins: two_logins(),
        grant_answer: answer,
        ..Mock::unlocked().known("Chrome", "id-key")
    });
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "id-key" }),
    );
    (extension, connection)
}

fn sorted_keys(response: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = response
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

// The reply to a `get-logins` without the new fields has exactly the fields
// it had, and nobody is asked anything — whatever the host would answer.
#[test]
fn get_logins_without_the_agent_fields_is_what_it_always_was() {
    let (extension, mut connection) = agent_connection(GrantAnswer::Active("gh2".into()));
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "submitUrl": "https://github.com/session" }),
    );
    assert_eq!(response["count"], 2);
    assert_eq!(response["hash"], "abc123");
    assert_eq!(
        response["entries"],
        json!([
            { "login": "octocat", "password": "hunter2", "name": "GitHub", "uuid": "gh", "totp": "123456" },
            { "login": "octocorp", "password": "s3cret", "name": "GitHub (work)", "uuid": "gh2" },
        ])
    );
    assert_eq!(
        sorted_keys(&response),
        ["action", "count", "entries", "hash", "nonce", "success", "version"]
    );
    assert!(connection.host().grant_requests.borrow().is_empty());
}

#[test]
fn in_agent_mode_a_fill_is_a_grant_and_only_the_admitted_login_is_served() {
    let (extension, mut connection) = agent_connection(GrantAnswer::Active("gh2".into()));
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "agentMode": "true" }),
    );
    assert_eq!(response["count"], 1);
    assert_eq!(response["grantId"], GRANT_ID);
    assert_eq!(response["hash"], "abc123");
    assert_eq!(
        response["entries"],
        json!([{ "login": "octocorp", "password": "s3cret", "name": "GitHub (work)", "uuid": "gh2" }])
    );
    assert_eq!(
        sorted_keys(&response),
        ["action", "count", "entries", "grantId", "hash", "nonce", "success", "version"]
    );

    // What the host was asked: by the connection's key, for the page's
    // logins as candidates, at the page, to fill.
    let asked = connection.host().grant_requests.borrow();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].0, "id-key");
    assert_eq!(
        asked[0].1,
        GrantRequest {
            client_key: "id-key".into(),
            agent_id: None,
            acting_for: None,
            items: ItemSelector::Ids(vec!["gh".into(), "gh2".into()]),
            targets: vec![Target::Origin(GH_LOGIN.into())],
            modes: vec![Mode::Fill],
            run_id: None,
            reason: None,
        }
    );
}

#[test]
fn agent_mode_is_read_as_the_string_true_or_a_json_true() {
    let (extension, mut connection) = agent_connection(GrantAnswer::Active("gh".into()));
    let boolean = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "agentMode": true }),
    );
    assert_eq!(boolean["grantId"], GRANT_ID);
    assert_eq!(boolean["count"], 1);

    let off = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "agentMode": "false" }),
    );
    assert_eq!(off["count"], 2);
    assert!(off.get("grantId").is_none());
    assert_eq!(connection.host().grant_requests.borrow().len(), 1);
}

#[test]
fn a_denied_grant_refuses_the_fill_and_the_refusal_carries_nothing() {
    let (extension, mut connection) = agent_connection(GrantAnswer::Denied);
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "agentMode": "true" }),
    );
    assert_eq!(error_code(&response), Code::GrantDenied as u8);
    assert_eq!(response["error"], "Grant denied");
    assert_eq!(sorted_keys(&response), ["action", "error", "errorCode"]);
    let wire = serde_json::to_string(&response).unwrap();
    for secret in [
        "hunter2", "s3cret", "123456", "octocat", "entries", "password",
    ] {
        assert!(!wire.contains(secret), "{secret} in {wire}");
    }
    assert_eq!(connection.host().grant_requests.borrow().len(), 1);
}

#[test]
fn a_pending_grant_says_so_and_when_to_ask_again() {
    let (extension, mut connection) = agent_connection(GrantAnswer::Pending(Some(1_500)));
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "agentMode": "true" }),
    );
    assert_eq!(error_code(&response), Code::GrantPending as u8);
    assert_eq!(response["retryAfterMs"], 1_500);
    assert_eq!(
        sorted_keys(&response),
        ["action", "error", "errorCode", "retryAfterMs"]
    );

    // No time given: no field, and the refusal is the one it always was.
    let (extension, mut connection) = agent_connection(GrantAnswer::Pending(None));
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "agentMode": "true" }),
    );
    assert_eq!(error_code(&response), Code::GrantPending as u8);
    assert_eq!(sorted_keys(&response), ["action", "error", "errorCode"]);
}

#[test]
fn with_agents_off_an_agent_mode_fill_is_refused_and_a_plain_one_is_not() {
    let (extension, mut connection) = agent_connection(GrantAnswer::Disabled);
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "agentMode": "true" }),
    );
    assert_eq!(error_code(&response), Code::AgentsDisabled as u8);
    assert_eq!(
        response["error"],
        "Agents are turned off in Rowel's settings"
    );
    let plain = extension.send(&mut connection, "get-logins", json!({ "url": GH_LOGIN }));
    assert_eq!(plain["count"], 2);
}

// Before the grant comes the page's logins: a site with none is the refusal
// it always was, and nobody is asked for nothing.
#[test]
fn in_agent_mode_a_site_without_logins_asks_nobody() {
    let (extension, mut connection) = agent_connection(GrantAnswer::Active("gh".into()));
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://example.com", "agentMode": "true" }),
    );
    assert_eq!(error_code(&response), Code::NoLoginsFound as u8);
    assert!(connection.host().grant_requests.borrow().is_empty());
}

#[test]
fn a_grant_the_extension_holds_serves_only_what_it_covers() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock {
        logins: two_logins(),
        ..Mock::unlocked()
            .known("Chrome", "id-key")
            .granting("g1", "gh", Mode::Fill)
    });
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "id-key" }),
    );

    let covered = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "grantId": "g1" }),
    );
    assert_eq!(covered["count"], 1);
    assert_eq!(covered["entries"][0]["uuid"], "gh");
    assert!(covered.get("grantId").is_none());

    let unknown = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "grantId": "g9" }),
    );
    assert_eq!(error_code(&unknown), Code::GrantDenied as u8);
    assert!(
        connection.host().grant_requests.borrow().is_empty(),
        "a held grant asks nobody"
    );

    // Both fields: the grant in hand wins, and nobody is asked again.
    let both = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "grantId": "g1", "agentMode": "true" }),
    );
    assert_eq!(both["count"], 1);
    assert!(connection.host().grant_requests.borrow().is_empty());
}

#[test]
fn get_totp_under_a_grant_needs_the_grant_to_cover_the_login_for_a_fill() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(
        Mock::unlocked()
            .known("Chrome", "id-key")
            .granting("g1", "gh", Mode::Fill)
            .granting("g2", "gh", Mode::Passkey),
    );
    extension.exchange(&mut connection);
    extension.send(
        &mut connection,
        "test-associate",
        json!({ "id": "Chrome", "key": "id-key" }),
    );

    let covered = extension.send(
        &mut connection,
        "get-totp",
        json!({ "uuid": "gh", "grantId": "g1" }),
    );
    assert_eq!(covered["totp"], "654321");
    let unknown = extension.send(
        &mut connection,
        "get-totp",
        json!({ "uuid": "gh", "grantId": "g9" }),
    );
    assert_eq!(error_code(&unknown), Code::GrantDenied as u8);
    let other_mode = extension.send(
        &mut connection,
        "get-totp",
        json!({ "uuid": "gh", "grantId": "g2" }),
    );
    assert_eq!(error_code(&other_mode), Code::GrantDenied as u8);
    let plain = extension.send(&mut connection, "get-totp", json!({ "uuid": "gh" }));
    assert_eq!(plain["totp"], "654321");
}

#[test]
fn report_exposure_reaches_the_host() {
    let (extension, mut connection) = associated();
    let response = extension.send(
        &mut connection,
        "report-exposure",
        json!({ "grantId": "g1" }),
    );
    assert_eq!(response["success"], "true");
    assert_eq!(
        sorted_keys(&response),
        ["action", "nonce", "success", "version"]
    );
    assert_eq!(connection.host().exposures.borrow().as_slice(), ["g1"]);

    let blank = extension.send(&mut connection, "report-exposure", json!({}));
    assert_eq!(error_code(&blank), Code::NoValidUuidProvided as u8);
    assert_eq!(connection.host().exposures.borrow().len(), 1);
}

#[test]
fn the_grant_paths_are_refused_before_association() {
    let (extension, mut connection) = ready();
    let agent = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "agentMode": "true" }),
    );
    assert_eq!(error_code(&agent), Code::AssociationFailed as u8);
    let held = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": GH_LOGIN, "grantId": "g1" }),
    );
    assert_eq!(error_code(&held), Code::AssociationFailed as u8);
    let totp = extension.send(
        &mut connection,
        "get-totp",
        json!({ "uuid": "gh", "grantId": "g1" }),
    );
    assert_eq!(error_code(&totp), Code::AssociationFailed as u8);
    let exposure = extension.send(
        &mut connection,
        "report-exposure",
        json!({ "grantId": "g1" }),
    );
    assert_eq!(error_code(&exposure), Code::AssociationFailed as u8);

    let host = connection.host();
    assert!(host.grant_requests.borrow().is_empty());
    assert!(host.exposures.borrow().is_empty());
}

#[test]
fn set_login_without_a_uuid_saves_a_new_login_for_the_site() {
    let (extension, mut connection) = associated();
    let response = extension.send(
        &mut connection,
        "set-login",
        json!({
            "url": "https://Example.com/signup",
            "submitUrl": "https://example.com/session",
            "login": "me",
            "password": "s3cret",
            "group": "",
            "groupUuid": "",
            "downloadFavicon": "true",
        }),
    );
    assert_eq!(response["error"], "success");
    assert_eq!(response["hash"], "abc123");
    assert!(response["count"].is_null() && response["entries"].is_null());
    assert_eq!(
        connection.host().saves.borrow().as_slice(),
        &[(
            None,
            "https://Example.com/signup".into(),
            "example.com".into(),
            "me".into(),
            "s3cret".into()
        )]
    );
}

#[test]
fn set_login_with_a_uuid_updates_that_login() {
    let (extension, mut connection) = associated();
    let response = extension.send(
        &mut connection,
        "set-login",
        json!({ "url": "https://github.com/login", "uuid": "gh", "login": "octocat", "password": "new" }),
    );
    assert_eq!(response["error"], "success");
    let saves = connection.host().saves.borrow();
    assert_eq!(saves.len(), 1);
    assert_eq!(saves[0].0.as_deref(), Some("gh"));
    assert_eq!(saves[0].4, "new");
}

#[test]
fn set_login_for_an_unknown_uuid_is_refused() {
    let (extension, mut connection) = associated();
    let response = extension.send(
        &mut connection,
        "set-login",
        json!({ "url": "https://github.com", "uuid": "nope", "login": "a", "password": "b" }),
    );
    assert_eq!(error_code(&response), Code::NoValidUuidProvided as u8);
    assert!(connection.host().saves.borrow().is_empty());
}

#[test]
fn set_login_needs_an_association() {
    let (extension, mut connection) = ready();
    let response = extension.send(
        &mut connection,
        "set-login",
        json!({ "url": "https://github.com", "login": "a", "password": "b" }),
    );
    assert_eq!(error_code(&response), Code::AssociationFailed as u8);
    assert!(connection.host().saves.borrow().is_empty());
}

#[test]
fn a_set_login_that_fails_to_write_says_so_inside_the_reply() {
    let mut extension = Extension::new();
    let mut connection = Connection::new(Mock {
        read_only: true,
        ..Mock::unlocked().known("Chrome", "id-key")
    });
    extension.exchange(&mut connection);
    let response = extension.send(
        &mut connection,
        "set-login",
        json!({ "url": "https://github.com", "login": "a", "password": "b", "keys": [{ "id": "Chrome", "key": "id-key" }] }),
    );
    assert_eq!(response["error"], "error");
    assert_eq!(response["hash"], "abc123");
}

// --- the save itself, against a store --------------------------------------------

const THEN: &str = "2026-01-01T00:00:00.000Z";
const NOW: &str = "2026-09-26T12:00:00.000Z";

#[test]
fn a_save_without_an_id_creates_a_login_for_the_site() {
    let (_dir, store, cipher) = vault();
    save_login_in(
        &store,
        &cipher,
        None,
        "https://github.com/signup",
        "github.com",
        "octocat",
        "hunter2",
        NOW,
    )
    .unwrap();

    let metas = store.list().unwrap();
    assert_eq!(metas.len(), 1);
    assert_eq!(metas[0].kind, "login");
    assert_eq!(metas[0].url_host, "github.com");
    let entry = stored(&store, &cipher, &metas[0].id);
    assert_eq!(entry.title, "github.com");
    assert_eq!(entry.website.as_deref(), Some("https://github.com/signup"));
    assert_eq!(entry.username.as_deref(), Some("octocat"));
    assert_eq!(entry.password.as_deref(), Some("hunter2"));
    assert_eq!(entry.created_at.as_deref(), Some(NOW));
    assert_eq!(entry.updated_at.as_deref(), Some(NOW));
    assert_eq!(entry.password_updated_at.as_deref(), Some(NOW));
}

// A login kept by its email is served to the extension under that email
// (`logins_for`), so what comes back goes to the same field: the row must not
// grow a username beside the email it already had.
#[test]
fn a_save_over_an_email_login_writes_back_to_its_email() {
    let (_dir, store, cipher) = vault();
    seed(
        &store,
        &cipher,
        &Entry {
            id: "gh".into(),
            kind: "login".into(),
            title: "GitHub".into(),
            website: Some("https://github.com".into()),
            email: Some("octocat@example.com".into()),
            password: Some("old".into()),
            ..Entry::default()
        },
    );

    save_login_in(
        &store,
        &cipher,
        Some("gh"),
        "https://github.com/login",
        "github.com",
        "octo@example.com",
        "new",
        NOW,
    )
    .unwrap();

    let entry = stored(&store, &cipher, "gh");
    assert_eq!(entry.email.as_deref(), Some("octo@example.com"));
    assert_eq!(entry.username, None, "no second name beside the email");
    assert_eq!(entry.password.as_deref(), Some("new"));

    // A name that is not an email cannot go in the email field: it becomes
    // the username, and the email is left as it was.
    save_login_in(
        &store,
        &cipher,
        Some("gh"),
        "https://github.com/login",
        "github.com",
        "octocat",
        "new",
        NOW,
    )
    .unwrap();
    let entry = stored(&store, &cipher, "gh");
    assert_eq!(entry.username.as_deref(), Some("octocat"));
    assert_eq!(entry.email.as_deref(), Some("octo@example.com"));

    // With a username of its own, that is the field — the email stays what
    // it was, whatever the page called the user.
    seed(
        &store,
        &cipher,
        &Entry {
            id: "both".into(),
            kind: "login".into(),
            title: "GitHub".into(),
            website: Some("https://github.com".into()),
            username: Some("octocat".into()),
            email: Some("octocat@example.com".into()),
            password: Some("old".into()),
            ..Entry::default()
        },
    );
    save_login_in(
        &store,
        &cipher,
        Some("both"),
        "https://github.com/login",
        "github.com",
        "octocat2",
        "new",
        NOW,
    )
    .unwrap();
    let entry = stored(&store, &cipher, "both");
    assert_eq!(entry.username.as_deref(), Some("octocat2"));
    assert_eq!(entry.email.as_deref(), Some("octocat@example.com"));
}

#[test]
fn a_save_over_a_login_changes_only_what_the_page_sent() {
    let (_dir, store, cipher) = vault();
    seed(
        &store,
        &cipher,
        &Entry {
            id: "gh".into(),
            kind: "login".into(),
            title: "GitHub (work)".into(),
            website: Some("https://github.com".into()),
            username: Some("octocat".into()),
            password: Some("old".into()),
            otp: Some("JBSWY3DPEHPK3PXP".into()),
            tags: Some(vec!["work".into()]),
            created_at: Some(THEN.into()),
            updated_at: Some(THEN.into()),
            password_updated_at: Some(THEN.into()),
            ..Entry::default()
        },
    );

    save_login_in(
        &store,
        &cipher,
        Some("gh"),
        "https://github.com/settings",
        "github.com",
        "octocat",
        "new",
        NOW,
    )
    .unwrap();

    let entry = stored(&store, &cipher, "gh");
    assert_eq!(entry.password.as_deref(), Some("new"));
    assert_eq!(entry.password_updated_at.as_deref(), Some(NOW), "rotated");
    assert_eq!(entry.updated_at.as_deref(), Some(NOW));
    // Untouched: the row is the user's, the page only knows two fields of it.
    assert_eq!(entry.title, "GitHub (work)");
    assert_eq!(entry.website.as_deref(), Some("https://github.com"));
    assert_eq!(entry.otp.as_deref(), Some("JBSWY3DPEHPK3PXP"));
    assert_eq!(entry.tags, Some(vec!["work".to_string()]));
    assert_eq!(entry.created_at.as_deref(), Some(THEN));
    assert_eq!(
        store.list().unwrap().len(),
        1,
        "an update, not a second row"
    );
}

#[test]
fn a_save_with_the_same_password_does_not_count_as_a_rotation() {
    let (_dir, store, cipher) = vault();
    seed(
        &store,
        &cipher,
        &Entry {
            id: "gh".into(),
            kind: "login".into(),
            title: "GitHub".into(),
            username: Some("old-name".into()),
            password: Some("same".into()),
            password_updated_at: Some(THEN.into()),
            ..Entry::default()
        },
    );
    save_login_in(
        &store,
        &cipher,
        Some("gh"),
        "https://github.com",
        "github.com",
        "new-name",
        "same",
        NOW,
    )
    .unwrap();
    let entry = stored(&store, &cipher, "gh");
    assert_eq!(entry.username.as_deref(), Some("new-name"));
    assert_eq!(entry.password_updated_at.as_deref(), Some(THEN));
    assert_eq!(entry.updated_at.as_deref(), Some(NOW));
    assert_eq!(entry.password_history, None, "nothing was replaced");
}

// A password the page rotated is kept in the login's history, in front of
// what was already there — the same record the editor's save makes.
#[test]
fn a_save_that_rotates_the_password_keeps_the_old_one_in_history() {
    let (_dir, store, cipher) = vault();
    seed(
        &store,
        &cipher,
        &Entry {
            id: "gh".into(),
            kind: "login".into(),
            title: "GitHub".into(),
            username: Some("octocat".into()),
            password: Some("old".into()),
            password_history: Some(vec![PasswordHistoryItem {
                password: "older".into(),
                replaced_at: THEN.into(),
            }]),
            ..Entry::default()
        },
    );
    save_login_in(
        &store,
        &cipher,
        Some("gh"),
        "https://github.com",
        "github.com",
        "octocat",
        "new",
        NOW,
    )
    .unwrap();

    let entry = stored(&store, &cipher, "gh");
    assert_eq!(entry.password.as_deref(), Some("new"));
    let history: Vec<(&str, &str)> = entry
        .password_history
        .iter()
        .flatten()
        .map(|p| (p.password.as_str(), p.replaced_at.as_str()))
        .collect();
    assert_eq!(history, [("old", NOW), ("older", THEN)]);
}

#[test]
fn a_save_over_something_that_is_not_a_login_is_refused() {
    let (_dir, store, cipher) = vault();
    seed(
        &store,
        &cipher,
        &Entry {
            id: "note".into(),
            kind: "note".into(),
            title: "Not a login".into(),
            note: Some("keep".into()),
            ..Entry::default()
        },
    );
    let missing = save_login_in(
        &store,
        &cipher,
        Some("nope"),
        "https://x.com",
        "x.com",
        "a",
        "b",
        NOW,
    );
    assert_eq!(missing, Err(Code::NoValidUuidProvided));
    let wrong_kind = save_login_in(
        &store,
        &cipher,
        Some("note"),
        "https://x.com",
        "x.com",
        "a",
        "b",
        NOW,
    );
    assert_eq!(wrong_kind, Err(Code::NoValidUuidProvided));
    assert_eq!(
        stored(&store, &cipher, "note").note.as_deref(),
        Some("keep")
    );
}

#[test]
fn generate_password_needs_no_association() {
    let (extension, mut connection) = ready();
    let response = extension.send(
        &mut connection,
        "generate-password",
        json!({ "requestID": "abcdefgh" }),
    );
    assert_eq!(response["password"], "generated");
}

#[test]
fn lock_database_locks_for_an_extension_that_was_let_in() {
    let (extension, mut connection) = associated();
    let response = extension.send(&mut connection, "lock-database", json!({}));
    assert_eq!(response["success"], "true");
    assert_eq!(connection.host().locks.get(), 1);
}

#[test]
fn lock_database_is_refused_to_a_stranger() {
    let (extension, mut connection) = ready();
    let response = extension.send(&mut connection, "lock-database", json!({}));
    assert_eq!(error_code(&response), Code::AssociationFailed as u8);
    assert_eq!(connection.host().locks.get(), 0);
}

// --- access that ends -------------------------------------------------------

#[test]
fn a_forgotten_extension_loses_access_on_its_next_request() {
    let (extension, mut connection) = associated();
    let before = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com" }),
    );
    assert_eq!(before["count"], 1);

    // Settings › forget, while the connection is still up.
    connection.host().clients.borrow_mut().clear();

    let after = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com", "keys": [{ "id": "Chrome", "key": "id-key" }] }),
    );
    assert_eq!(error_code(&after), Code::AssociationFailed as u8);
    let totp = extension.send(&mut connection, "get-totp", json!({ "uuid": "gh" }));
    assert_eq!(error_code(&totp), Code::AssociationFailed as u8);
}

#[test]
fn an_association_does_not_follow_the_connection_into_another_vault() {
    // The list is the open vault's: a switch to a workspace the extension was
    // never let into reads as a vault with no clients at all.
    let (extension, mut connection) = associated();
    *connection.host().clients.borrow_mut() = vec![Client {
        name: "Chrome".into(),
        key: "a key of the other vault's".into(),
    }];
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com", "keys": [{ "id": "Chrome", "key": "id-key" }] }),
    );
    assert_eq!(error_code(&response), Code::AssociationFailed as u8);

    // Let in there too, and the key it carries opens it again.
    connection.host().clients.borrow_mut().push(Client {
        name: "Chrome".into(),
        key: "id-key".into(),
    });
    let response = extension.send(
        &mut connection,
        "get-logins",
        json!({ "url": "https://github.com", "keys": [{ "id": "Chrome", "key": "id-key" }] }),
    );
    assert_eq!(response["count"], 1);
}

#[test]
fn groups_and_unknown_actions_are_refused_in_the_protocols_words() {
    let (extension, mut connection) = associated();
    let groups = extension.send(&mut connection, "get-database-groups", json!({}));
    assert_eq!(error_code(&groups), Code::NoGroupsFound as u8);
    let entries = extension.send(&mut connection, "get-database-entries", json!({}));
    assert_eq!(error_code(&entries), Code::IncorrectAction as u8);
    assert_eq!(entries["error"], Code::IncorrectAction.message());
}

// --- framing and the wire ----------------------------------------------------

#[test]
fn frames_round_trip_and_end_cleanly() {
    let mut wire = Vec::new();
    frame::write(&mut wire, b"{\"a\":1}").unwrap();
    frame::write(&mut wire, b"").unwrap();
    let mut reader = Cursor::new(wire);
    assert_eq!(
        frame::read(&mut reader).unwrap().as_deref(),
        Some(&b"{\"a\":1}"[..])
    );
    assert_eq!(frame::read(&mut reader).unwrap().as_deref(), Some(&b""[..]));
    assert_eq!(frame::read(&mut reader).unwrap(), None);
}

#[test]
fn a_frame_over_the_cap_is_refused_before_it_is_read() {
    let mut wire = Vec::new();
    wire.extend_from_slice(&((frame::MAX_FRAME as u32) + 1).to_ne_bytes());
    assert!(frame::read(&mut Cursor::new(wire)).is_err());
    assert!(frame::write(&mut Vec::new(), &vec![0u8; frame::MAX_FRAME + 1]).is_err());
}

#[test]
fn a_frame_cut_short_is_an_error_not_an_end() {
    let mut wire = Vec::new();
    wire.extend_from_slice(&8u32.to_ne_bytes());
    wire.extend_from_slice(b"abc");
    assert!(frame::read(&mut Cursor::new(wire)).is_err());
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[test]
fn chrome_and_firefox_keep_the_data_directory_socket() {
    // The debug override wins; a run that sets it has nothing to say about
    // the platform default.
    if std::env::var_os("ROWEL_DB_DIR").is_some() {
        return;
    }
    // Where it always was, on every platform, macOS included: the app-data
    // directory under the identifier, `dev` in a debug (`cargo test`) build.
    assert_eq!(
        root_dir(),
        Some(dirs::data_dir().unwrap().join(IDENTIFIER).join("dev"))
    );
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[test]
fn the_socket_name_follows_the_root() {
    let a = socket_name(std::path::Path::new("/tmp/rowel-a")).unwrap();
    let again = socket_name(std::path::Path::new("/tmp/rowel-a")).unwrap();
    let b = socket_name(std::path::Path::new("/tmp/rowel-b")).unwrap();
    assert_eq!(format!("{a:?}"), format!("{again:?}"));
    assert_ne!(format!("{a:?}"), format!("{b:?}"));
}
