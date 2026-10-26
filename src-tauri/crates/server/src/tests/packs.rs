//! The pack endpoints as the app's Rowel Server provider speaks them, with a
//! device played by the core's halves of the sync exchange: its own copy of
//! the lockbox opened with the master the server sealed to it, pulled with
//! `HEAD` and `GET` and merged, pushed with `PUT` under `If-Match`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use crypto_box::SecretKey;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_MATCH, IF_NONE_MATCH};
use reqwest::{Method, RequestBuilder, Response};
use tempfile::TempDir;
use zeroize::Zeroizing;

use rowel_broker::agents;
use rowel_broker::audit::{verify, ActorKind, Draft, Event};
use rowel_core::crypto::{random_hex_id, KdfParams, PayloadCipher, VaultKey};
use rowel_core::models::Entry;
use rowel_core::store::{identity, AuditRecord, Record, SqliteStore, VaultStore};
use rowel_core::sync::{pack, snapshot};

use super::harness::{json, Api};
use crate::lockboxes;
use crate::state::{now_ms, AppState};
use crate::store::{Lockbox, LockboxRole, Member, Role};

/// A request to lockbox `id`'s pack under `access`.
pub(super) fn pack_request(api: &Api, method: Method, id: &str, access: &str) -> RequestBuilder {
    reqwest::Client::new()
        .request(method, format!("{}/v1/lockboxes/{id}/pack", api.url))
        .bearer_auth(access)
}

fn etag_of(response: &Response) -> String {
    response.headers()[ETAG].to_str().unwrap().to_string()
}

/// `email` as a member holding `lockbox`, made on first use.
fn holder(api: &Api, lockbox: &Lockbox, email: &str) -> Member {
    let store = &api.state.store;
    let member = match store.member_by_email(email).unwrap() {
        Some(member) => member,
        None => api.server.member(email, Role::Member),
    };
    store
        .add_lockbox_member(&lockbox.id, &member.id, LockboxRole::Reader)
        .unwrap();
    member
}

/// A session signed in as `email` and bound to the device whose secret key
/// is `secret`, as the app's session is once it has enrolled.
async fn device_session(api: &Api, email: &str, secret: &SecretKey) -> String {
    let tokens = api.sign_in(email).await;
    let enrolled = api.enroll(&tokens.access, secret).await;
    assert_eq!(enrolled.status().as_u16(), 200);
    tokens.access
}

/// One member's device holding an org lockbox: its own SQLCipher copy under
/// the master the server sealed to it, the descriptor it was listed with, and
/// two revisions, which a push sets apart.
pub(super) struct Device {
    url: String,
    access: String,
    lockbox_id: String,
    kdf: String,
    /// The audit writer this device's rows are under.
    pub writer: String,
    pub key: VaultKey,
    pub store: SqliteStore,
    /// The revision a push is conditional on: the one it last pulled at or
    /// pushed to.
    pub etag: Option<String>,
    /// The revision whose pack it last merged. A push lands at a revision
    /// whose pack is the server's merge, which can differ from what was
    /// pushed (a row of the push lost to one the server held), so only a
    /// pull tells the device what that revision holds.
    merged: Option<String>,
    cipher: PayloadCipher,
    scratch: PathBuf,
    _dir: TempDir,
}

impl Device {
    /// A new approved device of `email`'s, who is made to hold `lockbox`:
    /// signed in, enrolled, handed the master by the listing, its copy made
    /// as the app makes one for an org lockbox (the lockbox id as its vault
    /// id) and pulled once.
    pub async fn join(api: &Api, lockbox: &Lockbox, email: &str) -> Self {
        let member = holder(api, lockbox, email);
        let (_, secret) = api.server.approved_device(&member);
        let access = device_session(api, email, &secret).await;
        let listing = json(
            api.call(Method::GET, "/v1/members/me/lockboxes", Some(&access), None)
                .await,
        )
        .await;
        let listed = listing
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == lockbox.id.as_str())
            .unwrap();
        let sealed = STANDARD
            .decode(listed["wrappedMaster"].as_str().unwrap())
            .unwrap();
        let key = VaultKey::from_material(Zeroizing::new(secret.unseal(&sealed).unwrap()), true);
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("vault.db"), &*key.sqlcipher_key()).unwrap();
        identity::adopt_vault_id(&store, &lockbox.id).unwrap();
        let mut joined = Self {
            url: api.url.clone(),
            access,
            lockbox_id: lockbox.id.clone(),
            kdf: listed["kdf"].as_str().unwrap().to_string(),
            writer: random_hex_id(),
            cipher: key.payload_cipher(),
            key,
            store,
            etag: None,
            merged: None,
            scratch: dir.path().join("scratch"),
            _dir: dir,
        };
        assert!(joined.pull().await);
        joined
    }

    fn request(&self, method: Method) -> RequestBuilder {
        reqwest::Client::new()
            .request(
                method,
                format!("{}/v1/lockboxes/{}/pack", self.url, self.lockbox_id),
            )
            .bearer_auth(&self.access)
    }

    /// The revision the server is at now.
    pub async fn head(&self) -> String {
        let reply = self.request(Method::HEAD).send().await.unwrap();
        assert_eq!(reply.status().as_u16(), 200);
        etag_of(&reply)
    }

    /// The device's pull: a `HEAD`, and when it is not the revision whose
    /// pack this device last merged, the `GET` opened and merged in, the
    /// revision taken from the `HEAD` as the app's engine takes it — and
    /// again while the revision keeps moving under the pull, as the engine's
    /// pre-flight re-pulls. Whether it pulled anything.
    pub async fn pull(&mut self) -> bool {
        const MAX_PULLS: usize = 3;
        for pulls in 0..=MAX_PULLS {
            let seen = self.head().await;
            if self.merged.as_deref() == Some(seen.as_str()) {
                return pulls > 0;
            }
            assert!(
                pulls < MAX_PULLS,
                "the revision kept moving under {MAX_PULLS} pulls"
            );
            let reply = self.request(Method::GET).send().await.unwrap();
            assert_eq!(reply.status().as_u16(), 200);
            let bytes = reply.bytes().await.unwrap();
            let unpacked = pack::unpack(&bytes).unwrap();
            assert_eq!(unpacked.kdf_params_json, self.kdf);
            let contents = snapshot::read(
                &unpacked.snapshot,
                &*self.key.sqlcipher_key(),
                &self.scratch,
            )
            .unwrap();
            self.store
                .merge(&contents.records, &contents.attachments, &contents.audit)
                .unwrap();
            self.merged = Some(seen.clone());
            self.etag = Some(seen);
        }
        unreachable!("the last round either returns or fails its assertion")
    }

    /// This device's copy packed under `kdf`, as a push sends it.
    pub fn pack_under(&self, kdf: &str) -> Vec<u8> {
        pack::pack_store(&self.store, &*self.key.sqlcipher_key(), kdf, &self.scratch).unwrap()
    }

    /// One `PUT` of this device's copy under `If-Match` on the revision it
    /// holds; on 200 it holds the new one.
    pub async fn put(&mut self) -> u16 {
        let reply = self
            .request(Method::PUT)
            .header(IF_MATCH, self.etag.clone().unwrap())
            .body(self.pack_under(&self.kdf))
            .send()
            .await
            .unwrap();
        let status = reply.status().as_u16();
        if status == 200 {
            self.etag = Some(etag_of(&reply));
        }
        status
    }

    /// The device's push: a `PUT`, and on 412 a pull and one more `PUT`.
    /// The statuses in order.
    pub async fn push(&mut self) -> Vec<u16> {
        let first = self.put().await;
        if first != 412 {
            return vec![first];
        }
        assert!(
            self.pull().await,
            "a 412 under a revision that did not move"
        );
        vec![first, self.put().await]
    }

    /// An item saved on this device, sealed as the app seals one.
    pub fn save(&self, id: &str, title: &str) {
        rowel_broker::testing::seed(
            &self.store,
            &self.cipher,
            &Entry {
                id: id.into(),
                kind: "login".into(),
                title: title.into(),
                username: Some("octocat".into()),
                password: Some("hunter2".into()),
                ..Default::default()
            },
        );
    }

    /// A row of this device's own chain, as its app writes one.
    pub fn log(&self, event: Event) {
        agents::record(
            &self.store,
            &self.writer,
            Draft::new(ActorKind::Human, event),
            now_ms(),
        );
    }

    pub fn entries(&self) -> Vec<Record> {
        self.store.export_for_sync().unwrap()
    }
}

/// The server's file through a connection of the test's own.
pub(super) fn server_copy(state: &AppState, lockbox: &Lockbox) -> lockboxes::Opened {
    lockboxes::open(&state.store, &*state.root, &state.data_dir, &lockbox.id).unwrap()
}

/// A row an agent's grant writes, the way the host writes it: under the
/// vault's session lock, mirrored after.
pub(super) fn agent_writes(state: &AppState, lockbox: &Lockbox) {
    let vault = state.vaults.open(state, lockbox).unwrap();
    {
        let session = vault.session();
        agents::record(
            session.store().unwrap(),
            &state.writer_id,
            Draft::new(ActorKind::Agent, Event::Granted),
            now_ms(),
        );
    }
    vault.mirror(state);
}

/// Every writer's chain in `rows`, each verified.
pub(super) fn assert_chains_verify(rows: &[AuditRecord]) {
    let mut chains: BTreeMap<&str, Vec<AuditRecord>> = BTreeMap::new();
    for row in rows {
        chains.entry(&row.writer_id).or_default().push(row.clone());
    }
    for (writer, chain) in chains {
        assert_eq!(verify(&chain), Ok(()), "the chain of {writer}");
    }
}

/// Lockbox "Deploy" on a server with the pack routes.
pub(super) async fn deploy() -> (Api, Lockbox) {
    let api = Api::start().await;
    let lockbox = api.server.lockbox("Deploy");
    (api, lockbox)
}

#[tokio::test]
async fn head_and_get_agree_and_the_pack_opens_under_the_unsealed_master() {
    let (api, lockbox) = deploy().await;
    let device = Device::join(&api, &lockbox, "pat@acme.example").await;

    let head = device.request(Method::HEAD).send().await.unwrap();
    assert_eq!(head.status().as_u16(), 200);
    let got = device.request(Method::GET).send().await.unwrap();
    assert_eq!(got.status().as_u16(), 200);
    assert_eq!(etag_of(&got), etag_of(&head));
    assert_eq!(got.headers()[CONTENT_TYPE], "application/octet-stream");
    let bytes = got.bytes().await.unwrap();
    // The probe's length is the pack's, never the zero of an empty reply.
    assert_eq!(
        head.headers()[CONTENT_LENGTH].to_str().unwrap(),
        bytes.len().to_string()
    );

    let unpacked = pack::unpack(&bytes).unwrap();
    assert_eq!(unpacked.kdf_params_json, lockbox.kdf);
    assert!(KdfParams::from_json(&unpacked.kdf_params_json)
        .unwrap()
        .is_external());
    let contents = snapshot::read(
        &unpacked.snapshot,
        &*device.key.sqlcipher_key(),
        &device.scratch,
    )
    .unwrap();
    assert_eq!(contents.name.as_deref(), Some("Deploy"));
    // The pull unwrapped the master, and that is in the log the device got.
    assert!(contents
        .audit
        .iter()
        .any(|row| row.event == "key_unwrapped" && row.writer_id == api.state.writer_id));
    // The unwrap moved the revision past the lockbox's first.
    assert_ne!(etag_of(&head), lockboxes::etag(lockbox.revision));
    assert_eq!(device.etag.as_deref(), Some(etag_of(&head).as_str()));
}

#[tokio::test]
async fn a_push_under_the_current_etag_merges_into_the_servers_file() {
    let (api, lockbox) = deploy().await;
    let mut device = Device::join(&api, &lockbox, "pat@acme.example").await;
    let before = device.etag.clone().unwrap();
    device.save("gh", "GitHub");
    device.log(Event::Used);

    assert_eq!(device.push().await, [200]);
    let after = device.etag.clone().unwrap();
    assert_ne!(after, before);
    assert_eq!(device.head().await, after);

    let server = server_copy(&api.state, &lockbox);
    assert_eq!(server.store.get("gh").unwrap().unwrap().title, "GitHub");
    assert_eq!(
        server.store.state_digest().unwrap(),
        device.store.state_digest().unwrap()
    );
    // The device's row reached the export's mirror with the merge.
    let mirrored = api
        .state
        .store
        .audit_high_water(&lockbox.id, &device.writer)
        .unwrap();
    assert_eq!(mirrored, 1);
    // The server's name stands, whatever the device's copy says.
    identity::set_vault_name(&device.store, "Renamed", now_ms() + 60_000).unwrap();
    assert_eq!(device.push().await, [200]);
    assert_eq!(
        identity::vault_name(&server_copy(&api.state, &lockbox).store)
            .unwrap()
            .0
            .as_deref(),
        Some("Deploy")
    );

    // A second device's pull carries the entry.
    let other = Device::join(&api, &lockbox, "sam@acme.example").await;
    assert_eq!(other.entries(), device.entries());

    // A push that changes nothing still lands at a revision of its own.
    let landed = device.etag.clone().unwrap();
    assert_eq!(device.push().await, [200]);
    assert_ne!(device.etag.clone().unwrap(), landed);
}

#[tokio::test]
async fn a_put_is_a_compare_and_swap_and_never_blind() {
    let (api, lockbox) = deploy().await;
    let mut device = Device::join(&api, &lockbox, "pat@acme.example").await;
    let body = device.pack_under(&device.kdf);
    let put = |headers: &[(&str, &str)]| {
        let mut request = device.request(Method::PUT).body(body.clone());
        for &(name, value) in headers {
            request = request.header(name, value);
        }
        request.send()
    };

    let stale = put(&[(IF_MATCH.as_str(), "\"0\"")]).await.unwrap();
    assert_eq!(stale.status().as_u16(), 412);
    let blind = put(&[]).await.unwrap();
    assert_eq!(blind.status().as_u16(), 428);
    let create = put(&[(IF_NONE_MATCH.as_str(), "*")]).await.unwrap();
    assert_eq!(create.status().as_u16(), 412);
    let conditional_get = put(&[(IF_NONE_MATCH.as_str(), "\"1\"")]).await.unwrap();
    assert_eq!(conditional_get.status().as_u16(), 400);
    // None of them moved anything.
    assert_eq!(device.head().await, device.etag.clone().unwrap());

    let garbage = device
        .request(Method::PUT)
        .header(IF_MATCH, device.etag.clone().unwrap())
        .body(&b"not a pack at all"[..])
        .send()
        .await
        .unwrap();
    assert_eq!(garbage.status().as_u16(), 400);
    assert!(!garbage.text().await.unwrap().is_empty());

    let delete = device.request(Method::DELETE).send().await.unwrap();
    assert_eq!(delete.status().as_u16(), 405);

    assert_eq!(device.push().await, [200]);
}

#[tokio::test]
async fn a_pack_under_another_descriptor_is_a_conflict_in_plain_words() {
    let (api, lockbox) = deploy().await;
    let device = Device::join(&api, &lockbox, "pat@acme.example").await;
    let rotated = KdfParams::external(random_hex_id(), 2).to_json().unwrap();

    let reply = device
        .request(Method::PUT)
        .header(IF_MATCH, device.etag.clone().unwrap())
        .body(device.pack_under(&rotated))
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status().as_u16(), 409);
    assert_eq!(
        reply.text().await.unwrap(),
        "this pack is under another key generation; refresh the lockbox key"
    );
}

#[tokio::test]
async fn only_an_approved_device_of_a_holder_reaches_a_pack() {
    let (api, lockbox) = deploy().await;
    let device = Device::join(&api, &lockbox, "pat@acme.example").await;
    let pat = holder(&api, &lockbox, "pat@acme.example");

    let (_, pending) = api.server.pending_device(&pat);
    let from_pending = device_session(&api, "pat@acme.example", &pending).await;
    let unenrolled = api.sign_in("pat@acme.example").await.access;
    let sam = api.server.member("sam@acme.example", Role::Member);
    let (_, sams) = api.server.approved_device(&sam);
    let non_member = device_session(&api, "sam@acme.example", &sams).await;

    for method in [Method::HEAD, Method::GET, Method::PUT] {
        let ask = |access: &str, id: &str| {
            pack_request(&api, method.clone(), id, access)
                .header(IF_MATCH, device.etag.clone().unwrap())
                .send()
        };
        assert_eq!(ask(&from_pending, &lockbox.id).await.unwrap().status(), 403);
        assert_eq!(ask(&unenrolled, &lockbox.id).await.unwrap().status(), 403);
        assert_eq!(ask(&non_member, &lockbox.id).await.unwrap().status(), 404);
        assert_eq!(ask(&device.access, "no-such").await.unwrap().status(), 404);
        assert_eq!(ask("not-a-token", &lockbox.id).await.unwrap().status(), 401);
    }
}

#[tokio::test]
async fn a_frozen_lockbox_takes_no_push_and_still_serves_its_pack() {
    let (api, lockbox) = deploy().await;
    let mut device = Device::join(&api, &lockbox, "pat@acme.example").await;
    device.save("gh", "GitHub");
    assert_eq!(device.push().await, [200]);
    api.state.store.set_frozen(&lockbox.id, true).unwrap();

    device.save("aws", "AWS");
    assert_eq!(device.push().await, [423]);
    let at = device.head().await;
    assert_eq!(at, device.etag.clone().unwrap());
    let got = device.request(Method::GET).send().await.unwrap();
    assert_eq!(got.status().as_u16(), 200);
    assert_eq!(etag_of(&got), at);
    // Serving it wrote nothing into it: the revision stands.
    assert_eq!(device.head().await, at);
    let server = server_copy(&api.state, &lockbox);
    assert!(server.store.get("gh").unwrap().is_some());
    assert!(server.store.get("aws").unwrap().is_none());
}

#[tokio::test]
async fn a_server_side_write_moves_the_etag() {
    let (api, lockbox) = deploy().await;
    let device = Device::join(&api, &lockbox, "pat@acme.example").await;
    let before = device.head().await;

    agent_writes(&api.state, &lockbox);
    let after = device.head().await;
    assert_ne!(after, before);
    // Once per pass, however many rows it carried; a pass with none moves
    // nothing.
    api.state
        .vaults
        .get(&lockbox.id)
        .unwrap()
        .mirror(&api.state);
    assert_eq!(device.head().await, after);
}
