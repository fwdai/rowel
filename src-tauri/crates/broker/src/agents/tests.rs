use super::*;
use crate::actions::{Client, ClientKind};
use crate::audit::ActorKind;
use crate::grant::{
    Approval, CreatedBy, GrantRequest, ItemSelector, Mode, ResolveTarget, Target, Terms,
};
use crate::lockbox::CLIENTS_META;
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

// A key the lockbox never named is shown by its ends, as Settings does.
#[test]
fn an_unknown_client_is_named_by_its_key_fingerprint() {
    assert_eq!(
        client_label(&[], "AAAAbbbbCCCCddddEEEEffffGGGGhhhhIIIIjjjjKKK="),
        "AAAAbbbb…KKK="
    );
    assert_eq!(client_label(&[], "short"), "short");
}
