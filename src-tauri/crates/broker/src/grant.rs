//! The grant: *this client* may use *these items* at *these targets* in
//! *these modes* until *this time*. Nothing else confers access to an agent —
//! a fill consumes one, a header swap checks one, a standing permission is one
//! with a long life — so every tier speaks the same vocabulary and a policy
//! tested on a laptop reads the same in CI.
//!
//! Everything here is data and pure functions over it. Time is always a
//! `now_ms` the caller passes in, never the clock, so a host can reason about
//! expiry under its own lock and a test can pin the moment. The serde shapes
//! are the wire and the stored form at once: camelCase fields, lowercase
//! variants, `approval` as the spec writes it (`step_up`), so a reply, a row
//! under `meta` and an audit column all read the same.

use serde::{Deserialize, Serialize};

use crate::protocol::Code;
use rowel_core::crypto::random_hex_id;
use rowel_core::host::site_matches;

/// How a secret is put to use without being handed over (`spec/05`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Fill,
    Passkey,
    Header,
    Token,
    Env,
}

impl Mode {
    /// The wire word, which is also the audit column's.
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Fill => "fill",
            Mode::Passkey => "passkey",
            Mode::Header => "header",
            Mode::Token => "token",
            Mode::Env => "env",
        }
    }
}

/// Who has to say yes. Ordered so that "stricter" is comparable: `Auto < Ask
/// < StepUp`, and a floor raises an approval with `max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Approval {
    Auto,
    Ask,
    StepUp,
}

impl Approval {
    /// The wire word, which is also the audit column's.
    pub fn as_str(self) -> &'static str {
        match self {
            Approval::Auto => "auto",
            Approval::Ask => "ask",
            Approval::StepUp => "step_up",
        }
    }
}

/// Where a grant applies. Each kind is matched against the same kind of
/// [`Probe`] and against nothing else.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    /// `fill` / `passkey`: a site, matched the way a login's website field is
    /// matched to a page (`rowel_core::host::site_matches`): the host or a
    /// parent of it, the scheme when one was written, the port when one was.
    Origin(String),
    /// `header` / `token`: a host — exact, or `*.` for the subdomains of one —
    /// a path glob (`*` runs over any characters, `/` included) and the
    /// methods allowed, none meaning any.
    Http {
        host: String,
        path: String,
        methods: Vec<String>,
    },
    /// `env`: a glob over the *full* command line, never `argv[0]` alone, so a
    /// rule for `node deploy.js*` says nothing about `node -e …`.
    Command(String),
}

/// What is being asked for, as the host saw it: a page, a request, a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe<'a> {
    Origin(&'a str),
    Http {
        host: &'a str,
        path: &'a str,
        method: &'a str,
    },
    Command(&'a str),
}

impl Target {
    pub fn matches(&self, probe: &Probe) -> bool {
        match (self, probe) {
            (Target::Origin(saved), Probe::Origin(page_url)) => site_matches(page_url, saved),
            (
                Target::Http {
                    host,
                    path,
                    methods,
                },
                Probe::Http {
                    host: probe_host,
                    path: probe_path,
                    method,
                },
            ) => {
                host_pattern_matches(host, probe_host)
                    && glob_matches(path, probe_path)
                    && (methods.is_empty()
                        || methods.iter().any(|m| m.eq_ignore_ascii_case(method)))
            }
            (Target::Command(pattern), Probe::Command(line)) => glob_matches(pattern, line),
            _ => false,
        }
    }
}

/// `*.example.com` is the subdomains of `example.com`, not the apex: a rule
/// that wants both names both. Hosts compare case-insensitively.
fn host_pattern_matches(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(parent) => {
            let (host, parent) = (host.as_bytes(), parent.as_bytes());
            host.len() > parent.len() + 1
                && host[host.len() - parent.len() - 1] == b'.'
                && host[host.len() - parent.len()..].eq_ignore_ascii_case(parent)
        }
        None => pattern.eq_ignore_ascii_case(host),
    }
}

/// Whether `text` is one of the strings `pattern` names, `*` standing for any
/// run of characters (including none). The classic two-pointer walk with one
/// backtrack point: a `*` is as greedy as the rest of the pattern lets it be.
/// Over bytes, which is sound for UTF-8 — a literal byte of the pattern only
/// matches its equal, so a match never lands inside a multi-byte character.
pub fn glob_matches(pattern: &str, text: &str) -> bool {
    let (pattern, text) = (pattern.as_bytes(), text.as_bytes());
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && pattern[p] == b'*' {
            star = Some((p, t));
            p += 1;
        } else if p < pattern.len() && pattern[p] == text[t] {
            p += 1;
            t += 1;
        } else if let Some((star_p, star_t)) = star {
            p = star_p + 1;
            t = star_t + 1;
            star = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&b| b == b'*')
}

/// Which items a grant or a rule names: these ids, everything in the lockbox,
/// everything carrying a tag, everything of a kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemSelector {
    Ids(Vec<String>),
    Lockbox,
    Tag(String),
    Kind(String),
}

/// An item as a selector sees it: nothing secret, nothing the list does not
/// already show.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateItem {
    pub id: String,
    pub kind: String,
    pub tags: Vec<String>,
}

impl ItemSelector {
    pub fn admits(&self, item: &CandidateItem) -> bool {
        match self {
            ItemSelector::Ids(ids) => ids.contains(&item.id),
            ItemSelector::Lockbox => true,
            ItemSelector::Tag(tag) => item.tags.iter().any(|t| t == tag),
            ItemSelector::Kind(kind) => item.kind == *kind,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Pending,
    Active,
    Spent,
    Expired,
    Revoked,
    Denied,
}

/// Who put a grant or a rule in place: the user at a dialog, a rule by its
/// id, an administrator by their member id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CreatedBy {
    User,
    Policy(String),
    Admin(String),
}

/// What a client asks for. `client_key` is the association key the request
/// came in under (the connection's, not the message's: a client cannot ask on
/// another's behalf); `agent_id` is the agent definition in Tier 2.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantRequest {
    pub client_key: String,
    pub agent_id: Option<String>,
    pub acting_for: Option<String>,
    pub items: ItemSelector,
    pub targets: Vec<Target>,
    pub modes: Vec<Mode>,
    pub run_id: Option<String>,
    pub reason: Option<String>,
}

/// What a grant takes from the rule that allowed it rather than from the
/// request: the gate, the limits, and who is on record as having set them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Terms {
    pub approval: Approval,
    pub ttl_ms: i64,
    pub max_uses: Option<u32>,
    pub reason_required: bool,
    pub created_by: CreatedBy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Grant {
    /// Unguessable: 128 random bits, hex. The id is the capability an agent
    /// hands a sidecar, so it has to be.
    pub id: String,
    pub lockbox_id: String,
    pub client_key: String,
    pub agent_id: Option<String>,
    pub acting_for: Option<String>,
    pub items: ItemSelector,
    pub targets: Vec<Target>,
    pub modes: Vec<Mode>,
    pub approval: Approval,
    pub reason_required: bool,
    pub not_before_ms: i64,
    pub expires_at_ms: i64,
    pub max_uses: Option<u32>,
    pub uses: u32,
    pub run_id: Option<String>,
    pub reason: Option<String>,
    pub approver: Option<String>,
    pub created_by: CreatedBy,
    pub state: State,
}

impl Grant {
    /// A fresh grant id: 16 random bytes, hex, like every id the app mints.
    pub fn new_id() -> String {
        random_hex_id()
    }

    /// The grant a request becomes once policy has allowed it: `Pending`,
    /// valid from `now_ms` for `terms.ttl_ms`, with no uses and no approver
    /// yet. The host activates it when the approver says yes (or at once
    /// under `Auto`), or marks it `Denied`.
    pub fn from_request(lockbox_id: &str, req: GrantRequest, terms: Terms, now_ms: i64) -> Self {
        Self {
            id: Self::new_id(),
            lockbox_id: lockbox_id.to_string(),
            client_key: req.client_key,
            agent_id: req.agent_id,
            acting_for: req.acting_for,
            items: req.items,
            targets: req.targets,
            modes: req.modes,
            approval: terms.approval,
            reason_required: terms.reason_required,
            not_before_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(terms.ttl_ms),
            max_uses: terms.max_uses,
            uses: 0,
            run_id: req.run_id,
            reason: req.reason,
            approver: None,
            created_by: terms.created_by,
            state: State::Pending,
        }
    }

    /// `Active`, inside its window, and not used up.
    pub fn is_active(&self, now_ms: i64) -> bool {
        self.state == State::Active
            && now_ms >= self.not_before_ms
            && now_ms < self.expires_at_ms
            && self.max_uses.is_none_or(|max| self.uses < max)
    }

    /// Whether this grant, right now, lets `item` be used at `probe` in
    /// `mode`. A grant with no targets covers nothing: the request named
    /// where, and a grant is never wider than its request.
    pub fn covers(&self, item: &CandidateItem, probe: &Probe, mode: Mode, now_ms: i64) -> bool {
        self.is_active(now_ms)
            && self.modes.contains(&mode)
            && self.items.admits(item)
            && self.targets.iter().any(|t| t.matches(probe))
    }

    /// Count one use: a fill served, a placeholder resolved, an assertion
    /// signed. The last allowed use flips the grant to `Spent`. A grant that
    /// is used up or past its window refuses with `GrantExpired`; one that
    /// was never, or is no longer, allowed refuses with `GrantDenied`.
    pub fn use_once(&mut self, now_ms: i64) -> Result<(), Code> {
        self.expire_if_due(now_ms);
        match self.state {
            State::Active => {}
            State::Spent | State::Expired => return Err(Code::GrantExpired),
            State::Pending | State::Revoked | State::Denied => return Err(Code::GrantDenied),
        }
        if !self.is_active(now_ms) {
            return Err(Code::GrantDenied);
        }
        self.uses += 1;
        if self.max_uses.is_some_and(|max| self.uses >= max) {
            self.state = State::Spent;
        }
        Ok(())
    }

    /// Flip an `Active` grant whose window has closed to `Expired`. True
    /// when this call did the flipping, so a host sweeping its grants knows
    /// which ones to write an `ended` row for.
    pub fn expire_if_due(&mut self, now_ms: i64) -> bool {
        if self.state == State::Active && now_ms >= self.expires_at_ms {
            self.state = State::Expired;
            return true;
        }
        false
    }
}

/// An item as a client may see it: enough to tell the user which account
/// will be used, nothing that could be used without the grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibleItem {
    pub id: String,
    pub title: String,
    pub username: Option<String>,
}

/// The answer to a grant request: the grant's id and state, when it ends,
/// when to ask again if the consent slot was busy, and the items it admits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantReply {
    pub grant_id: String,
    pub state: State,
    pub expires_at_ms: Option<i64>,
    pub retry_after_ms: Option<u64>,
    pub items: Vec<VisibleItem>,
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

    fn request() -> GrantRequest {
        GrantRequest {
            client_key: "ext-key".into(),
            agent_id: None,
            acting_for: None,
            items: ItemSelector::Ids(vec!["gh".into()]),
            targets: vec![Target::Origin("https://github.com".into())],
            modes: vec![Mode::Fill],
            run_id: None,
            reason: None,
        }
    }

    fn terms(ttl_ms: i64, max_uses: Option<u32>) -> Terms {
        Terms {
            approval: Approval::Ask,
            ttl_ms,
            max_uses,
            reason_required: false,
            created_by: CreatedBy::User,
        }
    }

    fn active(now_ms: i64, ttl_ms: i64, max_uses: Option<u32>) -> Grant {
        Grant {
            state: State::Active,
            ..Grant::from_request("lb", request(), terms(ttl_ms, max_uses), now_ms)
        }
    }

    // --- targets -------------------------------------------------------------

    #[test]
    fn an_origin_target_matches_the_way_a_login_matches_a_page() {
        let saved = Target::Origin("https://example.com".into());
        assert!(saved.matches(&Probe::Origin("https://example.com/login")));
        assert!(saved.matches(&Probe::Origin("https://accounts.example.com/")));
        assert!(
            !saved.matches(&Probe::Origin("http://example.com/")),
            "scheme"
        );
        assert!(
            !saved.matches(&Probe::Origin("https://notexample.com/")),
            "lookalike"
        );
        assert!(
            !saved.matches(&Probe::Origin("https://example.com.evil.io/")),
            "a parent must be a suffix at a label boundary"
        );

        let bare = Target::Origin("example.com".into());
        assert!(bare.matches(&Probe::Origin("http://example.com/")));

        let ported = Target::Origin("https://example.com:8443".into());
        assert!(ported.matches(&Probe::Origin("https://example.com:8443/")));
        assert!(
            !ported.matches(&Probe::Origin("https://example.com/")),
            "port"
        );

        assert!(!saved.matches(&Probe::Command("curl https://example.com")));
    }

    #[test]
    fn an_http_target_matches_host_path_and_method_patterns() {
        let target = Target::Http {
            host: "api.stripe.com".into(),
            path: "/v1/charges*".into(),
            methods: vec!["GET".into(), "post".into()],
        };
        let probe = |host, path, method| Probe::Http { host, path, method };
        assert!(target.matches(&probe("api.stripe.com", "/v1/charges", "GET")));
        assert!(target.matches(&probe("API.Stripe.com", "/v1/charges/ch_1", "POST")));
        assert!(!target.matches(&probe("api.stripe.com", "/v1/charges", "DELETE")));
        assert!(!target.matches(&probe("api.stripe.com", "/v1/customers", "GET")));
        assert!(!target.matches(&probe("stripe.com", "/v1/charges", "GET")));

        let any_method = Target::Http {
            host: "*.example.com".into(),
            path: "/*".into(),
            methods: vec![],
        };
        assert!(any_method.matches(&probe("api.example.com", "/anything/at/all", "PATCH")));
        assert!(any_method.matches(&probe("a.b.example.com", "/", "GET")));
        assert!(
            !any_method.matches(&probe("example.com", "/", "GET")),
            "`*.` names the subdomains, not the apex"
        );
        assert!(!any_method.matches(&probe("notexample.com", "/", "GET")));
        assert!(!any_method.matches(&probe("api.example.com.evil.io", "/", "GET")));
    }

    #[test]
    fn a_command_target_is_a_glob_over_the_whole_line() {
        let deploy = Target::Command("node deploy.js*".into());
        assert!(deploy.matches(&Probe::Command("node deploy.js")));
        assert!(deploy.matches(&Probe::Command("node deploy.js --prod")));
        assert!(!deploy.matches(&Probe::Command("node -e \"console.log(process.env)\"")));
        assert!(!deploy.matches(&Probe::Command("node")));
        assert!(!deploy.matches(&Probe::Command("sh -c 'node deploy.js'")));

        let middle = Target::Command("stripe * --live".into());
        assert!(middle.matches(&Probe::Command("stripe charges list --live")));
        assert!(!middle.matches(&Probe::Command("stripe charges list")));
    }

    #[test]
    fn the_glob_handles_the_edges() {
        assert!(glob_matches("", ""));
        assert!(glob_matches("*", ""));
        assert!(glob_matches("**", "anything"));
        assert!(!glob_matches("", "x"));
        assert!(glob_matches("a*b*c", "a-b-c"));
        assert!(glob_matches("a*b*c", "abc"));
        assert!(!glob_matches("a*b*c", "a-c-b"));
        assert!(glob_matches("*.js", "deploy.js"));
        assert!(!glob_matches("*.js", "deploy.ts"));
        assert!(glob_matches("ключ*", "ключ-密码"));
        assert!(!glob_matches("ключ*", "клю"));
    }

    // --- selectors -----------------------------------------------------------

    #[test]
    fn a_selector_admits_by_id_tag_kind_or_everything() {
        let login = item("gh", "login", &["work"]);
        let card = item("visa", "card", &[]);
        assert!(ItemSelector::Ids(vec!["gh".into()]).admits(&login));
        assert!(!ItemSelector::Ids(vec!["gh".into()]).admits(&card));
        assert!(ItemSelector::Tag("work".into()).admits(&login));
        assert!(!ItemSelector::Tag("work".into()).admits(&card));
        assert!(ItemSelector::Kind("card".into()).admits(&card));
        assert!(!ItemSelector::Kind("card".into()).admits(&login));
        assert!(ItemSelector::Lockbox.admits(&login) && ItemSelector::Lockbox.admits(&card));
    }

    // --- the grant's life ----------------------------------------------------

    #[test]
    fn a_grant_is_active_inside_its_window_and_in_the_active_state() {
        let grant = active(1_000, 500, None);
        assert!(!grant.is_active(999), "not before");
        assert!(grant.is_active(1_000));
        assert!(grant.is_active(1_499));
        assert!(!grant.is_active(1_500), "expires_at is exclusive");
        assert!(!Grant {
            state: State::Pending,
            ..grant.clone()
        }
        .is_active(1_100));
        assert!(!Grant {
            state: State::Revoked,
            ..grant
        }
        .is_active(1_100));
    }

    #[test]
    fn a_grant_is_spent_at_its_last_use() {
        let mut grant = active(0, 1_000, Some(2));
        assert_eq!(grant.use_once(10), Ok(()));
        assert_eq!(grant.state, State::Active);
        assert_eq!(grant.use_once(20), Ok(()));
        assert_eq!((grant.state, grant.uses), (State::Spent, 2));
        assert_eq!(grant.use_once(30), Err(Code::GrantExpired));
        assert_eq!(grant.uses, 2, "a refused use is not counted");
    }

    #[test]
    fn an_unlimited_grant_counts_its_uses_and_stays_active() {
        let mut grant = active(0, 1_000, None);
        for _ in 0..5 {
            grant.use_once(1).unwrap();
        }
        assert_eq!((grant.state, grant.uses), (State::Active, 5));
    }

    #[test]
    fn a_use_past_the_window_expires_the_grant_and_refuses() {
        let mut grant = active(0, 1_000, None);
        assert_eq!(grant.use_once(1_000), Err(Code::GrantExpired));
        assert_eq!(grant.state, State::Expired);
        assert_eq!(grant.use_once(5), Err(Code::GrantExpired), "and stays so");
    }

    #[test]
    fn a_use_of_a_grant_nobody_allowed_is_denied() {
        for state in [State::Pending, State::Revoked, State::Denied] {
            let mut grant = Grant {
                state,
                ..active(0, 1_000, None)
            };
            assert_eq!(grant.use_once(10), Err(Code::GrantDenied), "{state:?}");
            assert_eq!(grant.state, state);
        }
        let mut early = active(1_000, 1_000, None);
        assert_eq!(early.use_once(10), Err(Code::GrantDenied), "not yet valid");
    }

    #[test]
    fn expire_if_due_flips_once_and_only_an_active_grant() {
        let mut grant = active(0, 1_000, None);
        assert!(!grant.expire_if_due(999));
        assert!(grant.expire_if_due(1_000));
        assert_eq!(grant.state, State::Expired);
        assert!(!grant.expire_if_due(2_000), "already flipped");

        let mut spent = Grant {
            state: State::Spent,
            ..active(0, 1_000, Some(1))
        };
        assert!(!spent.expire_if_due(5_000));
        assert_eq!(spent.state, State::Spent);
    }

    #[test]
    fn covers_needs_the_item_the_target_the_mode_and_the_moment() {
        let grant = active(0, 1_000, None);
        let gh = item("gh", "login", &[]);
        let page = Probe::Origin("https://github.com/login");
        assert!(grant.covers(&gh, &page, Mode::Fill, 10));
        assert!(!grant.covers(&gh, &page, Mode::Passkey, 10), "mode");
        assert!(!grant.covers(&item("other", "login", &[]), &page, Mode::Fill, 10));
        assert!(!grant.covers(&gh, &Probe::Origin("https://gitlab.com/"), Mode::Fill, 10));
        assert!(!grant.covers(&gh, &page, Mode::Fill, 1_000), "expired");
        let targetless = Grant {
            targets: vec![],
            ..grant
        };
        assert!(!targetless.covers(&gh, &page, Mode::Fill, 10));
    }

    #[test]
    fn from_request_carries_the_request_and_the_terms_and_mints_an_id() {
        let grant = Grant::from_request(
            "lb",
            request(),
            Terms {
                approval: Approval::StepUp,
                ttl_ms: 900_000,
                max_uses: Some(1),
                reason_required: true,
                created_by: CreatedBy::Policy("r1".into()),
            },
            1_700_000_000_000,
        );
        assert_eq!(grant.id.len(), 32);
        assert!(grant.id.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(grant.id, Grant::new_id());
        assert_eq!(grant.lockbox_id, "lb");
        assert_eq!(grant.client_key, "ext-key");
        assert_eq!(grant.items, ItemSelector::Ids(vec!["gh".into()]));
        assert_eq!(grant.approval, Approval::StepUp);
        assert!(grant.reason_required);
        assert_eq!(grant.not_before_ms, 1_700_000_000_000);
        assert_eq!(grant.expires_at_ms, 1_700_000_900_000);
        assert_eq!(grant.max_uses, Some(1));
        assert_eq!(grant.created_by, CreatedBy::Policy("r1".into()));
        assert_eq!(
            (grant.state, grant.uses, grant.approver),
            (State::Pending, 0, None)
        );
    }

    // --- the wire and stored shapes -------------------------------------------

    #[test]
    fn the_serde_shapes_read_naturally() {
        assert_eq!(
            serde_json::to_value(Approval::StepUp).unwrap(),
            serde_json::json!("step_up")
        );
        for approval in [Approval::Auto, Approval::Ask, Approval::StepUp] {
            assert_eq!(
                serde_json::to_value(approval).unwrap(),
                serde_json::json!(approval.as_str())
            );
        }
        for mode in [
            Mode::Fill,
            Mode::Passkey,
            Mode::Header,
            Mode::Token,
            Mode::Env,
        ] {
            assert_eq!(
                serde_json::to_value(mode).unwrap(),
                serde_json::json!(mode.as_str())
            );
        }
        assert!(Approval::Auto < Approval::Ask && Approval::Ask < Approval::StepUp);

        let targets = vec![
            Target::Origin("https://github.com".into()),
            Target::Http {
                host: "api.stripe.com".into(),
                path: "/v1/*".into(),
                methods: vec!["GET".into()],
            },
            Target::Command("node deploy.js*".into()),
        ];
        assert_eq!(
            serde_json::to_value(&targets).unwrap(),
            serde_json::json!([
                { "origin": "https://github.com" },
                { "http": { "host": "api.stripe.com", "path": "/v1/*", "methods": ["GET"] } },
                { "command": "node deploy.js*" },
            ])
        );
        assert_eq!(
            serde_json::to_value(ItemSelector::Lockbox).unwrap(),
            serde_json::json!("lockbox")
        );
        assert_eq!(
            serde_json::to_value(CreatedBy::Policy("r1".into())).unwrap(),
            serde_json::json!({ "policy": "r1" })
        );

        let reply = GrantReply {
            grant_id: "g1".into(),
            state: State::Active,
            expires_at_ms: Some(5),
            retry_after_ms: None,
            items: vec![VisibleItem {
                id: "gh".into(),
                title: "GitHub".into(),
                username: Some("octocat".into()),
            }],
        };
        assert_eq!(
            serde_json::to_value(&reply).unwrap(),
            serde_json::json!({
                "grantId": "g1", "state": "active", "expiresAtMs": 5, "retryAfterMs": null,
                "items": [{ "id": "gh", "title": "GitHub", "username": "octocat" }]
            })
        );

        let grant = active(0, 1_000, Some(1));
        let json = serde_json::to_string(&grant).unwrap();
        assert_eq!(serde_json::from_str::<Grant>(&json).unwrap(), grant);
    }
}
