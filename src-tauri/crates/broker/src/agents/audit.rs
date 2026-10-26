//! What goes on a row about an agent, and the words Settings and the log
//! share for a target, a client and a mode. Nothing secret goes in a row: the
//! target is reduced to scheme, host and path before it is written.

use rowel_core::store::SqliteStore;

use crate::actions::{clamp, Client, ClientKind};
use crate::audit::{ActorKind, Draft, Event, Writer};
use crate::grant::{Grant, ItemSelector, Mode, ResolveTarget, Target};
use crate::policy::{AgentSelector, Rule};
use crate::protocol::Code;

/// A row about what client `key` did, as the kind of client the lockbox's
/// record says it is. A key the lockbox never named — a stale association —
/// is written as an extension, the one kind there was before there were
/// kinds, and the only kind that reaches the host without a record.
pub fn draft(event: Event, key: &str, clients: &[Client]) -> Draft {
    let client = clients.iter().find(|c| c.key == key);
    let kind = client.map_or(ClientKind::Extension, |c| c.kind);
    let mut draft = Draft::new(actor_kind(kind), event);
    draft.actor_id = Some(key.to_string());
    draft.actor_name = Some(client.map_or_else(|| key_fingerprint(key), |c| c.name.clone()));
    draft
}

/// The log's word for a client of `kind`.
pub fn actor_kind(kind: ClientKind) -> ActorKind {
    match kind {
        ClientKind::Extension => ActorKind::Extension,
        ClientKind::Agent => ActorKind::Agent,
        ClientKind::Sidecar => ActorKind::Sidecar,
    }
}

/// `draft`, about `grant`: its id, the item when it names one, its first
/// target reduced to what the log may hold ([`audit_target_of`]), the mode,
/// the gate and who passed it, and the agent's own words for what it is
/// doing (`run_id`, `acting_for`).
pub fn about(mut draft: Draft, grant: &Grant, detail: Option<&str>) -> Draft {
    draft.grant_id = Some(grant.id.clone());
    draft.run_id = grant.run_id.clone();
    draft.acting_for = grant.acting_for.clone();
    draft.item_id = single_id(&grant.items);
    draft.target = grant.targets.first().map(audit_target_of);
    draft.mode = Some(first_mode(&grant.modes));
    draft.approval = Some(grant.approval);
    draft.approver = grant.approver.clone();
    draft.detail = detail.map(str::to_string);
    draft
}

/// Chain `draft` onto the lockbox's log as the row a release stands on: a
/// grant going active, a use — a login, a code, a value, an assertion — or
/// a standing rule, which is a release to come. The log fails closed: a row
/// that will not write is `AuditUnavailable`, and the caller releases
/// nothing, so there is no use the log does not hold. A release is written
/// `record(…)?;` and then made, in the same section as its decision.
pub fn record(store: &SqliteStore, writer_id: &str, draft: Draft, now_ms: i64) -> Result<(), Code> {
    Writer::new(writer_id, store)
        .record(draft, now_ms)
        .map(drop)
        .map_err(|e| {
            log::warn!("agents: could not write the audit row, so nothing is released: {e}");
            Code::AuditUnavailable
        })
}

/// Chain `draft` onto the lockbox's log as a row about something that
/// releases nothing: a refusal, an end, a client let in or forgotten, an ask
/// put up. A row that will not write is logged, and what it describes still
/// happens — failing closed means never releasing without a row, not
/// refusing to end what is live. A releasing event goes through [`record`].
pub fn note(store: &SqliteStore, writer_id: &str, draft: Draft, now_ms: i64) {
    debug_assert!(
        !releases(draft.event),
        "a {} row goes through `record`",
        draft.event.as_str()
    );
    if let Err(e) = Writer::new(writer_id, store).record(draft, now_ms) {
        log::warn!("agents: could not write the audit row: {e}");
    }
}

/// Whether a row of `event` is one a release stands on ([`record`]).
fn releases(event: Event) -> bool {
    matches!(
        event,
        Event::Granted | Event::Used | Event::RuleAdded | Event::KeyUnwrapped
    )
}

/// A URL as the log may hold it: scheme, host and path, never the query or
/// the fragment, which is where a page puts tokens and the like.
pub fn audit_target(url: &str) -> String {
    url.split(['?', '#']).next().unwrap_or_default().to_string()
}

/// The origin of the page a request named — scheme, host and port — which is
/// what a standing rule is written for and what the dialog shows: a rule is
/// about a site, not about the page's path, and never about its query
/// string. A URL that will not parse keeps what the log would keep.
pub fn site_origin(url: &str) -> String {
    match url::Url::parse(url).map(|u| u.origin()) {
        Ok(origin @ url::Origin::Tuple(..)) => origin.ascii_serialization(),
        _ => audit_target(url),
    }
}

/// What to call a client: the name the user gave it, or — for a key the open
/// lockbox never let in, which a stale association can be — its two ends, as
/// Settings shows a key.
pub fn client_label(clients: &[Client], key: &str) -> String {
    clients
        .iter()
        .find(|c| c.key == key)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| key_fingerprint(key))
}

fn key_fingerprint(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 12 {
        return key.to_string();
    }
    let head: String = chars[..8].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

/// A target as the log may hold it: an origin cut to scheme, host and path
/// ([`audit_target`]); an HTTP pattern as `host/path`, the methods being the
/// row's own business; a command line cut to the length a reason is.
pub fn audit_target_of(target: &Target) -> String {
    match target {
        Target::Origin(origin) => audit_target(origin),
        Target::Http { host, path, .. } => host_path(host, path),
        Target::Command(line) => clamped_line(line),
    }
}

/// A target as Settings shows it: an origin as written; an HTTP pattern as
/// `host/path`, led by the methods it names (`GET, POST api.stripe.com/v1/*`)
/// when it names any; a command line as written.
pub fn target_text(target: &Target) -> String {
    match target {
        Target::Origin(origin) => origin.clone(),
        Target::Http {
            host,
            path,
            methods,
        } if methods.is_empty() => host_path(host, path),
        Target::Http {
            host,
            path,
            methods,
        } => format!("{} {}", methods.join(", "), host_path(host, path)),
        Target::Command(line) => line.clone(),
    }
}

/// Where a sidecar used a value, as the log holds it: `host/path` of the
/// one request, never its query — a sidecar's path could carry one — or the
/// one command line, clamped.
pub fn resolve_target_text(target: &ResolveTarget) -> String {
    match target {
        ResolveTarget::Http { host, path, .. } => host_path(host, &audit_target(path)),
        ResolveTarget::Command(line) => clamped_line(line),
    }
}

/// `api.stripe.com/v1/balance`: no scheme, since a header target is always
/// reached over TLS, and one slash between host and path however the path
/// was written.
fn host_path(host: &str, path: &str) -> String {
    format!("{host}/{}", path.trim_start_matches('/'))
}

/// A command line is the agent's own words, bounded as its reason is
/// (`actions::MAX_REASON_CHARS`, 300) before it goes on a row.
fn clamped_line(line: &str) -> String {
    let mut line = line.to_string();
    clamp(&mut line, MAX_LINE_CHARS);
    line
}

const MAX_LINE_CHARS: usize = 300;

/// The origin among `targets`, when there is one: the site a fill grant is
/// for, which is what the extensions are told to sign out of.
pub fn origin_of(targets: &[Target]) -> Option<&str> {
    targets.iter().find_map(|t| match t {
        Target::Origin(origin) => Some(origin.as_str()),
        _ => None,
    })
}

/// The one item a selector names, when it names exactly one.
pub fn single_id(selector: &ItemSelector) -> Option<String> {
    match selector {
        ItemSelector::Ids(ids) if ids.len() == 1 => ids.first().cloned(),
        _ => None,
    }
}

/// A grant's or a rule's first mode, which is what a row says it is for. A
/// grant has at least one; one stored without any reads as a fill, the mode
/// there was before there were others.
pub(super) fn first_mode(modes: &[Mode]) -> Mode {
    modes.first().copied().unwrap_or(Mode::Fill)
}

/// The client a rule of the user's names, when it names one by key.
pub(super) fn rule_client(rule: &Rule) -> Option<String> {
    match &rule.agents {
        AgentSelector::Ids(ids) => ids.first().cloned(),
        _ => None,
    }
}
