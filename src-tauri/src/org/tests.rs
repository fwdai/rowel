//! The org lockbox primitives against an in-process Rowel Server
//! ([`fake_server`]), one layer below the commands: the sign-in, enrollment
//! and approval, two devices converging on an org lockbox through the server
//! as the provider, a rotation of its master, and the approvals stream.

use std::sync::{Arc, Mutex};

use reqwest::header::LOCATION;
use reqwest::redirect::Policy;
use tauri::async_runtime::block_on;
use zeroize::Zeroizing;

use super::client::{DeviceState, ServerClient, Session};
use super::connections::{Connection, SessionResolver};
use super::enrollment::EnrollmentKey;
use super::fake_server::{FakeServer, MEMBER_EMAIL, MEMBER_ID, TENANT};
use crate::commands::password_sync::two_devices::Device;
use crate::crypto::{KdfParams, VaultKey};
use crate::error::Error;
use crate::sync::engine;
use crate::sync::providers::rowel::Store;
use crate::sync::{challenge, gen_nonce, http_client_builder, Provider, VaultStore};

const NOW: i64 = 1_700_000_000_000;
const REDIRECT: &str = "http://127.0.0.1:4242/callback";

/// One signed-in device of the member: its client, its enrollment key and
/// the connection record a store would look its session up in.
struct Signed {
    client: ServerClient,
    key: EnrollmentKey,
    device_id: String,
    connections: Arc<Mutex<Vec<Connection>>>,
}

impl Signed {
    /// The whole sign-in as the app will run it: the authorization page,
    /// which the fake server answers with a redirect in place of an identity
    /// provider, then the code exchanged under the PKCE verifier.
    fn sign_in(server: &FakeServer) -> (ServerClient, Session) {
        block_on(async {
            let verifier = gen_nonce();
            let state = gen_nonce();
            let anonymous = ServerClient::new(&server.url).unwrap();
            let url = anonymous.authorize_url(REDIRECT, &state, &challenge(&verifier));
            let browser = http_client_builder()
                .redirect(Policy::none())
                .build()
                .unwrap();
            let resp = browser.get(url).send().await.unwrap();
            assert!(resp.status().is_redirection(), "{}", resp.status());
            let back = url::Url::parse(resp.headers()[LOCATION].to_str().unwrap()).unwrap();
            assert!(back.as_str().starts_with(REDIRECT));
            let param = |name: &str| {
                back.query_pairs()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v.into_owned())
                    .unwrap()
            };
            assert_eq!(param("state"), state);
            let session = anonymous
                .token(&param("code"), &verifier, REDIRECT)
                .await
                .unwrap();
            assert_eq!(session.token_type, "Bearer");
            (
                ServerClient::with_session(&server.url, &session.access_token).unwrap(),
                session,
            )
        })
    }

    /// Sign in and enroll a device named `name`; it comes back pending.
    fn enroll(server: &FakeServer, name: &str) -> Self {
        let (client, session) = Self::sign_in(server);
        let key = EnrollmentKey::generate();
        let enrolled = block_on(client.enroll(&key.public_key_b64(), name)).unwrap();
        assert_eq!(enrolled.state, DeviceState::Pending);
        let mut connection = Connection {
            url: client.base_url().to_string(),
            tenant_name: TENANT.into(),
            member_id: MEMBER_ID.into(),
            member_email: MEMBER_EMAIL.into(),
            member_name: None,
            device_id: Some(enrolled.device_id.clone()),
            device_state: Some(enrolled.state),
            access_token: String::new(),
            refresh_token: None,
            expires_at_ms: 0,
        };
        connection.adopt(session, NOW);
        Self {
            client,
            key,
            device_id: enrolled.device_id,
            connections: Arc::new(Mutex::new(vec![connection])),
        }
    }

    /// Sign in, enroll, and have the admin approve: a member's first device.
    fn approved(server: &FakeServer, name: &str) -> Self {
        let signed = Self::enroll(server, name);
        server.admin_approve(&signed.device_id);
        signed
    }

    /// A sync store over the server under this device's session.
    fn store(&self, server: &FakeServer) -> Store {
        Store::over(
            &server.url,
            SessionResolver::Memory(self.connections.clone()),
        )
        .unwrap()
    }

    /// The lockbox `id` as the server hands it to this device: the key its
    /// master makes, and the descriptor that names it.
    fn lockbox(&self, id: &str) -> (VaultKey, String) {
        let lockboxes = block_on(self.client.lockboxes()).unwrap();
        let lockbox = lockboxes.into_iter().find(|l| l.id == id).unwrap();
        let master = self
            .key
            .unseal_master(lockbox.wrapped_master.as_deref().unwrap())
            .unwrap();
        (VaultKey::from_material(master, true), lockbox.kdf)
    }

    /// A device of this member holding lockbox `id`, as adding it leaves it:
    /// the server's descriptor in the sidecar, the vault id the lockbox id.
    fn device(&self, name: &'static str, id: &str) -> Device {
        let (key, kdf) = self.lockbox(id);
        Device::with_key(name, id, key, &kdf)
    }

    fn access_token(&self) -> String {
        self.connections.lock().unwrap()[0].access_token.clone()
    }

    fn sync(
        &self,
        server: &FakeServer,
        device: &Device,
    ) -> crate::error::Result<engine::SyncOutcome> {
        engine::sync(&device.remote(self.store(server)), device, NOW, || Ok(()))
    }
}

// (a) The code flow and a refresh: the two calls that make a session.
#[test]
fn a_code_is_exchanged_for_a_session_and_the_session_refreshed() {
    let server = FakeServer::start();
    let (client, session) = Signed::sign_in(&server);
    let me = block_on(client.me()).unwrap();
    assert_eq!(me.member.email, MEMBER_EMAIL);
    assert_eq!(me.tenant.name, TENANT);

    server.expire_sessions();
    match block_on(client.me()) {
        Err(Error::Other(why)) => assert!(why.contains("sign in again"), "{why}"),
        other => panic!("an expired session must say so, got {other:?}"),
    }
    let fresh = block_on(
        ServerClient::new(&server.url)
            .unwrap()
            .refresh(session.refresh_token.as_deref().unwrap()),
    )
    .unwrap();
    assert_ne!(fresh.access_token, session.access_token);
    let client = ServerClient::with_session(&server.url, &fresh.access_token).unwrap();
    assert_eq!(block_on(client.me()).unwrap().member.id, MEMBER_ID);
    // A refresh token is spent by its refresh.
    assert!(block_on(
        ServerClient::new(&server.url)
            .unwrap()
            .refresh(session.refresh_token.as_deref().unwrap())
    )
    .is_err());
    // And a bad verifier never gets a session at all.
    assert!(block_on(
        ServerClient::new(&server.url)
            .unwrap()
            .token("no-such-code", "v", REDIRECT)
    )
    .is_err());
}

// (a') A refresh that rotates the tokens, then a change of the device's
// standing: every write is a read-modify-write of the record as it stands,
// so the second keeps what the first rotated rather than putting back the
// copy it read beforehand.
#[test]
fn a_rotated_session_survives_a_later_change_to_the_record() {
    let server = FakeServer::start();
    let pat = Signed::enroll(&server, "Pat's Mac");
    let resolver = SessionResolver::Memory(pat.connections.clone());
    let before = resolver.connection(&server.url).unwrap();
    assert_eq!(before.device_state, Some(DeviceState::Pending));

    server.expire_sessions();
    let listed =
        block_on(resolver.as_member(&server.url, |client| async move { client.devices().await }))
            .unwrap();
    assert_eq!(listed.len(), 1, "served under the refreshed session");

    // Noted against the snapshot read before the refresh, as
    // `org_connections` notes it: the same member's same device, so it lands.
    let noted = resolver
        .update(
            &server.url,
            &crate::org::connections::Expect::sign_in(&before),
            |held| held.device_state = Some(DeviceState::Approved),
        )
        .unwrap();
    let crate::org::connections::Updated::Changed(noted) = noted else {
        panic!("the refresh was this sign-in's own, so the note lands")
    };
    assert_eq!(noted.device_state, Some(DeviceState::Approved));
    assert_ne!(noted.access_token, before.access_token);
    assert_ne!(
        noted.refresh_token, before.refresh_token,
        "the rotation stands"
    );
    // And the record is what the next call runs under.
    assert!(
        block_on(resolver.as_member(&server.url, |client| async move { client.me().await }))
            .is_ok()
    );
}

// (b) A new device is pending, is handed nothing, and is approved from a
// device that already is — after which the master it is handed opens.
#[test]
fn enrollment_is_pending_until_an_approved_device_says_yes() {
    let server = FakeServer::start();
    let first = Signed::approved(&server, "Pat's Mac");
    let lockbox = server.create_lockbox("Deploy");

    let second = Signed::enroll(&server, "Pat's phone");
    let listed = block_on(second.client.devices()).unwrap();
    assert_eq!(listed.len(), 2);
    let mine = listed.iter().find(|d| d.id == second.device_id).unwrap();
    assert_eq!(mine.state, DeviceState::Pending);
    assert_eq!(mine.name, "Pat's phone");
    assert_eq!(mine.public_key, second.key.public_key_b64());
    assert_eq!(
        listed
            .iter()
            .find(|d| d.id == first.device_id)
            .unwrap()
            .state,
        DeviceState::Approved
    );

    // Pending: the lockbox is listed, the master is not.
    let before = block_on(second.client.lockboxes()).unwrap();
    assert_eq!(before[0].id, lockbox);
    assert_eq!(before[0].name, "Deploy");
    assert!(before[0].wrapped_master.is_none());
    assert!(before[0].etag.is_none(), "no pack yet");
    // Nor may a pending device approve anyone, itself included.
    assert!(block_on(second.client.approve_device(&second.device_id)).is_err());

    block_on(first.client.approve_device(&second.device_id)).unwrap();
    let after = block_on(second.client.lockboxes()).unwrap();
    let wrapped = after[0].wrapped_master.as_deref().unwrap();
    assert_eq!(
        &*second.key.unseal_master(wrapped).unwrap(),
        &*server.master_of(&lockbox)
    );
    assert_eq!(after[0].kdf, server.kdf_of(&lockbox));
    assert!(KdfParams::from_json(&after[0].kdf).unwrap().is_external());
    // Wrapped to the second device only: the first cannot open its copy.
    assert!(first.key.unseal_master(wrapped).is_err());
}

// (c) Two devices of one member hold an org lockbox through the server, a
// push that loses a race is refused and retried, and an expired session is
// refreshed on the way.
#[test]
fn two_devices_converge_through_the_server_and_a_stale_put_is_retried() {
    let server = FakeServer::start();
    let pat = Signed::approved(&server, "Pat's Mac");
    let lockbox = server.create_lockbox("Deploy");

    let a = pat.device("a", &lockbox);
    a.write("1", "first");
    let outcome = pat.sync(&server, &a).unwrap();
    assert!(outcome.pushed, "the first run creates the pack");
    let listed = block_on(pat.store(&server).list_vaults()).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].handle, lockbox);
    assert_eq!(listed[0].name, format!("{lockbox}.rowel"));
    assert!(listed[0].revision.is_some());

    let b = pat.device("b", &lockbox);
    let outcome = pat.sync(&server, &b).unwrap();
    assert!(!outcome.pushed && outcome.merged == 1);
    assert_eq!(b.title("1").as_deref(), Some("first"));
    assert_eq!(a.digest(), b.digest());

    // A push from A, then B's write under the revision B pulled: another
    // writer lands between B's head check and its PUT, the server answers
    // 412, the run fails with the retry message, and the next run merges.
    a.write("2", "from a");
    assert!(pat.sync(&server, &a).unwrap().pushed);
    b.write("3", "from b");
    server.bump_on_next_put(&lockbox);
    match pat.sync(&server, &b) {
        Err(Error::Other(why)) => assert!(why.contains("sync will retry"), "{why}"),
        other => panic!("a stale PUT must fail, got {other:?}"),
    }
    assert!(pat.sync(&server, &b).unwrap().pushed);
    assert!(pat.sync(&server, &a).unwrap().merged >= 1);
    assert_eq!(a.title("3").as_deref(), Some("from b"));
    assert_eq!(b.title("2").as_deref(), Some("from a"));
    assert_eq!(a.digest(), b.digest());

    // The engine's own retry: A pushes while B is between its pull and its
    // push, B's pre-flight sees the head move, and B starts over from A's
    // state rather than failing.
    b.write("4", "b again");
    let remote = b.remote(pat.store(&server));
    let outcome = engine::sync(&remote, &b, NOW, || {
        a.write("5", "a meanwhile");
        pat.sync(&server, &a).map(|_| ())
    })
    .unwrap();
    assert!(outcome.pushed);
    assert_eq!(b.title("5").as_deref(), Some("a meanwhile"));
    assert!(pat.sync(&server, &a).unwrap().merged >= 1);
    assert_eq!(a.digest(), b.digest());

    // An access token that stopped working mid-way: the store refreshes once,
    // keeps the new session, and the run goes through.
    let before = pat.access_token();
    server.expire_sessions();
    a.write("6", "after expiry");
    assert!(pat.sync(&server, &a).unwrap().pushed);
    assert_ne!(pat.access_token(), before, "the refreshed session was kept");
    assert!(pat.sync(&server, &b).unwrap().merged >= 1);

    // A pack is never written unconditionally: an entry with no revision to
    // match on is refused before anything is sent, and the engine's next run
    // pulls one. (The fake answers a blind PUT with 428, so the pushes above
    // prove `If-Match` goes out on every update.)
    for revision in [None, Some(String::new())] {
        let blind = crate::sync::store::RemoteEntry {
            handle: lockbox.clone(),
            name: crate::sync::layout::vault_file_name(&lockbox),
            size: None,
            modified: String::new(),
            revision,
        };
        match block_on(pat.store(&server).update(&blind, b"not sent")) {
            Err(Error::Other(why)) => assert!(why.contains("no revision"), "{why}"),
            other => panic!("a PUT without If-Match must not go out, got {other:?}"),
        }
    }

    // A device never deletes an org lockbox.
    match block_on(pat.store(&server).delete(&lockbox)) {
        Err(Error::Unsupported(why)) => assert!(why.contains("never from a device"), "{why}"),
        other => panic!("a delete must be refused, got {other:?}"),
    }
    // And a pack that is not there is not there, whatever is asked.
    assert!(block_on(pat.store(&server).stat("deadbeef.rowel"))
        .unwrap()
        .is_none());
    assert!(
        block_on(pat.store(&server).stat(&format!("{lockbox}.deleted")))
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        block_on(pat.store(&server).read("deadbeef", 1024)),
        Err(Error::NotFound)
    ));
}

// (d) The admin rotates the master: one device re-keys onto the new
// descriptor as a password change would, the pack a slower device pushed
// under the old epoch is still opened and carried forward, and that device
// is then told the key changed and re-keys from the server in turn.
#[test]
fn a_rotated_master_reaches_every_device_like_a_password_change() {
    let server = FakeServer::start();
    let pat = Signed::approved(&server, "Pat's Mac");
    let lockbox = server.create_lockbox("Deploy");
    let gen1 = server.kdf_of(&lockbox);
    assert_eq!(KdfParams::from_json(&gen1).unwrap().gen(), 1);

    let mut a = pat.device("a", &lockbox);
    a.write("1", "first");
    pat.sync(&server, &a).unwrap();
    let mut b = pat.device("b", &lockbox);
    pat.sync(&server, &b).unwrap();

    // The rotation: A learns of it first and re-keys. The old epoch is
    // retired under the new key, the sidecar is the server's new descriptor.
    server.rotate_master(&lockbox);
    let gen2 = server.kdf_of(&lockbox);
    assert_eq!(KdfParams::from_json(&gen2).unwrap().gen(), 2);
    assert_ne!(gen2, gen1);
    let (new_key, descriptor) = pat.lockbox(&lockbox);
    assert_eq!(descriptor, gen2);
    a.rekey(new_key, &descriptor);
    assert_eq!(a.gen(), 2);
    assert_eq!(a.sidecar(), gen2);
    assert_eq!(a.retired(), std::slice::from_ref(&gen1));

    // B, still on the old master, pushes under it. A opens that pack with
    // the retired key, merges it, and re-seals the account under the new one.
    b.write("2", "from b, old epoch");
    assert!(pat.sync(&server, &b).unwrap().pushed);
    let outcome = pat.sync(&server, &a).unwrap();
    assert!(outcome.pushed && outcome.rekeyed_remote);
    assert_eq!(a.title("2").as_deref(), Some("from b, old epoch"));

    // B is now the slower device: told the key changed, it takes the
    // server's new master and descriptor, and converges.
    assert!(matches!(
        pat.sync(&server, &b),
        Err(Error::PasswordChangedRemotely)
    ));
    let (new_key, descriptor) = pat.lockbox(&lockbox);
    b.rekey(new_key, &descriptor);
    assert_eq!(b.sidecar(), a.sidecar());
    assert_eq!(b.retired(), [gen1]);
    // B's re-key re-sealed its rows under the new key: each side now holds a
    // copy of every row sealed afresh (new nonce, same stamp), and the
    // tie-break on the row hash picks one copy for both once each has pulled.
    pat.sync(&server, &b).unwrap();
    pat.sync(&server, &a).unwrap();
    assert_eq!(a.digest(), b.digest());

    // A fresh device joins under the new master alone and sees everything.
    let c = pat.device("c", &lockbox);
    pat.sync(&server, &c).unwrap();
    assert_eq!(c.title("1").as_deref(), Some("first"));
    assert_eq!(c.title("2").as_deref(), Some("from b, old epoch"));
    assert_eq!(c.digest(), a.digest());
}

/// An agent's ask for the Stripe key in one command, as the server queues it.
#[cfg(desktop)]
fn env_approval(id: &str, expires_at_ms: i64) -> super::client::Approval {
    super::client::Approval {
        id: id.into(),
        lockbox_id: "lb".into(),
        lockbox_name: "Deploy".into(),
        agent_name: "release-bot".into(),
        mode: rowel_broker::grant::Mode::Env,
        targets: vec![crate::events::AskTarget::Command {
            line: "stripe balance retrieve".into(),
        }],
        candidates: vec![rowel_broker::grant::VisibleItem {
            id: "stripe".into(),
            title: "Stripe".into(),
            username: None,
        }],
        reason: Some("deploying 1.2".into()),
        step_up: false,
        expires_at_ms,
    }
}

// (e) An agent's ask reaches the member over the stream — one already
// waiting and one that arrives while the stream is open, with the keepalive
// between them handed over as an empty frame — and the answer posted back
// is what the server records.
#[cfg(desktop)]
#[test]
fn an_approval_streams_to_the_member_and_the_answer_is_recorded() {
    use super::client::Approval;
    use rowel_broker::agents::GrantAnswer;

    let server = FakeServer::start();
    let pat = Signed::approved(&server, "Pat's Mac");

    server.push_approval(env_approval("ap-1", NOW + 120_000));
    let server = Arc::new(server);
    let pushes = server.clone();
    let streamed = block_on(async move {
        let mut seen = Vec::new();
        let mut keepalives = 0;
        pat.client
            .approvals_stream(|frame| {
                let Some(approval) = frame else {
                    keepalives += 1;
                    return true;
                };
                // The second arrives while the stream is open.
                if approval.id == "ap-1" {
                    pushes.push_approval(approval_two());
                }
                seen.push(approval);
                seen.len() < 2
            })
            .await
            .unwrap();
        (pat, seen, keepalives)
    });
    let (pat, seen, keepalives) = streamed;
    assert_eq!(keepalives, 1, "the comment the stream opens with");
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0], env_approval("ap-1", NOW + 120_000));
    assert_eq!(seen[1].id, "ap-2");

    let yes = GrantAnswer {
        item_id: "stripe".into(),
        standing_days: Some(7),
    };
    block_on(pat.client.answer("ap-1", Some(yes.clone()))).unwrap();
    block_on(pat.client.answer("ap-2", None)).unwrap();
    assert_eq!(
        server.answers(),
        [("ap-1".to_string(), Some(yes)), ("ap-2".to_string(), None)]
    );

    fn approval_two() -> Approval {
        Approval {
            id: "ap-2".into(),
            lockbox_id: "lb".into(),
            lockbox_name: "Deploy".into(),
            agent_name: "release-bot".into(),
            mode: rowel_broker::grant::Mode::Fill,
            targets: vec![crate::events::AskTarget::Origin {
                origin: "https://dashboard.stripe.com".into(),
            }],
            candidates: Vec::new(),
            reason: None,
            step_up: true,
            expires_at_ms: NOW + 120_000,
        }
    }
}

// (f) The approvals loop as the app runs it, with the dialog stood in for by
// a closure: an approval that arrives is put to it and its answer posted; one
// it leaves unanswered posts nothing; a session the server stopped accepting
// is refreshed and the stream resumed; and the stop flag ends the loop.
#[cfg(desktop)]
#[test]
fn the_approvals_loop_posts_what_the_person_decides_and_stops_when_told() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use rowel_broker::agents::{GrantAnswer, Verdict};

    let server = FakeServer::start();
    let pat = Signed::approved(&server, "Pat's Mac");
    let resolver = SessionResolver::Memory(pat.connections.clone());
    let stop = Arc::new(AtomicBool::new(false));
    let asked = Arc::new(Mutex::new(Vec::new()));
    let live = chrono::Utc::now().timestamp_millis() + 120_000;

    let yes = GrantAnswer {
        item_id: "stripe".into(),
        standing_days: None,
    };
    let loop_thread = {
        let (url, stop, asked, yes) =
            (server.url.clone(), stop.clone(), asked.clone(), yes.clone());
        std::thread::spawn(move || {
            super::approvals::follow(&resolver, &url, &stop, |approval| {
                asked.lock().unwrap().push(approval.id.clone());
                match approval.id.as_str() {
                    "yes" => Verdict::Answered(Some(yes.clone())),
                    "no" => Verdict::Answered(None),
                    _ => Verdict::Unanswered,
                }
            })
        })
    };
    let wait_for = |what: &str, check: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !check() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };

    server.push_approval(env_approval("yes", live));
    server.push_approval(env_approval("no", live));
    server.push_approval(env_approval("shrug", live));
    wait_for("three asks", &|| asked.lock().unwrap().len() == 3);
    wait_for("two answers", &|| server.answers().len() == 2);
    assert_eq!(
        server.answers(),
        [
            ("yes".to_string(), Some(yes.clone())),
            ("no".to_string(), None)
        ]
    );

    // An ask the server has already decided never reaches the person.
    server.push_approval(env_approval("stale", live - 240_000));
    server.push_approval(env_approval("after", live));
    wait_for("the live ask", &|| {
        asked.lock().unwrap().contains(&"after".to_string())
    });
    assert!(!asked.lock().unwrap().contains(&"stale".to_string()));

    // The access token stops working mid-stream: the next ask is posted under
    // a refreshed session rather than lost.
    let before = pat.access_token();
    server.expire_sessions();
    server.push_approval(env_approval("yes", live));
    wait_for("the answer after the refresh", &|| {
        server.answers().len() == 3
    });
    assert_ne!(pat.access_token(), before, "the refreshed session was kept");

    stop.store(true, Ordering::SeqCst);
    loop_thread.join().unwrap();
}

// (g) The server's token file is one more of the provider's: found by name
// while locked, read back with the server it names, and nothing else in it.
#[test]
fn the_server_token_file_round_trips_like_any_providers() {
    let dir = tempfile::tempdir().unwrap();
    let key = VaultKey::from_material(Zeroizing::new(vec![5u8; 32]), true);
    let cryptor = key.cryptor();
    let tokens = crate::sync::Tokens {
        provider: Provider::RowelServer,
        account: Some("https://rowel.acme.example".into()),
        ..Default::default()
    };
    crate::sync::persist_tokens_in(dir.path(), &cryptor, &tokens).unwrap();

    assert_eq!(
        crate::storage::token_file_in(dir.path()),
        Some(Provider::RowelServer)
    );
    assert_eq!(
        crate::storage::sync_provider_in(dir.path()),
        Some(Provider::RowelServer)
    );
    let read = crate::sync::read_tokens_in(dir.path(), &cryptor)
        .unwrap()
        .unwrap();
    assert_eq!(read.provider, Provider::RowelServer);
    assert_eq!(read.account.as_deref(), Some("https://rowel.acme.example"));
    assert!(read.access_token.is_none() && read.refresh_token.is_none());

    // Connecting a drive afterwards retires it, as between any two providers.
    let drive = crate::sync::Tokens {
        provider: Provider::Dropbox,
        access_token: Some("t".into()),
        ..Default::default()
    };
    crate::sync::persist_tokens_in(dir.path(), &cryptor, &drive).unwrap();
    assert_eq!(
        crate::storage::token_file_in(dir.path()),
        Some(Provider::Dropbox)
    );
    assert!(!dir
        .path()
        .join(Provider::RowelServer.token_file_name())
        .exists());
}
