//! A grant request between the lock and the ask, and every release under one:
//! what is decided before anyone is asked, what the answer settles, whether a
//! grant covers a fill, what a placeholder resolves to, and what an agent may
//! see. All of it under [`Locked`], which the host takes before calling in.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use rowel_core::crypto::PayloadCipher;
use rowel_core::store::{EntryMeta, SqliteStore, VaultStore};

use crate::actions::{self, Client, ClientKind, VisibleRef};
use crate::audit::{Draft, Event};
use crate::grant::{
    Approval, CandidateItem, Grant, GrantReply, GrantRequest, ItemSelector, Mode, Probe,
    ResolveContext, State, Target, VisibleItem,
};
use crate::lockbox::{clients_in, field_value, item_by_name, live_rules, write_rules_in};
use crate::policy::{self, defaults, AgentSelector, Decision, Floors, Limits};
use crate::protocol::Code;
use crate::reference::{self, Reference};
use crate::signal::Signal;

use super::audit::{
    about, audit_target_of, client_label, draft, note, origin_of, record, record_with,
    resolve_target_text, site_origin,
};
use super::state::AgentState;

/// What a grant is decided, settled or checked against, under the session
/// and agents locks: the open store, what the agents lock guards — the
/// grants held and the switches as the session holds them — and whose log
/// the rows go to. The caller holds both locks for the whole of it, which is
/// what makes the check of the switches and the release one critical
/// section; nothing of the app is in it, so the tests run the same code over
/// a store of their own.
pub struct Locked<'a> {
    pub store: &'a SqliteStore,
    pub held: &'a mut AgentState,
    pub writer: &'a str,
    pub now: i64,
}

/// What the user answered a grant ask with: which of the candidates the
/// dialog listed — the agent may be given that one and no other — and, for
/// "allow for N days", how many; `None` is this once. Serialized too: an
/// answer given on a member's device travels to the Rowel Server in this
/// shape (`POST /v1/approvals/{id}`), so the dialog's answer and the wire's
/// are one struct.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GrantAnswer {
    pub item_id: String,
    pub standing_days: Option<u32>,
}

/// What a grant request came to before anyone was asked: answered already
/// (served from a grant held, allowed outright by a rule), or waiting on the
/// user.
pub enum Prepared {
    Answered(GrantReply),
    // Boxed: an ask carries the whole grant and its candidates, a reply a
    // fraction of that.
    Ask(Box<Asking>),
}

/// A grant policy let through to the dialog, as it waits: the grant as it
/// stands — its targets are where the dialog says the item would go — and
/// what the dialog was shown. Which session it was decided in is the host's
/// to remember beside it: nothing here knows one.
pub struct Asking {
    /// The switches' epoch it was decided under (`AgentState::access_epoch`):
    /// a change since, whatever it left the switches at, settles it as a no.
    pub access_epoch: u64,
    pub grant: Grant,
    pub candidates: Vec<Candidate>,
    pub client_name: String,
}

/// How the ask came down: with the user's answer — a pick, or a refusal — or
/// without one, which is a minute gone by (or a dialog of another kind
/// answering by mistake).
pub enum Verdict {
    Answered(Option<GrantAnswer>),
    Unanswered,
}

/// One of the page's logins, as policy sees it and as the dialog shows it:
/// nothing sealed in either.
#[derive(Clone)]
pub struct Candidate {
    pub item: CandidateItem,
    pub visible: VisibleItem,
}

/// The open lockbox as a placeholder may name it: by its id, or by the slug
/// of its registry name when it has one.
pub struct Lockbox {
    pub id: String,
    pub name: Option<String>,
}

impl Lockbox {
    fn is_named(&self, name: &str) -> bool {
        name == self.id
            || self
                .name
                .as_deref()
                .is_some_and(|registered| reference::slug(registered) == name)
    }
}

/// Everything a grant request decides before anyone is asked: a grant this
/// client already holds for the place (served), the rules, the rate, policy,
/// and the grant itself when policy allows without asking (served). Both
/// releases come after one look at the switches under the same locks.
///
/// Two classes of request come through here. A *site* request (`fill`,
/// `passkey`) names a page and is answered with the login itself, so its
/// activation is a use. A *secret* request (`header`, `env`) names where a
/// sidecar will use a value and is answered with nothing but the grant's id:
/// the value leaves only through `resolve`, which is where its uses are
/// counted and its `used` rows written. The two never mix in one request,
/// and `token` is nobody's yet.
///
/// `extra` are rules the host holds outside the lockbox — a headless
/// broker's config file — evaluated after the baseline and before the
/// lockbox's own; the app has none and passes `&[]`.
pub fn decide(
    ctx: Locked<'_>,
    lockbox: &str,
    client: &Client,
    req: GrantRequest,
    extra: &[policy::Rule],
) -> Result<Prepared, Code> {
    let Locked {
        store,
        held,
        writer,
        now,
    } = ctx;
    let client_key = client.key.as_str();
    let clients = clients_in(store);
    let client_name = client_label(&clients, client_key);
    let row = |event: Event| draft(event, client_key, &clients);
    // A refusal before there is a grant to be about: the place and the
    // mode are still worth the row.
    let refusal = |event: Event, detail: String| {
        let mut draft = row(event);
        draft.target = req.targets.first().map(audit_target_of);
        draft.mode = req.modes.first().copied();
        draft.detail = Some(detail);
        draft
    };
    if !well_targeted(&req) {
        note(
            store,
            writer,
            refusal(Event::Denied, r#"{"reason":"no_target"}"#.into()),
            now,
        );
        return Err(Code::GrantDenied);
    }
    let secret = is_secret(&req.modes);
    // The switches as the session holds them, under the locks and before
    // either release below. The app's off is the extension's `AgentsDisabled`
    // and no row; the lockbox's off is a refusal on record.
    match held.access_on() {
        Ok(()) => {}
        Err("lockbox") => {
            note(
                store,
                writer,
                refusal(Event::Denied, r#"{"reason":"lockbox"}"#.into()),
                now,
            );
            return Err(Code::GrantDenied);
        }
        Err(_) => return Err(Code::AgentsDisabled),
    }
    let access_epoch = held.access_epoch;

    // What the request could be about. The extension names the page's logins
    // itself — it has them in hand, since it asked for them. An agent names a
    // site and, at most, a selector: the candidates are the lockbox's logins
    // for that site, as the plaintext `url_host` column has them, narrowed by
    // the selector. A secret request names items, by id or by the slug of
    // their title, among the kinds that have a field to serve. No payload is
    // opened to decide a grant.
    let candidates = if secret {
        match secret_candidates_in(store, &req.items) {
            Some(candidates) => candidates,
            None => {
                note(
                    store,
                    writer,
                    refusal(Event::Denied, r#"{"reason":"no_candidates"}"#.into()),
                    now,
                );
                held.rate(client_key).record_denial(now);
                return Err(Code::GrantDenied);
            }
        }
    } else if client.kind == ClientKind::Extension {
        candidates_in(store, &req.items)
    } else {
        let origin = origin_of(&req.targets).unwrap_or_default();
        site_candidates_in(store, origin, &req.items)
    };
    // The concrete places asked for: a page is itself; an HTTP target is one
    // probe per method it names, or one for any; a command is its line. A
    // rule has to cover every one of them.
    let probes: Vec<Probe> = req.targets.iter().flat_map(Target::probes).collect();

    // A grant this client already holds for the place is served, not asked
    // for again: a username-first sign-in asks for the page's logins twice,
    // and the second time is the same yes; a sidecar that requests before
    // every command gets the grant it already has. A fill reuse is a use —
    // the login is in the reply; an agent's is signalled again, since the
    // page it is for may be in a tab opened since. A secret reuse releases
    // nothing and is written nowhere: its uses are `resolve`'s — and it has
    // to admit every item asked for ([`admits_all`]), where a page's grant
    // need only admit one of the page's logins.
    let reusable = held.grants.iter_mut().find(|g| {
        g.client_key == client_key
            && g.is_active(now)
            && req.modes.iter().all(|m| g.modes.contains(m))
            && probes
                .iter()
                .all(|probe| g.targets.iter().any(|t| t.matches(probe)))
            && if secret {
                admits_all(&g.items, &candidates)
            } else {
                candidates.iter().any(|c| g.items.admits(&c.item))
            }
    });
    if let Some(grant) = reusable {
        let items = admitted(&candidates, &grant.items);
        if !secret {
            use_grant(
                store,
                writer,
                grant,
                None,
                |counted| about(row(Event::Used), counted, Some(r#"{"reuse":true}"#)),
                now,
            )?;
        }
        let grant = grant.clone();
        queue_fill(held, &clients, client, &grant, &items);
        return Ok(Prepared::Answered(reply_for(&grant, items)));
    }

    // The baseline every lockbox with the switch on starts with, then the
    // host's own rules, then the lockbox's — the ones the user wrote by
    // answering "for N days".
    let mut rules = defaults::personal();
    rules.extend(extra.iter().cloned());
    rules.extend(live_rules(store, now));

    if held
        .rate(client_key)
        .check(&Limits::default(), now)
        .is_err()
    {
        note(
            store,
            writer,
            refusal(Event::RateLimited, r#"{"reason":"rate"}"#.into()),
            now,
        );
        return Err(Code::RateLimited);
    }

    let items: Vec<CandidateItem> = candidates.iter().map(|c| c.item.clone()).collect();
    let decision = policy::evaluate(&rules, &req, &items, &probes, &Floors::default());
    let (terms, rule_id) = match &decision {
        Decision::Denied(reason) => {
            let reason = serde_json::to_string(reason).unwrap_or_default();
            note(
                store,
                writer,
                refusal(Event::Denied, format!(r#"{{"reason":{reason}}}"#)),
                now,
            );
            held.rate(client_key).record_denial(now);
            return Err(Code::GrantDenied);
        }
        Decision::Allow { rule_id, .. } => {
            (decision.terms().ok_or(Code::GrantDenied)?, rule_id.clone())
        }
    };
    let mut grant = Grant::from_request(lockbox, req, terms, now);
    // A grant is never wider than the rule that allowed it: what the
    // winning rule admits among the page's logins is what may be served,
    // and what the dialog offers.
    if let Some(rule) = rule_id.and_then(|id| rules.into_iter().find(|r| r.id == id)) {
        grant.items = ItemSelector::Ids(
            candidates
                .iter()
                .filter(|c| rule.items.admits(&c.item))
                .map(|c| c.item.id.clone())
                .collect(),
        );
    }
    let candidates: Vec<Candidate> = candidates
        .into_iter()
        .filter(|c| grant.items.admits(&c.item))
        .collect();

    match grant.approval {
        // `requested` is written here only for a request that is decided
        // here; an ask writes its own once the dialog has really gone up,
        // so a slot that was busy leaves no row behind.
        Approval::Auto => {
            note(
                store,
                writer,
                about(row(Event::Requested), &grant, None),
                now,
            );
            grant.state = State::Active;
            grant.approver = Some("policy".into());
            activate(store, writer, &row, &mut grant, now)?;
            let items: Vec<VisibleItem> = candidates.into_iter().map(|c| c.visible).collect();
            queue_fill(held, &clients, client, &grant, &items);
            let reply = reply_for(&grant, items);
            held.grants.push(grant);
            Ok(Prepared::Answered(reply))
        }
        // Not yet on record as an ask: that is written when the dialog goes
        // up, which a busy slot never lets it do.
        Approval::Ask | Approval::StepUp => Ok(Prepared::Ask(Box::new(Asking {
            access_epoch,
            grant,
            candidates,
            client_name,
        }))),
    }
}

/// A grant decided but never asked about: the switches moved between the
/// decision and the dialog. No `requested` row — nothing reached the
/// screen — just the refusal, in the session that decided it (`store` is
/// that session's, which the host has checked is still the one open).
pub fn deny_unasked(
    store: &SqliteStore,
    writer: &str,
    client: &Client,
    asking: &Asking,
    reason: &str,
    now: i64,
) {
    let clients = clients_in(store);
    let mut grant = asking.grant.clone();
    grant.state = State::Denied;
    note(
        store,
        writer,
        about(
            draft(Event::Denied, &client.key, &clients),
            &grant,
            Some(&format!(r#"{{"reason":"{reason}"}}"#)),
        ),
        now,
    );
}

/// A grant goes active, on record: `granted`, and — for a site grant, whose
/// reply carries the login — a use counted and written at once. A secret
/// grant's reply carries nothing, so nothing is counted here: its uses are
/// `resolve`'s to count, one per value served. A row that will not write is
/// `AuditUnavailable`, and the caller keeps nothing of the grant: it is not
/// among the grants held until this has returned, and a failure drops it, so
/// what this did to it is dropped too.
fn activate(
    store: &SqliteStore,
    writer: &str,
    row: &dyn Fn(Event) -> Draft,
    grant: &mut Grant,
    now: i64,
) -> Result<(), Code> {
    record(store, writer, about(row(Event::Granted), grant, None), now)?;
    if !is_secret(&grant.modes) {
        use_grant(
            store,
            writer,
            grant,
            None,
            |counted| about(row(Event::Used), counted, None),
            now,
        )?;
    }
    Ok(())
}

/// Count one use of `grant` — and claim it for `claim`, the extension served
/// under another client's grant — write the `used` row `row` builds from the
/// counted copy, and only then keep the copy: a row that will not write
/// leaves the grant exactly as it was, so a use the log does not hold never
/// spends, counts or claims anything. Every use of a grant goes through here.
fn use_grant(
    store: &SqliteStore,
    writer: &str,
    grant: &mut Grant,
    claim: Option<&str>,
    row: impl FnOnce(&Grant) -> Draft,
    now: i64,
) -> Result<(), Code> {
    let mut counted = grant.clone();
    counted.use_once(now)?;
    if let Some(client_key) = claim {
        counted.served_to = Some(client_key.to_string());
    }
    record(store, writer, row(&counted), now)?;
    *grant = counted;
    Ok(())
}

/// Whether `selector` admits every one of `candidates`, there being at least
/// one: what a secret request needs of a held grant, as `policy::evaluate`
/// needs it of a rule, since a command runs with every item it named or not
/// at all.
fn admits_all(selector: &ItemSelector, candidates: &[Candidate]) -> bool {
    !candidates.is_empty() && candidates.iter().all(|c| selector.admits(&c.item))
}

/// Whether `modes` are a secret request's ([`Mode::is_secret`]) rather than
/// a site's. A request mixing the two is refused before this is asked
/// ([`well_targeted`]).
fn is_secret(modes: &[Mode]) -> bool {
    modes.iter().any(|mode| mode.is_secret())
}

/// Whether every target of `req` is the kind of place every one of its modes
/// is performed at: an origin for `fill` and `passkey`, an HTTP pattern for
/// `header`, a command line for `env`. A `token` mode fits nothing, since
/// nothing serves it yet; a request mixing a site mode with a secret one, or
/// naming a page for a header, fits nothing either — a grant is never wider
/// than its request, and such a request has no one shape to be.
fn well_targeted(req: &GrantRequest) -> bool {
    let fits = |target: &Target, mode: Mode| {
        matches!(
            (target, mode),
            (Target::Origin(_), Mode::Fill | Mode::Passkey)
                | (Target::Http { .. }, Mode::Header)
                | (Target::Command(_), Mode::Env)
        )
    };
    !req.targets.is_empty()
        && !req.modes.is_empty()
        && req
            .targets
            .iter()
            .all(|target| req.modes.iter().all(|mode| fits(target, *mode)))
}

/// The user has answered (or not): the grant becomes active for the one
/// item picked — a standing answer writes its rule first — or denied. The
/// switches come first, under the same locks as the release, and the epoch
/// the request was decided under: a yes given while a switch moved is no
/// yes, and the row names the switch.
pub fn settle(
    ctx: Locked<'_>,
    client: &Client,
    asking: Asking,
    verdict: Verdict,
) -> Result<GrantReply, Code> {
    let Locked {
        store,
        held,
        writer,
        now,
    } = ctx;
    let Asking {
        access_epoch,
        mut grant,
        candidates,
        ..
    } = asking;
    let client_key = client.key.as_str();
    let clients = clients_in(store);
    let row = |event: Event| draft(event, client_key, &clients);

    // The ask went up and came down: the request is on record first,
    // whatever the answer was, as it stood when the user saw it.
    note(
        store,
        writer,
        about(row(Event::Requested), &grant, None),
        now,
    );

    let settled = held
        .access_since(access_epoch)
        .and_then(|()| outcome(&verdict, &candidates));
    // A site request's yes is for the one login picked; a secret request's
    // is for every item it named, since the command or the request needs
    // them all at once — the pick is still required to be one of them, so
    // a stale dialog cannot answer for a request it was not drawn for.
    let (items, standing_days) = match settled {
        Ok((_, standing_days)) if is_secret(&grant.modes) => (
            candidates.iter().map(|c| c.visible.clone()).collect(),
            standing_days,
        ),
        Ok((picked, standing_days)) => (vec![picked.visible.clone()], standing_days),
        Err(reason) => {
            grant.state = State::Denied;
            note(
                store,
                writer,
                about(
                    row(Event::Denied),
                    &grant,
                    Some(&format!(r#"{{"reason":"{reason}"}}"#)),
                ),
                now,
            );
            held.rate(client_key).record_denial(now);
            return Err(Code::GrantDenied);
        }
    };

    grant.items = ItemSelector::Ids(items.iter().map(|i| i.id.clone()).collect());
    grant.state = State::Active;
    grant.approver = Some("user".into());
    if let Some(days) = standing_days {
        let days = days.clamp(1, defaults::MAX_STANDING_DAYS);
        let rule = standing_rule(client_key, &grant, days, now);
        let mut added = about(
            row(Event::RuleAdded),
            &grant,
            Some(&format!(r#"{{"ruleId":"{}","days":{days}}}"#, rule.id)),
        );
        added.grant_id = None;
        added.approval = Some(Approval::Auto);
        let mut rules = live_rules(store, now);
        rules.push(rule);
        // A rule is a release to come, so it is stored only with its row, in
        // one transaction: one that will not commit leaves neither. The yes
        // it came with is not the rule's to refuse — its own rows below
        // decide that, and fail closed as well — so it still goes ahead,
        // only without the memory of it, and the next visit asks again.
        if record_with(store, writer, added, now, |tx| write_rules_in(tx, &rules)).is_err() {
            log::warn!("browser host: the standing rule was not kept");
        }
    }
    activate(store, writer, &row, &mut grant, now)?;
    held.rate(client_key).record_grant(now);
    queue_fill(held, &clients, client, &grant, &items);
    let reply = reply_for(&grant, items);
    held.grants.push(grant);
    Ok(reply)
}

/// The rule a "for N days" writes: this client, the items the settled
/// `grant` admits, where it is for, in its mode. For a fill that is the one
/// login picked at the site — not the page, since the rule outlives this
/// visit. For a secret grant it is every item the request named and the
/// request's own targets, every one of them, so a standing `env` rule names
/// the full command line and lifts the floor for that line alone, and
/// answers the whole request next time (`policy::evaluate`). `standing_for`
/// takes one item and one target, so it is given the first of each and the
/// whole lists put in their place; a settled grant has at least one of each
/// ([`well_targeted`], `outcome`), and the fallbacks are only so that this
/// cannot panic.
fn standing_rule(client_key: &str, grant: &Grant, days: u32, now: i64) -> policy::Rule {
    let first_id = match &grant.items {
        ItemSelector::Ids(ids) => ids.first().cloned().unwrap_or_default(),
        _ => String::new(),
    };
    if !is_secret(&grant.modes) {
        let origin = origin_of(&grant.targets).unwrap_or_default();
        return defaults::standing(client_key, &first_id, &site_origin(origin), days, now);
    }
    let mode = grant.modes.first().copied().unwrap_or(Mode::Header);
    let first = grant
        .targets
        .first()
        .cloned()
        .unwrap_or(Target::Command(String::new()));
    let mut rule = defaults::standing_for(client_key, &first_id, first, mode, days, now);
    rule.items = grant.items.clone();
    rule.targets = grant.targets.clone();
    rule
}

/// Whether a grant of `client`'s is one the extensions are told about: an
/// agent's fill grant. The extension's own grants need no telling — it holds
/// the reply — and nothing but a fill is the extension's to perform.
fn signals_fill(client: &Client, grant: &Grant) -> bool {
    client.kind != ClientKind::Extension && grant.modes.contains(&Mode::Fill)
}

/// An agent's fill grant is active: every extension of the lockbox is told,
/// since the host cannot tell which browser profile has the page open
/// (`Grant::served_to` settles that at the first fill). The item is the one
/// the grant admits — the dialog's pick, or the standing rule's one item.
fn queue_fill(
    held: &mut AgentState,
    clients: &[Client],
    client: &Client,
    grant: &Grant,
    items: &[VisibleItem],
) {
    let Some(item) = items.first() else {
        return;
    };
    if !signals_fill(client, grant) {
        return;
    }
    let Some(origin) = origin_of(&grant.targets) else {
        return;
    };
    held.signal_extensions(
        clients,
        &Signal::FillGrant {
            grant_id: grant.id.clone(),
            origin: site_origin(origin),
            item: item.clone(),
            auto_submit: false,
            expires_at_ms: grant.expires_at_ms,
        },
    );
}

/// Whether grant `grant_id` is active and covers `item_id` in `mode` right
/// now, for the client holding `client_key` — a predicate with a side
/// effect, on purpose: a yes here is the login or its code about to be
/// served, so it is a use of the grant, counted on it and written as a
/// `used` row, and this is the one place the grant is in hand to say so. A
/// yes whose row will not write is `AuditUnavailable`: nothing is served. The
/// switches come first, under the same locks: a grant still on the list is
/// not served once the session's switches say no. The targets are not
/// checked again: the TOTP step comes with no URL, and the grant was
/// narrowed to its page when it was issued. A grant another client requested
/// — an agent's, signalled to every extension — is claimed by the first
/// extension served under it (`Grant::served_to`) and refused to any other.
pub fn covered(
    ctx: Locked<'_>,
    client_key: &str,
    grant_id: &str,
    item_id: &str,
    mode: Mode,
) -> Result<bool, Code> {
    covered_via(ctx, client_key, grant_id, item_id, mode, "grantId")
}

/// [`covered`], with what the `used` row says the grant was found by.
fn covered_via(
    ctx: Locked<'_>,
    client_key: &str,
    grant_id: &str,
    item_id: &str,
    mode: Mode,
    via: &str,
) -> Result<bool, Code> {
    let Locked {
        store,
        held,
        writer,
        now,
    } = ctx;
    if held.access_on().is_err() {
        return Ok(false);
    }
    let Some(grant) = held.find_mut(grant_id) else {
        return Ok(false);
    };
    if !grant.is_active(now) || !grant.modes.contains(&mode) {
        return Ok(false);
    }
    let claim = match &grant.served_to {
        _ if grant.client_key == client_key => None,
        None => Some(client_key),
        Some(served) if served == client_key => None,
        Some(_) => return Ok(false),
    };
    // The item as the grant's selector sees it. Every grant issued here
    // names ids, which the id alone answers; a wider selector reads the
    // row's kind and tags, nothing sealed.
    let item = match &grant.items {
        ItemSelector::Ids(_) => CandidateItem {
            id: item_id.to_string(),
            kind: String::new(),
            tags: Vec::new(),
        },
        _ => match store.row_meta(item_id) {
            Ok(Some(meta)) => candidate_item(&meta),
            _ => return Ok(false),
        },
    };
    if !grant.items.admits(&item) {
        return Ok(false);
    }
    let clients = clients_in(store);
    let used = use_grant(
        store,
        writer,
        grant,
        claim,
        |counted| {
            let mut row = about(
                draft(Event::Used, &counted.client_key, &clients),
                counted,
                Some(&format!(r#"{{"via":"{via}"}}"#)),
            );
            row.item_id = Some(item_id.to_string());
            row.mode = Some(mode);
            row
        },
        now,
    );
    // A use refused is a no; a use the log would not take is a refusal.
    match used {
        Err(Code::AuditUnavailable) => Err(Code::AuditUnavailable),
        used => Ok(used.is_ok()),
    }
}

/// [`covered`] over the grants the client holding `client_key` was issued or
/// already served under: the first that covers `item_id` counts the use.
/// What an extension's TOTP step is served under when it names no grant; a
/// grant nobody has been served under yet is not claimed this way.
pub fn client_covered(
    ctx: Locked<'_>,
    client_key: &str,
    item_id: &str,
    mode: Mode,
) -> Result<bool, Code> {
    let Locked {
        store,
        held,
        writer,
        now,
    } = ctx;
    let ids: Vec<String> = held
        .grants
        .iter()
        .filter(|g| g.client_key == client_key || g.served_to.as_deref() == Some(client_key))
        .map(|g| g.id.clone())
        .collect();
    for id in &ids {
        let ctx = Locked {
            store,
            held: &mut *held,
            writer,
            now,
        };
        if covered_via(ctx, client_key, id, item_id, mode, "client")? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The value `reference` names, for a sidecar acting under `grant_id` at
/// `ctx.target` — the whole of `Host::resolve` under the session and agents
/// locks, over the open store and the grants held. The switches come first,
/// as at every release. Then the grant by id, anyone's, since the id is the
/// capability an agent hands a sidecar; it must be in the open lockbox, which
/// the placeholder must name too, by id or by the registry name's slug. The
/// item is found by id or by the unique slug of its title; the field must be
/// one the item's kind has; the grant must cover the item at the target in
/// the mode the target implies. Only then is the payload unsealed and the
/// field read, and a field that is absent or empty is a refusal — there is
/// nothing to serve. A yes is a use, counted and written as a `used` row
/// that names the sidecar, the field and the one place the value went, and
/// a row that will not write is `AuditUnavailable` with no value. Every no
/// is `GrantDenied` (or `GrantExpired` from `use_once`), whatever the
/// reason: why a capability did not work is not for whoever holds it.
pub fn resolved(
    ctx: Locked<'_>,
    cipher: &PayloadCipher,
    lockbox: &Lockbox,
    client: &Client,
    grant_id: &str,
    reference: &Reference,
    rctx: &ResolveContext,
) -> Result<Zeroizing<String>, Code> {
    let Locked {
        store,
        held,
        writer,
        now,
    } = ctx;
    held.access_on().map_err(|_| Code::GrantDenied)?;
    let grant = held.find_mut(grant_id).ok_or(Code::GrantDenied)?;
    if !grant.is_active(now)
        || grant.lockbox_id != lockbox.id
        || !lockbox.is_named(&reference.lockbox)
    {
        return Err(Code::GrantDenied);
    }
    let metas = store.list().map_err(|_| Code::GrantDenied)?;
    let meta = item_by_name(&metas, &reference.item).ok_or(Code::GrantDenied)?;
    if !reference.field.kinds().contains(&meta.kind.as_str()) {
        return Err(Code::GrantDenied);
    }
    let item = candidate_item(meta);
    let mode = rctx.target.mode();
    if !grant.covers(&item, &rctx.target.probe(), mode, now) {
        return Err(Code::GrantDenied);
    }
    let value = store
        .get(&item.id)
        .ok()
        .flatten()
        .and_then(|record| cipher.unseal(&record.id, &record.payload).ok())
        .and_then(|entry| field_value(&entry, &reference.field))
        .ok_or(Code::GrantDenied)?;
    let clients = clients_in(store);
    use_grant(
        store,
        writer,
        grant,
        None,
        |counted| {
            let mut row = about(
                draft(Event::Used, &client.key, &clients),
                counted,
                // The field as the placeholder wrote it, JSON-escaped: an env
                // key may hold a quote or a backslash.
                Some(&format!(
                    r#"{{"via":"resolve","field":{}}}"#,
                    serde_json::Value::from(reference.field.to_string())
                )),
            );
            row.item_id = Some(item.id);
            row.mode = Some(mode);
            row.target = Some(resolve_target_text(&rctx.target));
            row
        },
        now,
    )?;
    Ok(value)
}

/// What the answer comes to: the candidate the user picked and for how many
/// days, or the reason the `denied` row names.
pub fn outcome<'c>(
    verdict: &Verdict,
    candidates: &'c [Candidate],
) -> Result<(&'c Candidate, Option<u32>), &'static str> {
    match verdict {
        Verdict::Answered(Some(answer)) => candidates
            .iter()
            .find(|c| c.item.id == answer.item_id)
            .map(|c| (c, answer.standing_days))
            .ok_or("item"),
        Verdict::Answered(None) => Err("user"),
        Verdict::Unanswered => Err("timeout"),
    }
}

/// The items `selector` names among the open vault's rows, from the list's
/// metadata alone — kind, tags, title and username are plaintext columns;
/// no payload is opened to decide a grant.
pub fn candidates_in(store: &SqliteStore, selector: &ItemSelector) -> Vec<Candidate> {
    let metas = match store.list() {
        Ok(metas) => metas,
        Err(e) => {
            log::warn!("browser host: could not list the vault: {e}");
            return Vec::new();
        }
    };
    metas
        .into_iter()
        .filter_map(|meta| candidate_of(meta, selector))
        .collect()
}

/// The lockbox's logins for the site at `origin`, as an agent's request names
/// them: the rows whose plaintext `url_host` is the origin's host or a parent
/// of it ([`actions::host_matches`], the same first pass a fill makes), then
/// what `selector` keeps of those. The column only narrows: what is served
/// under the grant is still decided by the sealed website at fill time
/// (`lockbox::logins_in`). An origin with no host names nothing.
fn site_candidates_in(
    store: &SqliteStore,
    origin: &str,
    selector: &ItemSelector,
) -> Vec<Candidate> {
    let Some(host) = actions::site_host(origin) else {
        return Vec::new();
    };
    let metas = match store.list() {
        Ok(metas) => metas,
        Err(e) => {
            log::warn!("browser host: could not list the vault: {e}");
            return Vec::new();
        }
    };
    metas
        .into_iter()
        .filter(|meta| meta.kind == "login" && actions::host_matches(&host, &meta.url_host))
        .filter_map(|meta| candidate_of(meta, selector))
        .collect()
}

/// The items a secret request names, among the kinds that have a field a
/// sidecar could be served (`reference::fields_for`): a note or an identity
/// is never a candidate, whatever the selector says. Ids may be written as
/// the slug of the item's title, the way a placeholder writes them
/// ([`item_by_name`]); the candidates carry the ids, so the grant does too.
/// `None` when a name among `Ids` picks out nothing servable: the command
/// needs every item it named, so a request for something that is not there
/// is not answered with the rest.
fn secret_candidates_in(store: &SqliteStore, selector: &ItemSelector) -> Option<Vec<Candidate>> {
    let metas = match store.list() {
        Ok(metas) => metas,
        Err(e) => {
            log::warn!("browser host: could not list the vault: {e}");
            return None;
        }
    };
    let servable = |meta: &EntryMeta| !reference::fields_for(&meta.kind).is_empty();
    let by_id = match selector {
        ItemSelector::Ids(names) => ItemSelector::Ids(
            names
                .iter()
                .map(|name| {
                    item_by_name(&metas, name)
                        .filter(|meta| servable(meta))
                        .map(|meta| meta.id.clone())
                })
                .collect::<Option<Vec<String>>>()?,
        ),
        other => other.clone(),
    };
    Some(
        metas
            .into_iter()
            .filter(servable)
            .filter_map(|meta| candidate_of(meta, &by_id))
            .collect(),
    )
}

/// `meta` as a selector sees it: id, kind and tags, nothing sealed.
fn candidate_item(meta: &EntryMeta) -> CandidateItem {
    CandidateItem {
        id: meta.id.clone(),
        kind: meta.kind.clone(),
        tags: serde_json::from_str(&meta.tags).unwrap_or_default(),
    }
}

/// `meta` as a candidate, if `selector` admits it.
fn candidate_of(meta: EntryMeta, selector: &ItemSelector) -> Option<Candidate> {
    let item = candidate_item(&meta);
    selector.admits(&item).then(|| Candidate {
        visible: VisibleItem {
            id: meta.id,
            title: meta.title,
            username: meta.username.filter(|u| !u.is_empty()),
        },
        item,
    })
}

fn admitted(candidates: &[Candidate], selector: &ItemSelector) -> Vec<VisibleItem> {
    candidates
        .iter()
        .filter(|c| selector.admits(&c.item))
        .map(|c| c.visible.clone())
        .collect()
}

pub fn reply_for(grant: &Grant, items: Vec<VisibleItem>) -> GrantReply {
    GrantReply {
        grant_id: grant.id.clone(),
        state: grant.state,
        expires_at_ms: Some(grant.expires_at_ms),
        retry_after_ms: None,
        uses: grant.uses,
        items,
    }
}

/// The items `client` may ask for by name, in `lockbox`: what its own live
/// rules admit — the ones that name its key, never the baseline, which names
/// every client and would make the whole lockbox visible to anyone let in —
/// and what its own active grants admit. From the list's metadata alone, like
/// every other decision about a grant: no payload is opened to say what
/// exists.
pub fn visible_in(
    store: &SqliteStore,
    held: &AgentState,
    client: &Client,
    lockbox: &str,
    now: i64,
) -> Result<Vec<VisibleRef>, Code> {
    // The switches gate the names as they gate the uses: an agent let in
    // before the Agents switch, or the lockbox's access, went off learns
    // nothing more from its standing rules than it could use.
    visible_access(held)?;
    let rules = live_rules(store, now);
    let selectors: Vec<&ItemSelector> = rules
        .iter()
        .filter(|rule| matches!(&rule.agents, AgentSelector::Ids(ids) if ids.contains(&client.key)))
        .map(|rule| &rule.items)
        .chain(
            held.grants
                .iter()
                .filter(|g| g.client_key == client.key && g.is_active(now))
                .map(|g| &g.items),
        )
        .collect();
    if selectors.is_empty() {
        return Ok(Vec::new());
    }
    let metas = match store.list() {
        Ok(metas) => metas,
        Err(e) => {
            log::warn!("browser host: could not list the vault: {e}");
            return Err(Code::DatabaseNotOpened);
        }
    };
    Ok(metas
        .into_iter()
        .filter(|meta| {
            let item = candidate_item(meta);
            selectors.iter().any(|selector| selector.admits(&item))
        })
        .map(|meta| VisibleRef {
            reference: format!("rowel://{lockbox}/{}", meta.id),
            title: meta.title,
            fields: reference::fields_for(&meta.kind)
                .iter()
                .map(|f| f.to_string())
                .collect(),
            kind: meta.kind,
        })
        .collect())
}

/// The switches as `visible_items` answers them: the lockbox's off is a
/// refusal, the app's off is `AgentsDisabled`, as at a request.
pub fn visible_access(held: &AgentState) -> Result<(), Code> {
    match held.access_on() {
        Ok(()) => {}
        Err("lockbox") => return Err(Code::GrantDenied),
        Err(_) => return Err(Code::AgentsDisabled),
    }
    Ok(())
}
