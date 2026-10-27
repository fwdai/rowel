//! The Rowel Server as a `Host`: `HeadlessHost` method by method, over the
//! org lockbox an agent definition is bound to rather than one lockbox the
//! process opened, with association by workload proof instead of a person,
//! and the asks routed to members' apps (`approvals`).
//!
//! A host is tenant-wide and made per connection; what a request is about
//! comes from its client: `client.agent_id` → the agent definition → its one
//! lockbox (I7 binds a definition to exactly one) → that lockbox's vault
//! (`vaults`), unwrapped on first use. Clients are looked up again on every
//! request, so disabling a definition ends its sessions at their next one.
//!
//! Locks and the epoch are `HeadlessHost`'s, per vault: session → agents,
//! the approvals queue only inside agents (an ask is queued there) or with
//! both let go, never agents → session; an ask blocks with no lock held. A
//! grant is decided under both locks in one session and settled under them
//! again only if the vault's session counter is still the one it was decided
//! in — an idle seal in between moves it, and the settle is a no.
//!
//! What a remote client cannot have here: a fill (`logins_for`, `totp`,
//! `save_login` — a remote extension is I11), a passkey (Q5: none from
//! server-held lockboxes), a generated password, and a lock: the server is
//! never locked as a whole, and a lockbox seals itself when idle.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::runtime::Handle;
use zeroize::Zeroizing;

use rowel_broker::actions::{Client, ClientKind, Hello, Host, Login, VisibleRef};
use rowel_broker::agents::{
    self, candidates_in, client_covered, covered, decide, deny_unasked, end_grant_in, expire_due,
    reply_for, resolved, settle, visible_in, Asking, GrantEnd, Locked, Prepared, Verdict,
};
use rowel_broker::audit::Event;
use rowel_broker::grant::{GrantReply, GrantRequest, Mode, ResolveContext};
use rowel_broker::hub::Hub;
use rowel_broker::passkeys::{Assertion, Registration};
use rowel_broker::protocol::Code;
use rowel_broker::reference::{self, Reference};
use rowel_broker::Signal;
use rowel_serve::approvals::Answer;

use crate::identity::workload::{self, ReplayCache, TrustPolicy, Verifier};
use crate::state::{now_ms, AppState};
use crate::store::{Agent, Lockbox, Store};
use crate::vaults::{initialise, Vault};

/// How long a client may go unseen before it is forgotten. A CI run mints a
/// key per run and never comes back under it.
const CLIENT_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// The tenant's server as a `Host`. Cheap to clone; one per connection.
#[derive(Clone)]
pub struct OrgHost {
    state: Arc<AppState>,
    /// The runtime a proof's issuer keys are fetched on: the connection's
    /// thread is not one of its workers, so it blocks on the fetch there.
    runtime: Handle,
    /// The tenant id digested, read once: it never changes.
    hash: Option<String>,
    /// The connection this host was made for, and no other. The server is
    /// never locked as a whole and raises no clear signal, so nothing goes
    /// out through it yet; a hub shared wider than one connection would be
    /// one lockbox's, so that its seal reaches its own agents and no other
    /// lockbox's.
    hub: Hub,
}

/// What a client's request is about: its definition, the definition's
/// lockbox as the row says it now, and the lockbox's vault, open.
struct Bound {
    agent: Agent,
    lockbox: Lockbox,
    vault: Arc<Vault>,
}

impl ReplayCache for Store {
    // A store that cannot say whether a `jti` was seen fails the proof:
    // letting a replay through is the worse mistake.
    fn note(&self, jti: &str, expires_at_ms: i64, now_ms: i64) -> bool {
        self.note_jti_at(jti, expires_at_ms, now_ms)
            .unwrap_or_else(|e| {
                log::warn!("rowel server: replay cache: {e}");
                false
            })
    }
}

impl OrgHost {
    pub fn new(state: Arc<AppState>, runtime: Handle) -> Self {
        let hash = match state.store.tenant() {
            Ok(tenant) => Some(hex::encode(Sha256::digest(tenant.id.as_bytes()))),
            Err(e) => {
                log::warn!("rowel server: the tenant row is unreadable: {e}");
                None
            }
        };
        Self {
            state,
            runtime,
            hash,
            hub: Hub::default(),
        }
    }

    fn writer(&self) -> &str {
        &self.state.writer_id
    }

    /// The definition `client` resolved to, enabled, and its lockbox's row.
    fn definition(&self, client: &Client) -> Option<(Agent, Lockbox)> {
        let store = &self.state.store;
        let agent = store
            .agent(client.agent_id.as_deref()?)
            .ok()
            .flatten()
            .filter(|agent| agent.enabled)?;
        let lockbox = store.lockbox(&agent.lockbox_id).ok().flatten()?;
        Some((agent, lockbox))
    }

    /// `client`'s lockbox, open, with its `agents` switch read into the
    /// session; a switch that moved refuses the asks waiting on it once the
    /// locks are let go.
    fn bind(&self, client: &Client) -> Result<Bound, Code> {
        let (agent, lockbox) = self.definition(client).ok_or(Code::GrantDenied)?;
        let vault = self.state.vaults.open(&self.state, &lockbox.id)?;
        if vault.sync_switch(self.writer(), lockbox.agents) {
            self.state
                .approvals
                .refuse_lockbox(&self.state.store, &lockbox.id);
        }
        Ok(Bound {
            agent,
            lockbox,
            vault,
        })
    }

    /// `op` over `client`'s lockbox, and the rows it wrote mirrored after.
    fn with_bound<T>(
        &self,
        client: &Client,
        op: impl FnOnce(&Bound) -> Result<T, Code>,
    ) -> Result<T, Code> {
        let bound = self.bind(client)?;
        let outcome = op(&bound);
        bound.vault.mirror(&self.state);
        outcome
    }

    /// `check` under the vault's session and agents locks; false while the
    /// vault is sealed.
    fn with_locked(
        &self,
        bound: &Bound,
        check: impl FnOnce(Locked<'_>) -> Result<bool, Code>,
    ) -> Result<bool, Code> {
        let session = bound.vault.session();
        let Ok(store) = session.store() else {
            return Ok(false);
        };
        let mut held = bound.vault.agents();
        initialise(&mut held, bound.lockbox.agents);
        check(Locked {
            store,
            held: &mut held,
            writer: self.writer(),
            now: now_ms(),
        })
    }

    /// The trust policies for the issuer the token names that admit a
    /// client of `kind`, each beside the definition it resolves to. Only
    /// those are candidates, so a token that matches an agent's policy alone
    /// is refused when it says it is a sidecar, and the other way round:
    /// which kind a client is decides what it may do (a sidecar resolves
    /// values, an agent never does), so the kind is the policy's to grant,
    /// not the client's to claim.
    fn policies_for(
        &self,
        token: &str,
        kind: ClientKind,
    ) -> Option<(Vec<TrustPolicy>, Vec<String>)> {
        let issuer = workload::unverified_issuer(token)?;
        let stored = match self.state.store.trust_policies_for_issuer(&issuer) {
            Ok(stored) => stored,
            Err(e) => {
                log::warn!("rowel server: trust policies unreadable: {e}");
                return None;
            }
        };
        Some(
            stored
                .into_iter()
                .filter(|p| p.kind == kind)
                .map(|p| {
                    (
                        TrustPolicy {
                            issuer: p.issuer,
                            claims: p.claims,
                            fixed_audience: p.fixed_audience,
                        },
                        p.agent_id,
                    )
                })
                .unzip(),
        )
    }

    // --- the grant flow's three phases --------------------------------------------------

    fn request_in(
        &self,
        bound: &Bound,
        client: &Client,
        req: GrantRequest,
    ) -> Result<GrantReply, Code> {
        let Bound {
            agent,
            lockbox,
            vault,
        } = bound;
        // The admin's rules are part of what decides; without them policy
        // would decide on less than it was given, so a read that fails is a
        // refusal rather than a decision on the defaults.
        let extra = self.state.store.rules_for(&lockbox.id).map_err(|e| {
            log::warn!("rowel server: rules for {} unreadable: {e}", lockbox.id);
            Code::GrantDenied
        })?;
        let (prepared, epoch) = {
            let session = vault.session();
            let store = session.store()?;
            let mut held = vault.agents();
            initialise(&mut held, lockbox.agents);
            let prepared = decide(
                Locked {
                    store,
                    held: &mut held,
                    writer: self.writer(),
                    now: now_ms(),
                },
                &lockbox.id,
                client,
                req,
                &extra,
            )?;
            (prepared, session.epoch())
        };
        let asking = match prepared {
            Prepared::Answered(reply) => return Ok(reply),
            Prepared::Ask(asking) => *asking,
        };
        // Queued under the agents lock, as `HeadlessHost` queues: a switch
        // that moved since the decision means no ask, and one that moves
        // from here on finds the ask queued and refuses it there. The ask
        // goes on the client's rate record here, where it reaches a person.
        let ticket = {
            let mut held = vault.agents();
            if let Err(reason) = held.access_since(asking.access_epoch) {
                drop(held);
                return self.deny_unasked(vault, client, epoch, &asking, reason);
            }
            held.rate(&client.key).record_ask(now_ms());
            self.state.approvals.enqueue(
                &self.state.store,
                lockbox,
                agent,
                client,
                &asking,
                self.state.config.ask_timeout(),
            )
        };
        let verdict = match ticket.wait() {
            Some(Answer::Grant(answer)) => Verdict::Answered(answer),
            _ => Verdict::Unanswered,
        };
        self.settle_grant(vault, client, epoch, asking, verdict)
    }

    /// A grant decided but never asked about: on record in the session that
    /// decided it, and nowhere if that session is gone.
    fn deny_unasked(
        &self,
        vault: &Vault,
        client: &Client,
        epoch: u64,
        asking: &Asking,
        reason: &str,
    ) -> Result<GrantReply, Code> {
        let session = vault.session();
        if let Ok(store) = session.store_at(epoch) {
            deny_unasked(store, self.writer(), client, asking, reason, now_ms());
        }
        Err(Code::GrantDenied)
    }

    /// The member answered, or nobody did: under the locks again, and only
    /// if it is still the session that asked, the grant is settled.
    fn settle_grant(
        &self,
        vault: &Vault,
        client: &Client,
        epoch: u64,
        asking: Asking,
        verdict: Verdict,
    ) -> Result<GrantReply, Code> {
        let session = vault.session();
        let store = session.store_at(epoch).map_err(|_| {
            log::info!(
                "rowel server: lockbox {} was sealed while an ask waited",
                vault.id()
            );
            Code::GrantDenied
        })?;
        let mut held = vault.agents();
        settle(
            Locked {
                store,
                held: &mut held,
                writer: self.writer(),
                now: now_ms(),
            },
            client,
            asking,
            verdict,
        )
    }
}

/// What the timer does once a second: seal the idle and frozen lockboxes
/// and expire what is due in the rest, forget the replay cache's expired
/// tokens, and the clients long unseen.
pub fn tick(state: &AppState, now: i64) {
    state.vaults.tick(state, now);
    if let Err(e) = state.store.prune_replay(now) {
        log::warn!("rowel server: replay cache prune: {e}");
    }
    if let Err(e) = state
        .store
        .prune_clients(now.saturating_sub(CLIENT_RETENTION_MS))
    {
        log::warn!("rowel server: client prune: {e}");
    }
}

impl Host for OrgHost {
    fn enabled(&self) -> bool {
        true
    }

    fn hub(&self) -> &Hub {
        &self.hub
    }

    // The tenant's digest: the server is never locked as a whole, and an
    // extension keys its associations by what this says.
    fn database_hash(&self) -> Option<String> {
        self.hash.clone()
    }

    // Every client whose definition is still there and enabled: the list
    // `Connection` checks its client against on every request, which is how
    // disabling a definition ends its sessions.
    fn clients(&self) -> Vec<Client> {
        let store = &self.state.store;
        let enabled: HashSet<String> = match store.list_agents() {
            Ok(agents) => agents
                .into_iter()
                .filter(|a| a.enabled)
                .map(|a| a.id)
                .collect(),
            Err(e) => {
                log::warn!("rowel server: agents unreadable: {e}");
                return Vec::new();
            }
        };
        match store.clients() {
            Ok(clients) => clients
                .into_iter()
                .filter(|c| c.agent_id.as_ref().is_some_and(|id| enabled.contains(id)))
                .collect(),
            Err(e) => {
                log::warn!("rowel server: clients unreadable: {e}");
                Vec::new()
            }
        }
    }

    // Never reached: the connection associates through
    // `associate_on_channel`, which carries the key a proof is bound to.
    // Without that key there is nothing to check a proof against.
    fn associate(&self, _key: &str, _who: &Hello) -> Result<Client, Code> {
        Err(Code::AssociationFailed)
    }

    // A proof or nothing: the server never falls back to asking a person.
    // The token is verified against the trust policies for the issuer it
    // names that admit the kind the client says it is, bound to
    // `session_key`, noted in the replay cache, and the
    // policy it matched names the definition the client becomes. Why a
    // proof failed goes to the server's log, never to the client.
    fn associate_on_channel(
        &self,
        session_key: &str,
        key: &str,
        who: &Hello,
    ) -> Result<Client, Code> {
        let proof = who.proof.as_ref().ok_or(Code::AssociationFailed)?;
        let (policies, agent_ids) = self
            .policies_for(&proof.token, who.kind)
            .ok_or(Code::AssociationFailed)?;
        let audience = self.state.tenant_audience().map_err(|e| {
            log::warn!("rowel server: the tenant row is unreadable: {e}");
            Code::AssociationFailed
        })?;
        let verifier = Verifier {
            issuers: &self.state.issuers,
            challenges: &self.state.challenges,
            replay: &self.state.store,
        };
        let verified = verifier
            .verify_proof_blocking(
                &self.runtime,
                proof,
                session_key,
                &audience,
                &policies,
                now_ms(),
            )
            .map_err(|e| {
                log::info!("rowel server: an association was refused: {e}");
                Code::AssociationFailed
            })?;
        let agent_id = &agent_ids[verified.policy_index];
        let lockbox_of = |agent_id: &str| -> Option<(Agent, Lockbox)> {
            let agent = self.state.store.agent(agent_id).ok().flatten()?;
            let lockbox = self.state.store.lockbox(&agent.lockbox_id).ok().flatten()?;
            Some((agent, lockbox))
        };
        let (agent, lockbox) = lockbox_of(agent_id)
            .filter(|(agent, lockbox)| agent.enabled && !lockbox.frozen)
            .ok_or_else(|| {
                log::info!("rowel server: a proof named agent {agent_id}, disabled or frozen");
                Code::AssociationFailed
            })?;
        let now = now_ms();
        let client = Client {
            name: agent.name.clone(),
            key: key.to_string(),
            kind: who.kind,
            platform: who.platform.clone(),
            created_at_ms: now,
            last_seen_at_ms: Some(now),
            agent_id: Some(agent.id.clone()),
            driven_by_agent: false,
        };
        self.state.store.put_client(&client).map_err(|e| {
            log::warn!("rowel server: could not keep the client: {e}");
            Code::AssociationFailed
        })?;
        let vault = self
            .state
            .vaults
            .open(&self.state, &lockbox.id)
            .map_err(|_| Code::AssociationFailed)?;
        {
            let session = vault.session();
            if let Ok(store) = session.store() {
                agents::note(
                    store,
                    self.writer(),
                    agents::draft(Event::Associated, key, std::slice::from_ref(&client)),
                    now,
                );
            }
        }
        vault.mirror(&self.state);
        Ok(client)
    }

    fn client_seen(&self, key: &str) {
        if let Err(e) = self.state.store.client_seen(key) {
            log::debug!("rowel server: client_seen: {e}");
        }
    }

    fn logins_for(&self, _page_url: &str, _host: &str) -> Vec<Login> {
        Vec::new()
    }

    fn totp(&self, _id: &str) -> Option<String> {
        None
    }

    fn generate_password(&self) -> Option<String> {
        None
    }

    fn save_login(
        &self,
        _id: Option<&str>,
        _url: &str,
        _host: &str,
        _username: &str,
        _password: &str,
    ) -> Result<(), Code> {
        Err(Code::ActionCancelledOrDenied)
    }

    fn passkey_register(
        &self,
        _client: &Client,
        _registration: Registration,
    ) -> Result<Value, Code> {
        Err(Code::ActionCancelledOrDenied)
    }

    fn passkey_get(&self, _client: &Client, _assertion: Assertion) -> Result<Value, Code> {
        Err(Code::ActionCancelledOrDenied)
    }

    fn lock(&self) {}

    fn unlock_requested(&self) {}

    fn request_grant(&self, client: &Client, req: GrantRequest) -> Result<GrantReply, Code> {
        self.with_bound(client, |bound| self.request_in(bound, client, req))
    }

    // The long-parked ask of unattended agents is I8; until then a grant is
    // settled when `request_grant` answers, as in I5.
    fn wait_grant(&self, client: &Client, id: &str, _timeout_ms: u64) -> Result<GrantReply, Code> {
        self.grant_status(client, id)
    }

    // What is due to expire does so first, with its row, so the reply never
    // says `active` of a grant the next use would find expired.
    fn grant_status(&self, client: &Client, id: &str) -> Result<GrantReply, Code> {
        self.with_bound(client, |bound| {
            let now = now_ms();
            let session = bound.vault.session();
            let store = session.store()?;
            let mut held = bound.vault.agents();
            initialise(&mut held, bound.lockbox.agents);
            expire_due(Some(store), &mut held, self.writer(), now);
            let grant = held
                .grants
                .iter()
                .find(|g| g.id == id && g.client_key == client.key)
                .ok_or(Code::GrantDenied)?;
            let items = candidates_in(store, &grant.items)
                .into_iter()
                .map(|c| c.visible)
                .collect();
            Ok(reply_for(grant, items))
        })
    }

    fn end_grant(&self, client: &Client, id: &str) -> Result<(), Code> {
        self.with_bound(client, |bound| {
            let session = bound.vault.session();
            let store = session.store()?;
            let mut held = bound.vault.agents();
            initialise(&mut held, bound.lockbox.agents);
            let ctx = Locked {
                store,
                held: &mut held,
                writer: self.writer(),
                now: now_ms(),
            };
            match end_grant_in(ctx, id, Some(&client.key), r#"{"reason":"agent"}"#) {
                GrantEnd::NotFound => Err(Code::GrantDenied),
                GrantEnd::AlreadyOver | GrantEnd::Ended => Ok(()),
            }
        })
    }

    // Only the client's own lockbox has anything to show; one named any
    // other way — by another id, or a name whose slug is not this one's —
    // is answered with nothing rather than this one's items under it.
    fn visible_items(
        &self,
        client: &Client,
        lockbox: Option<&str>,
    ) -> Result<Vec<VisibleRef>, Code> {
        self.with_bound(client, |bound| {
            let session = bound.vault.session();
            let store = session.store()?;
            let mut held = bound.vault.agents();
            initialise(&mut held, bound.lockbox.agents);
            agents::visible_access(&held)?;
            let ours = &bound.lockbox;
            if lockbox.is_some_and(|named| named != ours.id && named != reference::slug(&ours.name))
            {
                return Ok(Vec::new());
            }
            visible_in(store, &held, client, &ours.id, now_ms())
        })
    }

    // Read without unwrapping anything: a sealed vault has no signals.
    fn take_signals(&self, client: &Client) -> Vec<Signal> {
        let Some((_, lockbox)) = self.definition(client) else {
            return Vec::new();
        };
        let Some(vault) = self.state.vaults.get(&lockbox.id) else {
            return Vec::new();
        };
        let mut held = vault.agents();
        held.signals.remove(&client.key).unwrap_or_default()
    }

    // A client bound to no lockbox is covered by nothing; a yes whose row
    // will not write is the engine's refusal, passed on.
    fn grant_covers(
        &self,
        client: &Client,
        grant_id: &str,
        item_id: &str,
        mode: Mode,
    ) -> Result<bool, Code> {
        self.with_bound(client, |bound| {
            Ok(self.with_locked(bound, |ctx| {
                covered(ctx, &client.key, grant_id, item_id, mode)
            }))
        })
        .unwrap_or(Ok(false))
    }

    fn client_grant_covers(
        &self,
        client: &Client,
        item_id: &str,
        mode: Mode,
    ) -> Result<bool, Code> {
        self.with_bound(client, |bound| {
            Ok(self.with_locked(bound, |ctx| client_covered(ctx, &client.key, item_id, mode)))
        })
        .unwrap_or(Ok(false))
    }

    // The page showed what was filled: the grant ends now and the log says
    // why — when the reporter is the grant's client or the extension served
    // under it, and once.
    fn report_exposure(&self, client: &Client, grant_id: &str) {
        let _ = self.with_bound(client, |bound| {
            let session = bound.vault.session();
            let store = session.store()?;
            let mut held = bound.vault.agents();
            let ctx = Locked {
                store,
                held: &mut held,
                writer: self.writer(),
                now: now_ms(),
            };
            agents::report_exposure_in(ctx, grant_id, &client.key);
            Ok(())
        });
    }

    // The one reply that carries a value, in the same critical section
    // every other release is made in. The placeholder must name the
    // client's lockbox, by id or by its name's slug, as in Tier 1.
    fn resolve(
        &self,
        client: &Client,
        grant_id: &str,
        reference: &Reference,
        ctx: &ResolveContext,
    ) -> Result<Zeroizing<String>, Code> {
        self.with_bound(client, |bound| {
            let session = bound.vault.session();
            let open = session.open()?;
            let mut held = bound.vault.agents();
            initialise(&mut held, bound.lockbox.agents);
            resolved(
                Locked {
                    store: &open.opened.store,
                    held: &mut held,
                    writer: self.writer(),
                    now: now_ms(),
                },
                &open.cipher,
                &agents::Lockbox {
                    id: bound.lockbox.id.clone(),
                    name: Some(bound.lockbox.name.clone()),
                },
                client,
                grant_id,
                reference,
                ctx,
            )
        })
    }
}
