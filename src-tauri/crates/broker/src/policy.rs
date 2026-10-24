//! Policy: which requests may become grants, and on whose say-so. A flat list
//! of rules evaluated deny-by-default, the most specific match winning and
//! the stricter of equally specific ones, under floors no rule can lower.
//! Asking is itself something policy grants: a request no rule covers is
//! denied, not asked, so an agent cannot spam approvers for items it was
//! never meant to touch, and a client that keeps being denied is slowed down
//! ([`RateState`]).
//!
//! [`evaluate`] is a pure function over data — no I/O, no clock — so the same
//! policy runs in the app, a headless broker and a server, and its tests are
//! the whole of its behaviour. Policy as code is a later layer on the same
//! interface, not a prerequisite.

use serde::{Deserialize, Serialize};

use crate::grant::{
    Approval, CandidateItem, CreatedBy, GrantRequest, ItemSelector, Mode, Probe, Target, Terms,
};
use crate::protocol::Code;

/// Which clients a rule is for. `Platform` names a client platform from the
/// `hello` of a Tier 2 association; a request carries no platform yet, so it
/// admits nothing until clients say what they are.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentSelector {
    Any,
    Ids(Vec<String>),
    Platform(String),
}

impl AgentSelector {
    pub fn admits(&self, client_key: &str, agent_id: Option<&str>) -> bool {
        match self {
            AgentSelector::Any => true,
            AgentSelector::Ids(ids) => ids
                .iter()
                .any(|id| id == client_key || agent_id.is_some_and(|agent| id == agent)),
            AgentSelector::Platform(_) => false,
        }
    }
}

/// Tier 2: whose tap a rule's asks go to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApproverSelector {
    Owner,
    Group(String),
    Principal,
}

/// One rule. An empty `targets` list means any target: the rule is about
/// who and what, not where. `expires_at_ms` is when a standing rule stops
/// applying ("for 7 days"); `None` is until revoked. [`evaluate`] does not
/// read the clock, so a host drops rules that are no longer
/// [`Rule::is_live`] before it calls.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    pub agents: AgentSelector,
    pub items: ItemSelector,
    pub targets: Vec<Target>,
    pub modes: Vec<Mode>,
    pub approval: Approval,
    /// How long a grant under this rule lives once active.
    pub ttl_ms: i64,
    pub max_uses: Option<u32>,
    pub reason_required: bool,
    pub approvers: Option<ApproverSelector>,
    pub sidecar_for: Option<Vec<String>>,
    pub created_at_ms: i64,
    pub created_by: CreatedBy,
    pub expires_at_ms: Option<i64>,
}

impl Rule {
    pub fn is_live(&self, now_ms: i64) -> bool {
        self.expires_at_ms.is_none_or(|at| now_ms < at)
    }
}

/// The minimums no rule can lower. Cards and identities, and anything at a
/// `money` target, take a tap plus biometrics; a passkey is never `auto`,
/// because the approver's tap is the user-presence gesture WebAuthn asks
/// for; `env` is at least `ask` unless a rule named the full command line,
/// because the child process can print what it was given.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Floors {
    /// `Http` targets a policy marks as money or identity.
    pub money: Vec<Target>,
}

impl Floors {
    /// `approval` raised to the floors that apply: to the items that will be
    /// served (`admitted`), the places asked for (`probes`) and the `modes`.
    /// `command_named` is whether a rule's `Target::Command` matched every
    /// command line asked for, which is what lifts the `env` floor.
    pub fn raise(
        &self,
        approval: Approval,
        admitted: &[&CandidateItem],
        probes: &[Probe],
        modes: &[Mode],
        command_named: bool,
    ) -> Approval {
        let mut floor = Approval::Auto;
        if admitted
            .iter()
            .any(|item| item.kind == "card" || item.kind == "identity")
        {
            floor = Approval::StepUp;
        }
        if modes.contains(&Mode::Passkey) {
            floor = floor.max(Approval::Ask);
        }
        if modes.contains(&Mode::Env) && !command_named {
            floor = floor.max(Approval::Ask);
        }
        if probes.iter().any(|probe| {
            matches!(probe, Probe::Http { .. }) && self.money.iter().any(|t| t.matches(probe))
        }) {
            floor = Approval::StepUp;
        }
        approval.max(floor)
    }
}

/// How often one client may reach a person. Rules may raise these for a
/// specific agent later; these are the defaults every client starts under.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    pub asks_per_minute: u32,
    pub denial_cooldown_base_ms: i64,
    pub denial_cooldown_max_ms: i64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            asks_per_minute: 10,
            denial_cooldown_base_ms: 30_000,
            denial_cooldown_max_ms: 3_600_000,
        }
    }
}

const MINUTE_MS: i64 = 60_000;
/// Denials in a row before a cool-down starts.
const DENIALS_BEFORE_COOLDOWN: u32 = 3;

/// One client's recent history, as far as the limits care: when it asked in
/// the last minute, and how many times in a row it was told no. The host
/// keeps one per client key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RateState {
    asks_ms: Vec<i64>,
    denials: u32,
    last_denial_ms: Option<i64>,
}

impl RateState {
    /// A request that reached a person.
    pub fn record_ask(&mut self, now_ms: i64) {
        self.asks_ms.retain(|&at| now_ms - at < MINUTE_MS);
        self.asks_ms.push(now_ms);
    }

    pub fn record_denial(&mut self, now_ms: i64) {
        self.denials = self.denials.saturating_add(1);
        self.last_denial_ms = Some(now_ms);
    }

    /// A yes ends the run of denials.
    pub fn record_grant(&mut self, _now_ms: i64) {
        self.denials = 0;
        self.last_denial_ms = None;
    }

    /// Whether this client may ask now; `RateLimited` otherwise.
    pub fn check(&self, limits: &Limits, now_ms: i64) -> Result<(), Code> {
        if self.retry_after_ms(limits, now_ms).is_some() {
            return Err(Code::RateLimited);
        }
        Ok(())
    }

    /// How long until this client may ask again, or `None` if it may now.
    /// The cool-down after `DENIALS_BEFORE_COOLDOWN` denials in a row starts
    /// at the base and doubles with each further denial, up to the cap.
    pub fn retry_after_ms(&self, limits: &Limits, now_ms: i64) -> Option<u64> {
        let mut wait: i64 = 0;
        let in_the_minute: Vec<i64> = self
            .asks_ms
            .iter()
            .copied()
            .filter(|&at| now_ms - at < MINUTE_MS)
            .collect();
        if in_the_minute.len() >= limits.asks_per_minute as usize {
            let oldest = in_the_minute.iter().copied().min().unwrap_or(now_ms);
            wait = wait.max(oldest + MINUTE_MS - now_ms);
        }
        if let (Some(last), true) = (self.last_denial_ms, self.denials >= DENIALS_BEFORE_COOLDOWN) {
            let doublings = (self.denials - DENIALS_BEFORE_COOLDOWN).min(62);
            let cooldown = limits
                .denial_cooldown_base_ms
                .saturating_mul(1i64 << doublings)
                .min(limits.denial_cooldown_max_ms);
            wait = wait.max(last + cooldown - now_ms);
        }
        (wait > 0).then_some(wait as u64)
    }
}

/// Why a request was denied. `RateLimited` is the host's to raise after
/// [`RateState::check`]: [`evaluate`] knows nothing about time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyReason {
    NoRule,
    NoCandidates,
    ReasonRequired,
    RateLimited,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    Denied(DenyReason),
    Allow {
        approval: Approval,
        rule_id: Option<String>,
        ttl_ms: i64,
        max_uses: Option<u32>,
        reason_required: bool,
    },
}

impl Decision {
    /// What a grant takes from an `Allow`; `None` for a denial. A grant under
    /// a rule is on record as the rule's.
    pub fn terms(&self) -> Option<Terms> {
        match self {
            Decision::Denied(_) => None,
            Decision::Allow {
                approval,
                rule_id,
                ttl_ms,
                max_uses,
                reason_required,
            } => Some(Terms {
                approval: *approval,
                ttl_ms: *ttl_ms,
                max_uses: *max_uses,
                reason_required: *reason_required,
                created_by: rule_id.clone().map_or(CreatedBy::User, CreatedBy::Policy),
            }),
        }
    }
}

/// Decide `req` against `rules`, for the `candidates` the request's selector
/// named and the `probes` its targets become.
///
/// A rule matches when its agent selector admits the client, its item
/// selector admits at least one candidate, every probe matches one of its
/// targets (an empty target list matching any), and every requested mode is
/// among its modes. The order of precedence, in full:
///
/// 1. No candidates, or no matching rule: denied.
/// 2. Among the matching rules, the most specific item selector wins:
///    item id > tag > kind > lockbox.
/// 3. Among the most specific, the stricter approval wins; the first listed
///    breaks a tie.
/// 4. A winning rule that requires a reason denies a request without one.
/// 5. [`Floors`] raise the approval over the items the winning rule admits;
///    nothing lowers it.
pub fn evaluate(
    rules: &[Rule],
    req: &GrantRequest,
    candidates: &[CandidateItem],
    probes: &[Probe],
    floors: &Floors,
) -> Decision {
    if candidates.is_empty() {
        return Decision::Denied(DenyReason::NoCandidates);
    }
    let matching: Vec<&Rule> = rules
        .iter()
        .filter(|rule| rule_matches(rule, req, candidates, probes))
        .collect();
    let Some(top) = matching.iter().map(|r| specificity(&r.items)).max() else {
        return Decision::Denied(DenyReason::NoRule);
    };
    let mut winner: Option<&Rule> = None;
    for rule in matching.iter().filter(|r| specificity(&r.items) == top) {
        if winner.is_none_or(|w| rule.approval > w.approval) {
            winner = Some(rule);
        }
    }
    let Some(rule) = winner else {
        return Decision::Denied(DenyReason::NoRule);
    };
    if rule.reason_required && req.reason.as_deref().is_none_or(|r| r.trim().is_empty()) {
        return Decision::Denied(DenyReason::ReasonRequired);
    }
    let admitted: Vec<&CandidateItem> = candidates
        .iter()
        .filter(|item| rule.items.admits(item))
        .collect();
    let approval = floors.raise(
        rule.approval,
        &admitted,
        probes,
        &req.modes,
        command_named(rule, probes),
    );
    Decision::Allow {
        approval,
        rule_id: Some(rule.id.clone()),
        ttl_ms: rule.ttl_ms,
        max_uses: rule.max_uses,
        reason_required: rule.reason_required,
    }
}

fn rule_matches(
    rule: &Rule,
    req: &GrantRequest,
    candidates: &[CandidateItem],
    probes: &[Probe],
) -> bool {
    rule.agents.admits(&req.client_key, req.agent_id.as_deref())
        && candidates.iter().any(|item| rule.items.admits(item))
        && (rule.targets.is_empty()
            || (!probes.is_empty()
                && probes
                    .iter()
                    .all(|probe| rule.targets.iter().any(|t| t.matches(probe)))))
        && !req.modes.is_empty()
        && req.modes.iter().all(|mode| rule.modes.contains(mode))
}

fn specificity(selector: &ItemSelector) -> u8 {
    match selector {
        ItemSelector::Ids(_) => 3,
        ItemSelector::Tag(_) => 2,
        ItemSelector::Kind(_) => 1,
        ItemSelector::Lockbox => 0,
    }
}

/// Whether a `Target::Command` of `rule` matched every command line among
/// `probes` — and there was at least one. A rule with no targets matches a
/// command by saying nothing about it, which is not naming it.
fn command_named(rule: &Rule, probes: &[Probe]) -> bool {
    let mut lines = probes.iter().filter_map(|probe| match probe {
        Probe::Command(line) => Some(*line),
        _ => None,
    });
    let Some(first) = lines.next() else {
        return false;
    };
    std::iter::once(first).chain(lines).all(|line| {
        rule.targets
            .iter()
            .any(|t| matches!(t, Target::Command(_)) && t.matches(&Probe::Command(line)))
    })
}

/// The rules a Tier 1 lockbox starts with, and the ones its dialogs write.
pub mod defaults {
    use super::*;

    /// The baseline rule's id, fixed so audit rows can name it and Settings
    /// can tell it from the rules the user wrote.
    pub const PERSONAL_FILL_RULE: &str = "personal-fill";
    /// How long a fill or passkey grant under the baseline lives once approved.
    pub const PERSONAL_TTL_MS: i64 = 15 * 60 * 1000;
    /// The dialog's default for "for N days".
    pub const DEFAULT_STANDING_DAYS: u32 = 7;
    const DAY_MS: i64 = 24 * 60 * 60 * 1000;

    /// The Tier 1 baseline the app prepends to a lockbox's own rules when its
    /// agent-access switch is on: any client may *ask* to fill or sign in
    /// with any item of the lockbox, anywhere, for the grant's window. Deny-by-default
    /// stays the evaluator's rule; this is what makes the first ever fill
    /// ask instead of fail.
    pub fn personal() -> Vec<Rule> {
        vec![Rule {
            id: PERSONAL_FILL_RULE.into(),
            agents: AgentSelector::Any,
            items: ItemSelector::Lockbox,
            targets: vec![],
            modes: vec![Mode::Fill, Mode::Passkey],
            approval: Approval::Ask,
            ttl_ms: PERSONAL_TTL_MS,
            // Not single-use: the fill is followed by the TOTP step under the
            // same grant, a username-first sign-in asks for the page's logins
            // twice, and a failed submit is retried. The window bounds it.
            max_uses: None,
            reason_required: false,
            approvers: None,
            sidecar_for: None,
            created_at_ms: 0,
            created_by: CreatedBy::User,
            expires_at_ms: None,
        }]
    }

    /// The rule the dialog's "for N days" answer writes: this client may fill
    /// this one item at this origin without asking, for `days` from `now_ms`.
    pub fn standing(client_key: &str, item_id: &str, origin: &str, days: u32, now_ms: i64) -> Rule {
        Rule {
            id: crate::grant::Grant::new_id(),
            agents: AgentSelector::Ids(vec![client_key.to_string()]),
            items: ItemSelector::Ids(vec![item_id.to_string()]),
            targets: vec![Target::Origin(origin.to_string())],
            modes: vec![Mode::Fill],
            approval: Approval::Auto,
            ttl_ms: PERSONAL_TTL_MS,
            max_uses: None,
            reason_required: false,
            approvers: None,
            sidecar_for: None,
            created_at_ms: now_ms,
            created_by: CreatedBy::User,
            expires_at_ms: Some(now_ms.saturating_add(i64::from(days).saturating_mul(DAY_MS))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, kind: &str, tags: &[&str]) -> CandidateItem {
        CandidateItem {
            id: id.into(),
            kind: kind.into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
        }
    }

    fn request(items: ItemSelector, targets: Vec<Target>, modes: Vec<Mode>) -> GrantRequest {
        GrantRequest {
            client_key: "ext-key".into(),
            agent_id: None,
            acting_for: None,
            items,
            targets,
            modes,
            run_id: None,
            reason: None,
        }
    }

    fn fill_request() -> GrantRequest {
        request(
            ItemSelector::Ids(vec!["gh".into()]),
            vec![Target::Origin("https://github.com".into())],
            vec![Mode::Fill],
        )
    }

    fn rule(id: &str, items: ItemSelector, approval: Approval) -> Rule {
        Rule {
            id: id.into(),
            agents: AgentSelector::Any,
            items,
            targets: vec![],
            modes: vec![Mode::Fill, Mode::Passkey, Mode::Header, Mode::Env],
            approval,
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

    const GH_PAGE: Probe<'static> = Probe::Origin("https://github.com/login");

    fn approval_of(decision: &Decision) -> Option<Approval> {
        match decision {
            Decision::Allow { approval, .. } => Some(*approval),
            Decision::Denied(_) => None,
        }
    }

    fn rule_of(decision: &Decision) -> Option<&str> {
        match decision {
            Decision::Allow { rule_id, .. } => rule_id.as_deref(),
            Decision::Denied(_) => None,
        }
    }

    // --- deny by default -------------------------------------------------------

    #[test]
    fn no_rule_means_denied_not_asked() {
        let gh = [item("gh", "login", &[])];
        assert_eq!(
            evaluate(&[], &fill_request(), &gh, &[GH_PAGE], &Floors::default()),
            Decision::Denied(DenyReason::NoRule)
        );
        let other_client = Rule {
            agents: AgentSelector::Ids(vec!["someone-else".into()]),
            ..rule("r", ItemSelector::Lockbox, Approval::Auto)
        };
        assert_eq!(
            evaluate(
                &[other_client],
                &fill_request(),
                &gh,
                &[GH_PAGE],
                &Floors::default()
            ),
            Decision::Denied(DenyReason::NoRule)
        );
        assert_eq!(
            evaluate(
                &defaults::personal(),
                &fill_request(),
                &[],
                &[GH_PAGE],
                &Floors::default()
            ),
            Decision::Denied(DenyReason::NoCandidates)
        );
    }

    #[test]
    fn the_personal_baseline_asks_for_a_fill_for_the_window() {
        let gh = [item("gh", "login", &[])];
        let decision = evaluate(
            &defaults::personal(),
            &fill_request(),
            &gh,
            &[GH_PAGE],
            &Floors::default(),
        );
        assert_eq!(
            decision,
            Decision::Allow {
                approval: Approval::Ask,
                rule_id: Some(defaults::PERSONAL_FILL_RULE.into()),
                ttl_ms: defaults::PERSONAL_TTL_MS,
                max_uses: None,
                reason_required: false,
            }
        );
        assert_eq!(
            decision.terms(),
            Some(Terms {
                approval: Approval::Ask,
                ttl_ms: defaults::PERSONAL_TTL_MS,
                max_uses: None,
                reason_required: false,
                created_by: CreatedBy::Policy(defaults::PERSONAL_FILL_RULE.into()),
            })
        );
        // Anywhere: the baseline names no target.
        let elsewhere = evaluate(
            &defaults::personal(),
            &request(
                ItemSelector::Ids(vec!["gh".into()]),
                vec![Target::Origin("https://gitlab.com".into())],
                vec![Mode::Fill],
            ),
            &gh,
            &[Probe::Origin("https://gitlab.com/users/sign_in")],
            &Floors::default(),
        );
        assert_eq!(approval_of(&elsewhere), Some(Approval::Ask));
        // But only to fill or sign in.
        let header = evaluate(
            &defaults::personal(),
            &request(
                ItemSelector::Ids(vec!["gh".into()]),
                vec![],
                vec![Mode::Header],
            ),
            &gh,
            &[],
            &Floors::default(),
        );
        assert_eq!(header, Decision::Denied(DenyReason::NoRule));
    }

    #[test]
    fn a_rule_must_cover_every_mode_and_every_probe_asked_for() {
        let gh = [item("gh", "login", &[])];
        let fill_only = Rule {
            modes: vec![Mode::Fill],
            ..rule("fill", ItemSelector::Lockbox, Approval::Auto)
        };
        let both = request(
            ItemSelector::Ids(vec!["gh".into()]),
            vec![Target::Origin("https://github.com".into())],
            vec![Mode::Fill, Mode::Passkey],
        );
        assert_eq!(
            evaluate(
                std::slice::from_ref(&fill_only),
                &both,
                &gh,
                &[GH_PAGE],
                &Floors::default()
            ),
            Decision::Denied(DenyReason::NoRule)
        );
        let github_only = Rule {
            targets: vec![Target::Origin("https://github.com".into())],
            ..fill_only
        };
        let two_sites = [GH_PAGE, Probe::Origin("https://gitlab.com/")];
        assert_eq!(
            evaluate(
                std::slice::from_ref(&github_only),
                &fill_request(),
                &gh,
                &two_sites,
                &Floors::default()
            ),
            Decision::Denied(DenyReason::NoRule)
        );
        assert_eq!(
            evaluate(
                std::slice::from_ref(&github_only),
                &fill_request(),
                &gh,
                &[],
                &Floors::default()
            ),
            Decision::Denied(DenyReason::NoRule),
            "a rule that names where does not match a request that names nowhere"
        );
        assert_eq!(
            approval_of(&evaluate(
                &[github_only],
                &fill_request(),
                &gh,
                &[GH_PAGE],
                &Floors::default()
            )),
            Some(Approval::Auto)
        );
        let no_modes = request(ItemSelector::Ids(vec!["gh".into()]), vec![], vec![]);
        assert_eq!(
            evaluate(
                &defaults::personal(),
                &no_modes,
                &gh,
                &[],
                &Floors::default()
            ),
            Decision::Denied(DenyReason::NoRule)
        );
    }

    // --- specificity -------------------------------------------------------------

    #[test]
    fn an_item_rule_beats_a_lockbox_rule_whatever_the_order() {
        let gh = [item("gh", "login", &["work"])];
        let lockbox = rule("lockbox", ItemSelector::Lockbox, Approval::StepUp);
        let kind = rule("kind", ItemSelector::Kind("login".into()), Approval::Ask);
        let tag = rule("tag", ItemSelector::Tag("work".into()), Approval::StepUp);
        let id = rule("id", ItemSelector::Ids(vec!["gh".into()]), Approval::Auto);

        let forward = evaluate(
            &[lockbox.clone(), kind.clone(), tag.clone(), id.clone()],
            &fill_request(),
            &gh,
            &[GH_PAGE],
            &Floors::default(),
        );
        assert_eq!(rule_of(&forward), Some("id"));
        assert_eq!(approval_of(&forward), Some(Approval::Auto));

        let backward = evaluate(
            &[id, tag.clone(), kind.clone(), lockbox.clone()],
            &fill_request(),
            &gh,
            &[GH_PAGE],
            &Floors::default(),
        );
        assert_eq!(rule_of(&backward), Some("id"));

        let without_id = evaluate(
            &[lockbox.clone(), kind.clone(), tag],
            &fill_request(),
            &gh,
            &[GH_PAGE],
            &Floors::default(),
        );
        assert_eq!(rule_of(&without_id), Some("tag"));
        let without_tag = evaluate(
            &[lockbox, kind],
            &fill_request(),
            &gh,
            &[GH_PAGE],
            &Floors::default(),
        );
        assert_eq!(rule_of(&without_tag), Some("kind"));
    }

    #[test]
    fn among_equally_specific_rules_the_stricter_wins() {
        let gh = [item("gh", "login", &[])];
        let auto = rule("auto", ItemSelector::Ids(vec!["gh".into()]), Approval::Auto);
        let ask = rule("ask", ItemSelector::Ids(vec!["gh".into()]), Approval::Ask);
        let step_up = rule(
            "step-up",
            ItemSelector::Ids(vec!["gh".into()]),
            Approval::StepUp,
        );
        let decision = evaluate(
            &[auto.clone(), step_up.clone(), ask.clone()],
            &fill_request(),
            &gh,
            &[GH_PAGE],
            &Floors::default(),
        );
        assert_eq!(rule_of(&decision), Some("step-up"));
        let decision = evaluate(
            &[ask, auto],
            &fill_request(),
            &gh,
            &[GH_PAGE],
            &Floors::default(),
        );
        assert_eq!(rule_of(&decision), Some("ask"));
    }

    #[test]
    fn a_rule_that_requires_a_reason_denies_a_request_without_one() {
        let gh = [item("gh", "login", &[])];
        let rules = [Rule {
            reason_required: true,
            ..rule("r", ItemSelector::Lockbox, Approval::Auto)
        }];
        assert_eq!(
            evaluate(&rules, &fill_request(), &gh, &[GH_PAGE], &Floors::default()),
            Decision::Denied(DenyReason::ReasonRequired)
        );
        let blank = GrantRequest {
            reason: Some("  ".into()),
            ..fill_request()
        };
        assert_eq!(
            evaluate(&rules, &blank, &gh, &[GH_PAGE], &Floors::default()),
            Decision::Denied(DenyReason::ReasonRequired)
        );
        let with_reason = GrantRequest {
            reason: Some("deploying".into()),
            ..fill_request()
        };
        assert_eq!(
            evaluate(&rules, &with_reason, &gh, &[GH_PAGE], &Floors::default()),
            Decision::Allow {
                approval: Approval::Auto,
                rule_id: Some("r".into()),
                ttl_ms: 1_000,
                max_uses: None,
                reason_required: true,
            }
        );
    }

    // --- floors ----------------------------------------------------------------

    #[test]
    fn cards_and_identities_are_stepped_up_whatever_the_rule_says() {
        let rules = [rule("all", ItemSelector::Lockbox, Approval::Auto)];
        for kind in ["card", "identity"] {
            let items = [item("x", kind, &[])];
            let req = request(
                ItemSelector::Ids(vec!["x".into()]),
                vec![],
                vec![Mode::Fill],
            );
            let decision = evaluate(&rules, &req, &items, &[GH_PAGE], &Floors::default());
            assert_eq!(approval_of(&decision), Some(Approval::StepUp), "{kind}");
        }
        // The floor is over what the winning rule admits, not over every
        // candidate: a login-only rule serving a login beside a card asks
        // for the login alone.
        let login_only = [rule(
            "logins",
            ItemSelector::Kind("login".into()),
            Approval::Ask,
        )];
        let mixed = [item("gh", "login", &[]), item("visa", "card", &[])];
        let req = request(ItemSelector::Lockbox, vec![], vec![Mode::Fill]);
        let decision = evaluate(&login_only, &req, &mixed, &[GH_PAGE], &Floors::default());
        assert_eq!(approval_of(&decision), Some(Approval::Ask));
    }

    #[test]
    fn a_passkey_is_never_auto() {
        let gh = [item("gh", "login", &[])];
        let rules = [rule("all", ItemSelector::Lockbox, Approval::Auto)];
        let req = request(
            ItemSelector::Ids(vec!["gh".into()]),
            vec![Target::Origin("https://github.com".into())],
            vec![Mode::Passkey],
        );
        let decision = evaluate(&rules, &req, &gh, &[GH_PAGE], &Floors::default());
        assert_eq!(approval_of(&decision), Some(Approval::Ask));
        let strict = [rule("all", ItemSelector::Lockbox, Approval::StepUp)];
        let decision = evaluate(&strict, &req, &gh, &[GH_PAGE], &Floors::default());
        assert_eq!(
            approval_of(&decision),
            Some(Approval::StepUp),
            "never lowered"
        );
    }

    #[test]
    fn env_asks_unless_a_rule_named_the_full_command_line() {
        let key = [item("stripe", "apikey", &[])];
        let env_request = |line: &str| {
            request(
                ItemSelector::Ids(vec!["stripe".into()]),
                vec![Target::Command(line.to_string())],
                vec![Mode::Env],
            )
        };
        let anywhere = [rule("any", ItemSelector::Lockbox, Approval::Auto)];
        let deploy = Probe::Command("node deploy.js --prod");
        let decision = evaluate(
            &anywhere,
            &env_request("node deploy.js --prod"),
            &key,
            &[deploy],
            &Floors::default(),
        );
        assert_eq!(
            approval_of(&decision),
            Some(Approval::Ask),
            "no target named the line"
        );

        let named = [Rule {
            targets: vec![Target::Command("node deploy.js*".into())],
            ..rule("deploy", ItemSelector::Lockbox, Approval::Auto)
        }];
        let decision = evaluate(
            &named,
            &env_request("node deploy.js --prod"),
            &key,
            &[deploy],
            &Floors::default(),
        );
        assert_eq!(
            approval_of(&decision),
            Some(Approval::Auto),
            "the rule named it"
        );

        let eval = Probe::Command("node -e \"console.log(process.env)\"");
        let decision = evaluate(
            &named,
            &env_request("node -e …"),
            &key,
            &[eval],
            &Floors::default(),
        );
        assert_eq!(
            decision,
            Decision::Denied(DenyReason::NoRule),
            "argv[0] alone is not a match"
        );

        let no_line = evaluate(
            &anywhere,
            &request(
                ItemSelector::Ids(vec!["stripe".into()]),
                vec![],
                vec![Mode::Env],
            ),
            &key,
            &[],
            &Floors::default(),
        );
        assert_eq!(
            approval_of(&no_line),
            Some(Approval::Ask),
            "no command line at all"
        );
    }

    #[test]
    fn a_money_target_is_stepped_up() {
        let key = [item("stripe", "apikey", &[])];
        let floors = Floors {
            money: vec![Target::Http {
                host: "api.stripe.com".into(),
                path: "/v1/charges*".into(),
                methods: vec!["POST".into()],
            }],
        };
        let rules = [rule("all", ItemSelector::Lockbox, Approval::Auto)];
        let req = request(
            ItemSelector::Ids(vec!["stripe".into()]),
            vec![],
            vec![Mode::Header],
        );
        let charge = Probe::Http {
            host: "api.stripe.com",
            path: "/v1/charges",
            method: "POST",
        };
        let decision = evaluate(&rules, &req, &key, &[charge], &floors);
        assert_eq!(approval_of(&decision), Some(Approval::StepUp));
        let balance = Probe::Http {
            host: "api.stripe.com",
            path: "/v1/balance",
            method: "GET",
        };
        let decision = evaluate(&rules, &req, &key, &[balance], &floors);
        assert_eq!(approval_of(&decision), Some(Approval::Auto));
    }

    #[test]
    fn empty_targets_on_a_rule_mean_any_target() {
        let gh = [item("gh", "login", &[])];
        let rules = [rule("any", ItemSelector::Lockbox, Approval::Auto)];
        for probe in [
            GH_PAGE,
            Probe::Origin("https://example.org/"),
            Probe::Http {
                host: "api.example.org",
                path: "/",
                method: "GET",
            },
        ] {
            let req = request(
                ItemSelector::Ids(vec!["gh".into()]),
                vec![],
                vec![Mode::Fill],
            );
            let decision = evaluate(&rules, &req, &gh, &[probe], &Floors::default());
            assert_eq!(approval_of(&decision), Some(Approval::Auto), "{probe:?}");
        }
    }

    #[test]
    fn the_standing_rule_is_one_client_one_item_one_origin_for_n_days() {
        let now = 1_700_000_000_000;
        let standing = defaults::standing("ext-key", "gh", "https://github.com", 7, now);
        assert_eq!(standing.agents, AgentSelector::Ids(vec!["ext-key".into()]));
        assert_eq!(standing.items, ItemSelector::Ids(vec!["gh".into()]));
        assert_eq!(
            standing.targets,
            vec![Target::Origin("https://github.com".into())]
        );
        assert_eq!(standing.approval, Approval::Auto);
        assert_eq!(standing.expires_at_ms, Some(now + 7 * 24 * 60 * 60 * 1000));
        assert!(standing.is_live(now + 6 * 24 * 60 * 60 * 1000));
        assert!(!standing.is_live(now + 7 * 24 * 60 * 60 * 1000));
        assert!(defaults::personal()[0].is_live(i64::MAX));

        let gh = [item("gh", "login", &[])];
        let mut rules = defaults::personal();
        rules.push(standing);
        let decision = evaluate(&rules, &fill_request(), &gh, &[GH_PAGE], &Floors::default());
        assert_eq!(approval_of(&decision), Some(Approval::Auto));
        let other_client = GrantRequest {
            client_key: "another".into(),
            ..fill_request()
        };
        let decision = evaluate(&rules, &other_client, &gh, &[GH_PAGE], &Floors::default());
        assert_eq!(approval_of(&decision), Some(Approval::Ask));
    }

    #[test]
    fn an_agent_selector_admits_by_client_key_or_agent_id() {
        let ids = AgentSelector::Ids(vec!["key-1".into(), "agent-9".into()]);
        assert!(ids.admits("key-1", None));
        assert!(ids.admits("other", Some("agent-9")));
        assert!(!ids.admits("other", Some("agent-1")));
        assert!(AgentSelector::Any.admits("anyone", None));
        assert!(!AgentSelector::Platform("claude-code".into()).admits("key-1", Some("agent-9")));
    }

    // --- rate limits -------------------------------------------------------------

    #[test]
    fn the_eleventh_ask_in_a_minute_is_refused() {
        let limits = Limits::default();
        let mut state = RateState::default();
        for i in 0..10 {
            assert_eq!(state.check(&limits, i * 1_000), Ok(()), "ask {i}");
            state.record_ask(i * 1_000);
        }
        assert_eq!(state.check(&limits, 10_000), Err(Code::RateLimited));
        assert_eq!(state.retry_after_ms(&limits, 10_000), Some(50_000));
        assert_eq!(state.check(&limits, 59_999), Err(Code::RateLimited));
        assert_eq!(
            state.check(&limits, 60_000),
            Ok(()),
            "the first ask aged out"
        );
    }

    #[test]
    fn three_denials_start_a_cool_down_that_doubles_and_caps() {
        let limits = Limits::default();
        let mut state = RateState::default();
        state.record_denial(0);
        state.record_denial(0);
        assert_eq!(state.check(&limits, 0), Ok(()), "two is not yet a run");
        state.record_denial(0);
        assert_eq!(state.check(&limits, 0), Err(Code::RateLimited));
        assert_eq!(state.retry_after_ms(&limits, 0), Some(30_000));
        assert_eq!(state.check(&limits, 30_000), Ok(()));

        state.record_denial(30_000);
        assert_eq!(state.retry_after_ms(&limits, 30_000), Some(60_000));
        state.record_denial(90_000);
        assert_eq!(state.retry_after_ms(&limits, 90_000), Some(120_000));

        let mut many = RateState::default();
        for _ in 0..40 {
            many.record_denial(0);
        }
        assert_eq!(many.retry_after_ms(&limits, 0), Some(3_600_000), "capped");
        assert_eq!(many.check(&limits, 3_600_000), Ok(()));
    }

    #[test]
    fn a_grant_ends_the_run_of_denials() {
        let limits = Limits::default();
        let mut state = RateState::default();
        for _ in 0..3 {
            state.record_denial(0);
        }
        assert_eq!(state.check(&limits, 0), Err(Code::RateLimited));
        state.record_grant(1);
        assert_eq!(state.check(&limits, 1), Ok(()));
        state.record_denial(2);
        state.record_denial(2);
        assert_eq!(state.check(&limits, 2), Ok(()), "the count started over");
    }
}
