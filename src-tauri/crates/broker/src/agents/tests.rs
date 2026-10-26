use super::*;
use crate::actions::{Client, ClientKind};
use crate::audit::ActorKind;
use crate::grant::{
    Approval, CreatedBy, GrantRequest, ItemSelector, Mode, ResolveTarget, Target, Terms,
};
use crate::lockbox::CLIENTS_META;
use crate::policy::defaults;
use rowel_core::store::VaultStore;

fn active(id: &str, now_ms: i64, ttl_ms: i64) -> Grant {
    let req = GrantRequest {
        client_key: "ext".into(),
        agent_id: None,
        acting_for: None,
        items: ItemSelector::Ids(vec!["gh".into()]),
        targets: vec![Target::Origin(
            "https://github.com/login?next=/x#top".into(),
        )],
        modes: vec![Mode::Fill],
        run_id: None,
        reason: None,
    };
    let terms = Terms {
        approval: Approval::Ask,
        ttl_ms,
        max_uses: None,
        reason_required: false,
        created_by: CreatedBy::User,
    };
    Grant {
        id: id.into(),
        state: State::Active,
        approver: Some("user".into()),
        ..Grant::from_request("lb", req, terms, now_ms)
    }
}

fn client(name: &str, key: &str, kind: ClientKind) -> Client {
    Client {
        name: name.into(),
        key: key.into(),
        kind,
        platform: None,
        created_at_ms: 0,
        last_seen_at_ms: None,
        agent_id: None,
        driven_by_agent: false,
    }
}

// --- the state ------------------------------------------------------------------

// The timer is armed to the earliest end; firing flips what is due and
// says which, and the flipped ones stay listed for an hour.
#[test]
fn the_state_expires_what_is_due_and_keeps_it_an_hour() {
    let mut state = AgentState::default();
    state.grants.push(active("a", 0, 1_000));
    state.grants.push(active("b", 0, 5_000));
    assert_eq!(state.next_wake_ms(), Some(1_000));

    let flipped = state.expire_due(1_000);
    assert_eq!(
        flipped.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
        ["a"]
    );
    assert_eq!(state.find_mut("a").unwrap().state, State::Expired);
    assert!(state.expire_due(1_000).is_empty(), "flipped once");
    assert_eq!(state.next_wake_ms(), Some(5_000), "the live one is next");

    assert!(!state.prune(1_000 + RETAIN_ENDED_MS - 1));
    state.expire_due(5_000);
    assert_eq!(
        state.next_wake_ms(),
        Some(1_000 + RETAIN_ENDED_MS),
        "then the hour's end"
    );
    assert!(state.prune(1_000 + RETAIN_ENDED_MS));
    assert!(state.find_mut("a").is_none());
    assert!(state.find_mut("b").is_some());
}

// Listing the grants is a read that needs the truth now, so a grant past
// its window ends there — through the one expiry path, with its `ended` row
// and its sign-out — and the timer that fires later has nothing left to
// write. A use meanwhile is refused and flips nothing.
#[test]
fn listing_expires_a_grant_with_its_row_and_signal_exactly_once() {
    let (_dir, store, _cipher) = crate::testing::vault();
    store
        .meta_set(
            CLIENTS_META,
            &serde_json::to_string(&[client("Work Chrome", "ext", ClientKind::Extension)]).unwrap(),
        )
        .unwrap();
    let mut held = AgentState::default();
    held.grants.push(active("g1", 0, 1_000));
    assert_eq!(
        held.find_mut("g1").unwrap().use_once(1_000),
        Err(crate::protocol::Code::GrantExpired)
    );
    assert_eq!(held.find_mut("g1").unwrap().state, State::Active);

    let list = |held: &mut AgentState, now| {
        list_grants_in(Locked {
            store: &store,
            held,
            writer: "dev",
            now,
        })
    };
    let (rows, expired) = list(&mut held, 1_000);
    assert!(expired);
    assert_eq!(rows[0].state, State::Expired);
    let ended_rows = store.audit_rows("dev").unwrap();
    assert_eq!(ended_rows.len(), 1);
    assert_eq!(ended_rows[0].event, "ended");
    assert_eq!(
        ended_rows[0].detail.as_deref(),
        Some(r#"{"reason":"expired"}"#)
    );
    assert_eq!(
        held.signals.get("ext"),
        Some(&vec![Signal::GrantEnded {
            grant_id: "g1".into(),
            origins: vec!["https://github.com".into()],
        }])
    );

    assert!(
        !expire_due(Some(&store), &mut held, "dev", 2_000),
        "the timer"
    );
    assert!(!list(&mut held, 2_000).1, "a second listing");
    assert_eq!(store.audit_rows("dev").unwrap().len(), 1);
    assert_eq!(held.signals.get("ext").map(Vec::len), Some(1));
}

// A seal forgets everything the session held — the grants, the rates,
// the switches as it read them — and moves the epoch, so what decided
// under the session settles no more.
#[test]
fn a_reset_forgets_what_the_session_held_and_moves_the_epoch() {
    let mut state = AgentState::default();
    state.initialise(AccessState {
        enabled: true,
        lockbox_on: true,
    });
    state.grants.push(active("a", 0, 1_000));
    state.rate("ext").record_denial(0);
    let before = state.access_epoch;
    state.reset();
    assert!(state.grants.is_empty() && state.rates.is_empty());
    assert_eq!(state.access, None);
    assert_eq!(state.next_wake_ms(), None);
    assert_eq!(state.access_epoch, before + 1);
    assert_eq!(state.access_since(before), Err("switch"));
}

// The switches as the session holds them say whether a release may go,
// and why not: unread, or the app's switch off, is `switch`; the open
// lockbox's access off is `lockbox`. And a request decided under one
// epoch settles under no other.
#[test]
fn the_switches_the_session_holds_say_whether_a_release_may_go() {
    let mut held = AgentState::default();
    assert_eq!(held.access_on(), Err("switch"), "not read yet");
    held.initialise(AccessState {
        enabled: true,
        lockbox_on: true,
    });
    assert_eq!(held.access_on(), Ok(()));
    let epoch = held.access_epoch;
    assert_eq!(held.access_since(epoch), Ok(()));

    held.access.as_mut().unwrap().lockbox_on = false;
    assert_eq!(held.access_on(), Err("lockbox"));
    held.access.as_mut().unwrap().enabled = false;
    assert_eq!(
        held.access_on(),
        Err("switch"),
        "the app's switch is the outer one"
    );

    held.access = Some(AccessState {
        enabled: true,
        lockbox_on: true,
    });
    held.bump_access();
    assert_eq!(held.access_on(), Ok(()), "on, for a new request");
    assert_eq!(
        held.access_since(epoch),
        Err("switch"),
        "not for one decided before the change"
    );
    held.access.as_mut().unwrap().lockbox_on = false;
    assert_eq!(
        held.access_since(epoch),
        Err("lockbox"),
        "the switch that is off, when one is"
    );
}

// --- the rows ---------------------------------------------------------------------

// What a row says about a grant, and what it must never say: the page's
// query string and fragment are gone, the rest of the URL is kept.
#[test]
fn a_row_about_a_grant_names_it_without_the_query_string() {
    let grant = active("g1", 0, 1_000);
    let clients = vec![client("Work Chrome", "ext", ClientKind::Extension)];
    let row = about(draft(Event::Used, "ext", &clients), &grant, Some("{}"));
    assert_eq!(row.actor_kind, ActorKind::Extension);
    assert_eq!(row.actor_name.as_deref(), Some("Work Chrome"));
    assert_eq!(row.grant_id.as_deref(), Some("g1"));
    assert_eq!(row.item_id.as_deref(), Some("gh"));
    assert_eq!(row.target.as_deref(), Some("https://github.com/login"));
    assert_eq!(row.mode, Some(Mode::Fill));
    assert_eq!(row.approval, Some(Approval::Ask));
    assert_eq!(row.approver.as_deref(), Some("user"));
    assert_eq!(row.run_id, None);

    assert_eq!(audit_target("https://a.b/c#frag"), "https://a.b/c");
    assert_eq!(audit_target("https://a.b"), "https://a.b");
}

// The actor is what the lockbox's record says the client is, and an
// agent's own words for its run go on the row with the grant.
#[test]
fn a_row_names_the_actor_by_the_clients_kind_and_carries_the_run() {
    let clients = vec![
        client("Work Chrome", "ext", ClientKind::Extension),
        client("Claude Code", "agent", ClientKind::Agent),
        client("CLI", "side", ClientKind::Sidecar),
    ];
    assert_eq!(
        draft(Event::Requested, "agent", &clients).actor_kind,
        ActorKind::Agent
    );
    assert_eq!(
        draft(Event::Requested, "side", &clients).actor_kind,
        ActorKind::Sidecar
    );
    assert_eq!(
        draft(Event::Requested, "nobody", &clients).actor_kind,
        ActorKind::Extension,
        "a key the lockbox never named is an extension's"
    );

    let grant = Grant {
        client_key: "agent".into(),
        run_id: Some("run-7".into()),
        acting_for: Some("alice".into()),
        ..active("g2", 0, 1_000)
    };
    let row = about(draft(Event::Granted, "agent", &clients), &grant, None);
    assert_eq!(row.actor_kind, ActorKind::Agent);
    assert_eq!(row.actor_name.as_deref(), Some("Claude Code"));
    assert_eq!(row.run_id.as_deref(), Some("run-7"));
    assert_eq!(row.acting_for.as_deref(), Some("alice"));
}

// Every way a grant ends goes through `ended`: the row, and the sign-out
// signal to every extension — and to nothing else.
#[test]
fn an_ended_grant_is_signalled_to_every_extension_and_no_agent() {
    let (_dir, store, _cipher) = crate::testing::vault();
    store
        .meta_set(
            CLIENTS_META,
            &serde_json::to_string(&[
                client("Work Chrome", "ext", ClientKind::Extension),
                client("Home Firefox", "ext2", ClientKind::Extension),
                client("Claude Code", "agent", ClientKind::Agent),
            ])
            .unwrap(),
        )
        .unwrap();
    let mut held = AgentState::default();
    let grant = Grant {
        client_key: "agent".into(),
        state: State::Revoked,
        ..active("g1", 0, 1_000)
    };

    ended(
        Some(&store),
        &mut held,
        "dev",
        &grant,
        Event::Revoked,
        r#"{"reason":"agent"}"#,
        5,
    );

    let rows = store.audit_rows("dev").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event, "revoked");
    assert_eq!(rows[0].actor_kind, "agent");
    assert_eq!(rows[0].detail.as_deref(), Some(r#"{"reason":"agent"}"#));
    let expected = Signal::GrantEnded {
        grant_id: "g1".into(),
        origins: vec!["https://github.com".into()],
    };
    assert_eq!(held.signals.get("ext"), Some(&vec![expected.clone()]));
    assert_eq!(held.signals.get("ext2"), Some(&vec![expected]));
    assert!(!held.signals.contains_key("agent"));

    // No lockbox open: nowhere to write, nobody to tell.
    let mut none = AgentState::default();
    ended(None, &mut none, "dev", &grant, Event::Ended, "{}", 5);
    assert!(none.signals.is_empty());

    // A header grant signed nothing in: its end is on the log and wakes
    // no extension for a sign-out of nowhere.
    let header = Grant {
        targets: vec![Target::Http {
            host: "api.stripe.com".into(),
            path: "/v1/*".into(),
            methods: vec![],
        }],
        modes: vec![Mode::Header],
        ..grant.clone()
    };
    let mut quiet = AgentState::default();
    ended(
        Some(&store),
        &mut quiet,
        "dev",
        &header,
        Event::Ended,
        r#"{"reason":"expired"}"#,
        6,
    );
    assert_eq!(store.audit_rows("dev").unwrap().len(), 2);
    assert_eq!(
        store.audit_rows("dev").unwrap()[1].target.as_deref(),
        Some("api.stripe.com/v1/*")
    );
    assert!(quiet.signals.is_empty(), "nobody is told");

    // A seal forgets what was queued with the rest.
    held.reset();
    assert!(held.signals.is_empty());
}

// A secret grant's target is rendered the same way on a row and in
// Settings — `host/path`, or the command line — with the methods only
// where a person reads them, and never a query or an unbounded line.
#[test]
fn a_secret_target_renders_as_host_and_path_or_the_command_line() {
    let http = Target::Http {
        host: "api.stripe.com".into(),
        path: "/v1/*".into(),
        methods: vec!["GET".into(), "POST".into()],
    };
    assert_eq!(target_text(&http), "GET, POST api.stripe.com/v1/*");
    assert_eq!(audit_target_of(&http), "api.stripe.com/v1/*");
    let any = Target::Http {
        host: "*.example.com".into(),
        path: "v2".into(),
        methods: vec![],
    };
    assert_eq!(target_text(&any), "*.example.com/v2");

    let line = format!("node deploy.js {}", "x".repeat(400));
    let command = Target::Command(line.clone());
    assert_eq!(target_text(&command), line, "Settings shows it whole");
    assert_eq!(audit_target_of(&command).chars().count(), 300);

    assert_eq!(
        resolve_target_text(&ResolveTarget::Http {
            host: "api.stripe.com".into(),
            path: "/v1/balance?expand=1".into(),
            method: "GET".into(),
        }),
        "api.stripe.com/v1/balance"
    );
    assert_eq!(
        resolve_target_text(&ResolveTarget::Command(line))
            .chars()
            .count(),
        300
    );

    let grant = Grant {
        targets: vec![http],
        modes: vec![Mode::Header],
        ..active("g3", 0, 1_000)
    };
    let row = about(draft(Event::Granted, "ext", &[]), &grant, None);
    assert_eq!(row.target.as_deref(), Some("api.stripe.com/v1/*"));
    assert_eq!(row.mode, Some(Mode::Header));
}

// A rule is written for the site, with the port when one was named, and
// without the path a page happened to be on.
#[test]
fn a_standing_rule_is_for_the_sites_origin() {
    assert_eq!(
        site_origin("https://github.com/login?next=/x#top"),
        "https://github.com"
    );
    assert_eq!(
        site_origin("http://localhost:8080/admin"),
        "http://localhost:8080"
    );
    assert_eq!(site_origin("not a url?x=1"), "not a url");
}

// --- covered ---------------------------------------------------------------------

// A grant the extension may be served under that is over — its window
// closed, before the timer flipped it or after — is `GrantExpired`, so the
// extension asks afresh; a revoked one, or one another extension claimed, is
// a plain no. A TOTP step naming no grant passes an expired one by.
#[test]
fn covered_says_expired_of_a_grant_that_is_over_for_whoever_may_use_it() {
    let (_dir, store, _cipher) = crate::testing::vault();
    let mut held = AgentState::default();
    held.initialise(AccessState {
        enabled: true,
        lockbox_on: true,
    });
    held.grants.push(active("g1", 0, 1_000));
    let check = |held: &mut AgentState, key: &str, now| {
        covered(
            Locked {
                store: &store,
                held,
                writer: "dev",
                now,
            },
            key,
            "g1",
            "gh",
            Mode::Fill,
        )
    };

    assert_eq!(check(&mut held, "ext", 500), Ok(true));
    assert_eq!(
        check(&mut held, "ext", 1_000),
        Err(crate::protocol::Code::GrantExpired),
        "due"
    );
    assert!(expire_due(Some(&store), &mut held, "dev", 1_000));
    assert_eq!(
        check(&mut held, "ext", 1_000),
        Err(crate::protocol::Code::GrantExpired),
        "flipped"
    );
    let by_client = client_covered(
        Locked {
            store: &store,
            held: &mut held,
            writer: "dev",
            now: 1_000,
        },
        "ext",
        "gh",
        Mode::Fill,
    );
    assert_eq!(by_client, Ok(false));

    let grant = held.find_mut("g1").unwrap();
    grant.client_key = "agent".into();
    grant.served_to = Some("ext2".into());
    assert_eq!(check(&mut held, "ext", 1_000), Ok(false), "another's");
    held.find_mut("g1").unwrap().state = State::Revoked;
    assert_eq!(check(&mut held, "ext2", 1_000), Ok(false), "revoked");
}

// --- exposure ---------------------------------------------------------------------

// A page that showed what was filled ends the grant — but only as reported
// by the client the grant is for or the extension served under it, and only
// once: a stranger who has heard the id ends nothing, and a second report
// writes no second row and sends no second sign-out.
#[test]
fn an_exposure_ends_a_grant_once_and_only_for_its_holder_or_server() {
    let (_dir, store, _cipher) = crate::testing::vault();
    store
        .meta_set(
            CLIENTS_META,
            &serde_json::to_string(&[
                client("Work Chrome", "ext", ClientKind::Extension),
                client("Home Firefox", "ext2", ClientKind::Extension),
                client("Claude Code", "agent", ClientKind::Agent),
            ])
            .unwrap(),
        )
        .unwrap();
    let mut held = AgentState::default();
    held.grants.push(Grant {
        client_key: "agent".into(),
        served_to: Some("ext".into()),
        ..active("g1", 0, 1_000)
    });
    held.grants.push(active("g2", 0, 1_000));
    let report = |held: &mut AgentState, id: &str, reporter: &str| {
        report_exposure_in(
            Locked {
                store: &store,
                held,
                writer: "dev",
                now: 5,
            },
            id,
            reporter,
        )
    };

    assert_eq!(report(&mut held, "g1", "ext2"), GrantEnd::NotFound);
    assert_eq!(report(&mut held, "g9", "ext"), GrantEnd::NotFound);
    assert_eq!(held.find_mut("g1").unwrap().state, State::Active);
    assert!(store.audit_rows("dev").unwrap().is_empty());
    assert!(held.signals.is_empty());

    assert_eq!(report(&mut held, "g1", "ext"), GrantEnd::Ended, "served to");
    assert_eq!(held.find_mut("g1").unwrap().state, State::Revoked);
    let rows = store.audit_rows("dev").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event, "exposed");
    assert_eq!(rows[0].detail.as_deref(), Some(r#"{"reason":"exposure"}"#));
    assert_eq!(held.signals.get("ext2").map(Vec::len), Some(1));

    assert_eq!(report(&mut held, "g1", "agent"), GrantEnd::AlreadyOver);
    assert_eq!(store.audit_rows("dev").unwrap().len(), 1, "no second row");
    assert_eq!(
        held.signals.get("ext2").map(Vec::len),
        Some(1),
        "no second sign-out"
    );

    assert_eq!(
        report(&mut held, "g2", "ext"),
        GrantEnd::Ended,
        "its holder"
    );
    assert_eq!(store.audit_rows("dev").unwrap().len(), 2);
}

// --- this run ---------------------------------------------------------------------

const RUN_PAGE: &str = "https://github.com/login";

/// A store with GitHub's login and two agents let in, and the switches on.
fn run_setup() -> (tempfile::TempDir, SqliteStore, AgentState) {
    let (dir, store, cipher) = crate::testing::vault();
    crate::testing::seed(
        &store,
        &cipher,
        &rowel_core::models::Entry {
            id: "gh".into(),
            kind: "login".into(),
            title: "GitHub".into(),
            website: Some("https://github.com".into()),
            username: Some("octocat".into()),
            password: Some("hunter2".into()),
            ..Default::default()
        },
    );
    store
        .meta_set(
            CLIENTS_META,
            &serde_json::to_string(&[
                client("Claude Code", "agent", ClientKind::Agent),
                client("Codex", "other", ClientKind::Agent),
            ])
            .unwrap(),
        )
        .unwrap();
    let mut held = AgentState::default();
    held.initialise(AccessState {
        enabled: true,
        lockbox_on: true,
    });
    (dir, store, held)
}

fn run_request(client_key: &str, run_id: Option<&str>) -> GrantRequest {
    GrantRequest {
        client_key: client_key.into(),
        agent_id: None,
        acting_for: None,
        items: ItemSelector::Lockbox,
        targets: vec![Target::Origin(RUN_PAGE.into())],
        modes: vec![Mode::Fill],
        run_id: run_id.map(str::to_string),
        reason: None,
    }
}

fn run_decided(
    store: &SqliteStore,
    held: &mut AgentState,
    client_key: &str,
    run_id: Option<&str>,
    now: i64,
) -> Prepared {
    let who = client(client_key, client_key, ClientKind::Agent);
    decide(
        Locked {
            store,
            held,
            writer: "dev",
            now,
        },
        "lb",
        &who,
        run_request(client_key, run_id),
        &[],
    )
    .unwrap_or_else(|code| panic!("decided: {code:?}"))
}

fn run_settled(
    store: &SqliteStore,
    held: &mut AgentState,
    run_id: Option<&str>,
    answer: GrantAnswer,
    now: i64,
) -> std::result::Result<crate::grant::GrantReply, crate::protocol::Code> {
    let asking = match run_decided(store, held, "agent", run_id, now) {
        Prepared::Ask(asking) => *asking,
        Prepared::Answered(_) => panic!("the baseline asks"),
    };
    settle(
        Locked {
            store,
            held,
            writer: "dev",
            now,
        },
        &client("agent", "agent", ClientKind::Agent),
        asking,
        Verdict::Answered(Some(answer)),
    )
}

fn for_run() -> GrantAnswer {
    GrantAnswer {
        item_id: "gh".into(),
        standing_days: None,
        for_run: true,
    }
}

/// Whether `prepared` was served from the grant `id` already held.
fn served_from(prepared: &Prepared, id: &str) -> bool {
    matches!(prepared, Prepared::Answered(reply) if reply.grant_id == id)
}

// A yes for the run serves further requests of the same client marked with
// the same run, past the once window and up to the run's cap, each a use on
// the log — and nothing else: not another run, not a request naming none,
// not another client naming the same run, not past the cap.
#[test]
fn a_yes_for_the_run_covers_that_run_of_that_client_until_the_cap() {
    let (_dir, store, mut held) = run_setup();
    let now = 1_000_000;
    let reply = run_settled(&store, &mut held, Some("run-1"), for_run(), now).unwrap();
    assert_eq!(reply.state, State::Active);
    assert_eq!(
        reply.expires_at_ms,
        Some(now + defaults::RUN_TTL_MS),
        "the run's cap, not the rule's window"
    );
    let grant = held.find_mut(&reply.grant_id).unwrap().clone();
    assert!(grant.for_run);
    assert_eq!(grant.run_id.as_deref(), Some("run-1"));
    let rows = store.audit_rows("dev").unwrap();
    let granted = rows.iter().find(|row| row.event == "granted").unwrap();
    assert_eq!(granted.detail.as_deref(), Some(r#"{"for":"run"}"#));
    assert_eq!(granted.run_id.as_deref(), Some("run-1"));

    let later = now + defaults::PERSONAL_TTL_MS + 60_000;
    let again = run_decided(&store, &mut held, "agent", Some("run-1"), later);
    assert!(served_from(&again, &reply.grant_id), "the same run, later");
    let reuse = store.audit_rows("dev").unwrap().pop().unwrap();
    assert_eq!(
        (reuse.event.as_str(), reuse.detail.as_deref()),
        ("used", Some(r#"{"reuse":true}"#))
    );

    for (key, run) in [
        ("agent", Some("run-2")),
        ("agent", None),
        ("other", Some("run-1")),
    ] {
        let asked = run_decided(&store, &mut held, key, run, later);
        assert!(
            matches!(asked, Prepared::Ask(_)),
            "{key} with {run:?} is asked again"
        );
    }

    let capped = now + defaults::RUN_TTL_MS;
    let past = run_decided(&store, &mut held, "agent", Some("run-1"), capped);
    assert!(matches!(past, Prepared::Ask(_)), "the cap ends the run");
}

// A once yes is unchanged by runs: its window bounds it, whatever run a
// later request names.
#[test]
fn a_once_yes_is_reused_inside_its_window_whatever_the_run() {
    let (_dir, store, mut held) = run_setup();
    let now = 1_000_000;
    let once = GrantAnswer {
        for_run: false,
        ..for_run()
    };
    let reply = run_settled(&store, &mut held, Some("run-1"), once, now).unwrap();
    assert_eq!(reply.expires_at_ms, Some(now + defaults::PERSONAL_TTL_MS));
    assert!(!held.find_mut(&reply.grant_id).unwrap().for_run);
    let other_run = run_decided(&store, &mut held, "agent", Some("run-2"), now + 1);
    assert!(served_from(&other_run, &reply.grant_id));
}

// The dialog offers the run only for a request that named one, and never
// beside days; an answer that does either anyway is a refusal on record.
#[test]
fn a_yes_for_a_run_the_request_did_not_name_is_refused() {
    let (_dir, store, mut held) = run_setup();
    assert_eq!(
        run_settled(&store, &mut held, None, for_run(), 1_000),
        Err(crate::protocol::Code::GrantDenied)
    );
    let both = GrantAnswer {
        standing_days: Some(7),
        ..for_run()
    };
    assert_eq!(
        run_settled(&store, &mut held, Some("run-1"), both, 2_000),
        Err(crate::protocol::Code::GrantDenied)
    );
    let denials: Vec<Option<String>> = store
        .audit_rows("dev")
        .unwrap()
        .into_iter()
        .filter(|row| row.event == "denied")
        .map(|row| row.detail)
        .collect();
    assert_eq!(
        denials,
        [
            Some(r#"{"reason":"run"}"#.to_string()),
            Some(r#"{"reason":"answer"}"#.to_string())
        ]
    );
    assert!(live_rules(&store, 2_000).is_empty(), "no rule was written");
    assert!(held.grants.is_empty());
}

// An answer from before *this run* reads as once.
#[test]
fn an_answer_without_for_run_reads_as_once() {
    let answer: GrantAnswer =
        serde_json::from_str(r#"{"itemId":"gh","standingDays":null}"#).unwrap();
    assert_eq!(answer.lasting(), Ok(Lasting::Once));
    let days: GrantAnswer = serde_json::from_str(r#"{"itemId":"gh","standingDays":3}"#).unwrap();
    assert_eq!(days.lasting(), Ok(Lasting::Days(3)));
}

// A key the lockbox never named is shown by its ends, as Settings does.
#[test]
fn an_unknown_client_is_named_by_its_key_fingerprint() {
    assert_eq!(
        client_label(&[], "AAAAbbbbCCCCddddEEEEffffGGGGhhhhIIIIjjjjKKK="),
        "AAAAbbbb…KKK="
    );
    assert_eq!(client_label(&[], "short"), "short");
}
