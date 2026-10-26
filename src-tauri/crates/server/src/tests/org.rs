//! A tenant with one org lockbox for agents: `agents` on, an API key and a
//! login sealed in it, an agent definition bound to it with two GitHub-shaped
//! trust policies at the fake issuer — the deploy workflow as the agent, and
//! the proxy's own reusable workflow as its sidecar — an approver and a
//! reader each with an approved device. The host is driven as the broker's connection drives
//! it, in-process, through the CLI's own peer.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use rowel_broker::actions::{ClientKind, Connection};
use rowel_broker::client::{Peer, PeerError};
use rowel_broker::grant::{Approval as Gate, CreatedBy, ItemSelector, Mode, Target};
use rowel_broker::policy::{AgentSelector, Rule};
use rowel_broker::protocol::Code;
use rowel_core::models::Entry;
use rowel_core::store::AuditRecord;

use crate::config::{Config, Listen, Members, Tenant, DEFAULT_SESSION_TTL_MS};
use crate::host::OrgHost;
use crate::identity::testing::FakeIdp;
use crate::identity::workload::expected_audience;
use crate::lockboxes;
use crate::state::{now_ms, AppState};
use crate::store::{Agent, Device, Lockbox, LockboxRole, Member, Role};
use crate::testing::{TempServer, TENANT};

pub(super) const STRIPE: &str = "stripe-item";
pub(super) const GITHUB: &str = "github-item";
pub(super) const STRIPE_KEY: &str = "sk_live_123456789";
pub(super) const KEY_IDLE_MS: u64 = 600_000;

pub(super) type AgentPeer = Peer<Connection<OrgHost>>;

pub(super) struct Org {
    pub server: TempServer,
    pub state: Arc<AppState>,
    pub idp: FakeIdp,
    pub lockbox: Lockbox,
    pub agent: Agent,
    pub approver: (Member, Device),
    pub reader: (Member, Device),
    // Last, so the fake issuer's task is aborted while its runtime stands.
    pub runtime: tokio::runtime::Runtime,
}

impl Org {
    /// The tenant, with asks given up on after `ask_timeout_ms`.
    pub fn new(ask_timeout_ms: u64) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let idp = runtime.block_on(FakeIdp::start());
        let server = TempServer::new();
        let state = runtime
            .block_on(AppState::with_config(
                config(&server, ask_timeout_ms),
                &server.data_dir(),
            ))
            .unwrap();
        let lockbox = server.lockbox("Deploy");
        let store = &state.store;
        store.set_agents(&lockbox.id, true).unwrap();
        let lockbox = store.lockbox(&lockbox.id).unwrap().unwrap();
        seed(&state, &lockbox.id);

        let agent = store.add_agent("release-bot", &lockbox.id).unwrap();
        store
            .add_trust_policy(
                &agent.id,
                idp.issuer(),
                deploy_policy(),
                false,
                ClientKind::Agent,
            )
            .unwrap();
        store
            .add_trust_policy(
                &agent.id,
                idp.issuer(),
                sidecar_policy(),
                false,
                ClientKind::Sidecar,
            )
            .unwrap();
        let approver = server.member("ana@acme.example", Role::Member);
        let (approver_device, _) = server.approved_device(&approver);
        store
            .add_lockbox_member(&lockbox.id, &approver.id, LockboxRole::Approver)
            .unwrap();
        let reader = server.member("rex@acme.example", Role::Member);
        let (reader_device, _) = server.approved_device(&reader);
        store
            .add_lockbox_member(&lockbox.id, &reader.id, LockboxRole::Reader)
            .unwrap();
        Self {
            server,
            state,
            idp,
            lockbox,
            agent,
            approver: (approver, approver_device),
            reader: (reader, reader_device),
            runtime,
        }
    }

    pub fn host(&self) -> OrgHost {
        OrgHost::new(self.state.clone(), self.runtime.handle().clone())
    }

    /// A token for the deploy workflow, minted for `audience`.
    pub fn token_for(&self, audience: &str) -> String {
        let now = now_ms() / 1000;
        self.idp.mint(deploy_claims(), audience, now, now + 600)
    }

    /// A token bound to the channel opened with `session_key`.
    pub fn token(&self, session_key: &str) -> String {
        self.bound_token(deploy_claims(), session_key)
    }

    /// A token for the proxy's job, bound to the channel opened with
    /// `session_key`.
    pub fn sidecar_token(&self, session_key: &str) -> String {
        self.bound_token(sidecar_claims(), session_key)
    }

    fn bound_token(&self, claims: Map<String, Value>, session_key: &str) -> String {
        let tenant = self.state.tenant_audience().unwrap();
        let audience = expected_audience(&tenant, session_key).unwrap();
        let now = now_ms() / 1000;
        self.idp.mint(claims, &audience, now, now + 600)
    }

    /// A channel to a fresh host, keys exchanged, nobody let in yet.
    pub fn connect(&self) -> AgentPeer {
        Peer::connect(Connection::new(self.host()), "rowel-cli").unwrap()
    }

    /// A channel let in as `kind` under `id_key`, with a token of that
    /// kind's workload minted for it.
    pub fn associated(&self, id_key: &str, kind: &str) -> AgentPeer {
        let mut peer = self.connect();
        let token = match kind {
            "sidecar" => self.sidecar_token(&peer.public_key()),
            _ => self.token(&peer.public_key()),
        };
        associate(&mut peer, id_key, &hello(kind, Some(&token))).unwrap();
        peer
    }

    /// A rule of the admin's for this lockbox, over the API key at Stripe.
    pub fn stripe_rule(&self, gate: Gate) {
        let rule = Rule {
            id: format!("admin-{}", now_ms()),
            agents: AgentSelector::Ids(vec![self.agent.id.clone()]),
            items: ItemSelector::Ids(vec![STRIPE.into()]),
            targets: vec![Target::Http {
                host: "api.stripe.com".into(),
                path: "/v1/*".into(),
                methods: vec![],
            }],
            modes: vec![Mode::Header],
            approval: gate,
            ttl_ms: 15 * 60 * 1000,
            max_uses: None,
            reason_required: false,
            approvers: None,
            sidecar_for: None,
            created_at_ms: now_ms(),
            created_by: CreatedBy::Admin("test".into()),
            expires_at_ms: None,
        };
        self.state
            .store
            .add_rule(Some(&self.lockbox.id), &rule)
            .unwrap();
    }

    /// Every row in the lockbox's own log, through a connection of the
    /// test's own.
    pub fn rows(&self) -> Vec<AuditRecord> {
        lockboxes::open(
            &self.state.store,
            &*self.state.root,
            &self.state.data_dir,
            &self.lockbox.id,
        )
        .unwrap()
        .store
        .audit_all()
        .unwrap()
    }

    /// The lockbox's log as `(event, detail)`, leaving out the unwraps.
    pub fn events(&self) -> Vec<(String, Option<String>)> {
        self.rows()
            .into_iter()
            .filter(|row| row.event != "key_unwrapped")
            .map(|row| (row.event, row.detail))
            .collect()
    }

    /// The state column of the recorded ask `id`.
    pub fn pending_state(&self, id: &str) -> String {
        let db =
            rusqlite::Connection::open(self.server.data_dir().join(crate::store::DB_FILE)).unwrap();
        db.query_row(
            "SELECT state FROM pending_approvals WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap()
    }

    /// Wait for the one ask routed to the approver, and hand its id back.
    pub fn routed_ask(&self) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let pending = self
                .state
                .approvals
                .pending_for(&self.state.store, &self.approver.0.id);
            if let Some(ask) = pending.first() {
                return ask.id.clone();
            }
            assert!(Instant::now() < deadline, "no ask was routed");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn config(server: &TempServer, ask_timeout_ms: u64) -> Config {
    Config {
        tenant: Tenant {
            name: TENANT.into(),
        },
        public_url: "http://127.0.0.1:7402".into(),
        listen: Listen::default(),
        root_key_file: server.dir.path().join("root.key"),
        members: Members::default(),
        ask_timeout_ms,
        key_idle_ms: KEY_IDLE_MS,
        session_ttl_ms: DEFAULT_SESSION_TTL_MS,
    }
}

/// The API key and a login, sealed into the lockbox's file as the app
/// writes items.
fn seed(state: &AppState, lockbox_id: &str) {
    let opened = lockboxes::open(&state.store, &*state.root, &state.data_dir, lockbox_id).unwrap();
    let cipher = opened.key.payload_cipher();
    for entry in [
        Entry {
            id: STRIPE.into(),
            kind: "apikey".into(),
            title: "Stripe (live)".into(),
            api_key: Some(STRIPE_KEY.into()),
            ..Default::default()
        },
        Entry {
            id: GITHUB.into(),
            kind: "login".into(),
            title: "GitHub".into(),
            website: Some("https://github.com".into()),
            username: Some("octocat".into()),
            password: Some("hunter2".into()),
            ..Default::default()
        },
    ] {
        rowel_broker::testing::seed(&opened.store, &cipher, &entry);
    }
}

fn deploy_policy() -> BTreeMap<String, String> {
    [
        ("repository", "acme/deploy"),
        ("ref", "refs/heads/main"),
        ("workflow", "release.yml"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

pub(super) fn deploy_claims() -> Map<String, Value> {
    FakeIdp::github_claims("acme/deploy", "refs/heads/main", "release.yml")
}

/// The reusable workflow the proxy's job runs, which is what tells its
/// token apart from the deploy job's in the same run.
const PROXY_WORKFLOW: &str = "acme/deploy/.github/workflows/rowel-proxy.yml@refs/heads/main";

fn sidecar_policy() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("repository".to_string(), "acme/deploy".to_string()),
        ("job_workflow_ref".to_string(), PROXY_WORKFLOW.to_string()),
    ])
}

fn sidecar_claims() -> Map<String, Value> {
    let mut claims = deploy_claims();
    claims.insert("job_workflow_ref".into(), json!(PROXY_WORKFLOW));
    claims
}

/// What the CLI says it is, with `token` as its proof when there is one.
pub(super) fn hello(kind: &str, token: Option<&str>) -> Value {
    let mut hello = json!({
        "kind": kind,
        "platform": "github-actions",
        "version": "1.2.0",
        "capabilities": ["grants", "placeholders"],
    });
    if let Some(token) = token {
        hello["proof"] = json!({ "kind": "oidc", "token": token });
    }
    hello
}

/// The sealed `associate`, as the CLI sends it.
pub(super) fn associate(
    peer: &mut AgentPeer,
    id_key: &str,
    hello: &Value,
) -> Result<Map<String, Value>, PeerError> {
    let key = peer.public_key();
    peer.call(
        "associate",
        json!({ "key": key, "idKey": id_key, "hello": hello }),
    )
}

pub(super) fn header_request(reason: &str) -> Value {
    json!({
        "runId": "run-1",
        "reason": reason,
        "items": { "ids": [STRIPE] },
        "targets": [{ "http": { "host": "api.stripe.com", "path": "/v1/balance", "methods": ["GET"] } }],
        "modes": ["header"],
    })
}

/// The sidecar's `resolve` of the API key under `grant_id`, the lockbox
/// named as `lockbox`.
pub(super) fn resolve_request(grant_id: &Value, lockbox: &str) -> Value {
    json!({
        "grantId": grant_id,
        "ref": format!("rowel://{lockbox}/{STRIPE}/apiKey"),
        "context": { "target": { "http": { "host": "api.stripe.com", "path": "/v1/balance", "method": "GET" } } },
    })
}

pub(super) fn refused(error: PeerError) -> Code {
    match error {
        PeerError::Refused(refused) => refused.code.expect("a known code"),
        other => panic!("not a refusal: {other:?}"),
    }
}

pub(super) fn row(event: &str, detail: Option<&str>) -> (String, Option<String>) {
    (event.into(), detail.map(String::from))
}
