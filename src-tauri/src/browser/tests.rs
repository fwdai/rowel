use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Map, Value};

use super::manifest::{self, Family, KEEPASSXC, ROWEL};
use super::protocol::str_of;
use super::protocol::Code;
use super::{
    covered, decide, frame, group_container, group_socket_dir, outcome, proxy, server, settle,
    socket_name, Answer, AskKind, Asking, Candidate, CandidateItem, GrantAnswer, Locked, NotAsked,
    Pending, Prepared, SafariSocketError, StepUp, Verdict, IDENTIFIER, SUN_PATH_MAX,
};
use crate::agents::{self, Access, AgentState};
use crate::crypto::PayloadCipher;
use crate::models::Entry;
use crate::session::{Epoch, Session};
use crate::store::{migrate, SqliteStore, VaultStore};
use rowel_broker::grant::{
    Approval, CreatedBy, Grant, GrantRequest, ItemSelector, Mode, State, Target, Terms, VisibleItem,
};
use rowel_broker::policy::defaults;
use rowel_broker::testing::{nonce, seed, vault, Extension, Mock};
use rowel_core::app::DESKTOP_APP_GROUP;

// --- the consent slot ----------------------------------------------------------

/// A `prepare` that lets the ask go up.
fn up() -> Result<(), ()> {
    Ok(())
}

// A grant request tells a slot another ask holds from an ask nobody
// answered: the first is "ask again shortly", the second a refusal.
#[test]
fn a_taken_slot_reads_as_busy_rather_than_as_no_answer() {
    let pending: Pending<bool> = Pending::new();
    let (_first, _answer) = pending.begin("a", AskKind::Associate, up).unwrap();
    assert!(matches!(
        pending.try_ask_with("b", AskKind::Grant { step_up: false }, up),
        Err(NotAsked::Busy)
    ));
}

// A grant ask whose access moved since it was decided never goes up: its
// `prepare` refuses under the slot lock, nothing reaches the screen, and
// the slot is free for the next ask.
#[test]
fn a_grant_ask_whose_access_moved_never_goes_up() {
    let pending: Pending<bool> = Pending::new();
    let mut held = AgentState::default();
    let access = Access {
        epoch: held.access_epoch,
        enabled: true,
        lockbox_on: true,
    };
    held.bump_access();

    let mut emitted = false;
    let asked = pending.try_ask_with("g", AskKind::Grant { step_up: false }, || {
        access.verify(&held)?;
        emitted = true;
        Ok(())
    });
    assert!(matches!(asked, Err(NotAsked::Refused("switch"))));
    assert!(!emitted, "no dialog");
    assert!(!pending.needs_step_up("g"));
    assert!(
        pending.begin("h", AskKind::Associate, up).is_ok(),
        "the slot is free"
    );
}

// A change of the switches refuses the grant ask up through the slot: the
// request waiting on it gets a "no", the dialog's id comes back for the
// closed event, and an ask of another kind is left alone.
#[test]
fn refusing_the_grant_ask_answers_it_and_no_other_kind() {
    let pending: Pending<Answer> = Pending::new();
    assert_eq!(pending.refuse_grant(), None, "nothing up");

    let (_, answer) = pending
        .begin("g", AskKind::Grant { step_up: true }, up)
        .unwrap();
    assert_eq!(pending.refuse_grant(), Some("g".to_string()));
    assert!(matches!(answer.try_recv(), Ok(Answer::Grant(None))));
    let (_, next) = pending
        .begin("next", AskKind::Associate, up)
        .expect("the slot is free again");
    assert_eq!(
        pending.refuse_grant(),
        None,
        "an associate ask is not a grant ask"
    );
    assert!(
        pending.answer("next", Answer::Name(None)),
        "and is still up"
    );
    assert!(matches!(next.try_recv(), Ok(Answer::Name(None))));

    let (_, passkey) = pending.begin("p", AskKind::Passkey, up).unwrap();
    assert_eq!(pending.refuse_grant(), None);
    assert!(pending.answer("p", Answer::Account(None)));
    assert!(matches!(passkey.try_recv(), Ok(Answer::Account(None))));
}

// The slot is the one record of what is up, so it is what says whether the
// ask up under an id takes a verified person before its yes counts.
#[test]
fn only_a_step_up_grant_ask_that_is_up_needs_a_step_up() {
    let pending: Pending<Answer> = Pending::new();
    assert!(!pending.needs_step_up("s"), "nothing up");

    let (serial, _answer) = pending
        .begin("s", AskKind::Grant { step_up: true }, up)
        .unwrap();
    assert!(pending.needs_step_up("s"));
    assert!(!pending.needs_step_up("other"), "another id");
    pending.end(serial);
    assert!(!pending.needs_step_up("s"), "down");

    let (serial, _answer) = pending
        .begin("s", AskKind::Grant { step_up: false }, up)
        .unwrap();
    assert!(!pending.needs_step_up("s"), "an ordinary grant ask");
    pending.end(serial);

    let (_, _answer) = pending.begin("s", AskKind::Associate, up).unwrap();
    assert!(!pending.needs_step_up("s"), "another kind");
}

// In this iteration no login fill reaches step-up — the floors raise cards,
// identities, passkeys, env and money targets — so the gate is exercised
// through the floors: a card among the page's items makes the baseline's
// `ask` a `step_up`, which is what puts the biometric or password step in
// front of the dialog's yes.
#[test]
fn a_card_fill_under_the_baseline_is_a_step_up_ask() {
    use rowel_broker::grant::Probe;
    use rowel_broker::policy::{evaluate, Decision, Floors};

    let request = GrantRequest {
        client_key: "ext".into(),
        agent_id: None,
        acting_for: None,
        items: ItemSelector::Ids(vec!["visa".into()]),
        targets: vec![Target::Origin("https://shop.example".into())],
        modes: vec![Mode::Fill],
        run_id: None,
        reason: None,
    };
    let card = [CandidateItem {
        id: "visa".into(),
        kind: "card".into(),
        tags: vec![],
    }];
    let decision = evaluate(
        &defaults::personal(),
        &request,
        &card,
        &[Probe::Origin("https://shop.example/checkout")],
        &Floors::default(),
    );
    assert!(matches!(
        decision,
        Decision::Allow {
            approval: Approval::StepUp,
            ..
        }
    ));
    // And a plain login stays an ordinary ask, with no step in front of it.
    let login = [CandidateItem {
        id: "visa".into(),
        kind: "login".into(),
        tags: vec![],
    }];
    let decision = evaluate(
        &defaults::personal(),
        &request,
        &login,
        &[Probe::Origin("https://shop.example/checkout")],
        &Floors::default(),
    );
    assert!(matches!(
        decision,
        Decision::Allow {
            approval: Approval::Ask,
            ..
        }
    ));
    // Whichever the device has, the ask says which step it is.
    assert!(matches!(
        StepUp::current().as_str(),
        "biometric" | "password"
    ));
}

#[test]
fn an_ask_that_was_answered_leaves_the_next_ask_its_slot() {
    let pending: Pending<bool> = Pending::new();
    let (first, answer) = pending.begin("a", AskKind::Associate, up).unwrap();
    assert!(
        pending.begin("b", AskKind::Associate, up).is_err(),
        "one ask at a time"
    );
    assert!(
        !pending.answer("b", true),
        "an answer for another tag does nothing"
    );
    assert!(pending.answer("a", true));
    assert_eq!(answer.recv().ok(), Some(true));

    // The answer freed the slot, and the next ask took it before the first
    // ask's thread got round to tidying up. That tidy-up must be a no-op.
    let (_, next) = pending
        .begin("b", AskKind::Associate, up)
        .expect("the answer freed the slot");
    pending.end(first);
    assert!(
        pending.answer("b", false),
        "the second ask is still waiting"
    );
    assert_eq!(next.recv().ok(), Some(false));

    // An ask nobody answered does clear its own slot.
    let (third, _) = pending.begin("c", AskKind::Passkey, up).unwrap();
    pending.end(third);
    assert!(pending.begin("d", AskKind::Associate, up).is_ok());
}

// --- releases, against a store and the grants held -----------------------------
//
// `decide`, `settle` and `covered` are the whole of what the host does
// under the session and agents locks, over the open store and the grants
// held; here they run over a store of their own, with the locks the tests'
// own `&mut`. An `Access` stands for the switches as a request read them,
// `bump_access` for what a switch change does under the locks.

const PAGE: &str = "https://github.com/login";

fn github_login(id: &str) -> Entry {
    Entry {
        id: id.into(),
        kind: "login".into(),
        title: "GitHub".into(),
        website: Some("https://github.com".into()),
        username: Some("octocat".into()),
        password: Some("hunter2".into()),
        ..Entry::default()
    }
}

fn fill_request(ids: &[&str]) -> GrantRequest {
    GrantRequest {
        client_key: "ext".into(),
        agent_id: None,
        acting_for: None,
        items: ItemSelector::Ids(ids.iter().map(|id| id.to_string()).collect()),
        targets: vec![Target::Origin(PAGE.into())],
        modes: vec![Mode::Fill],
        run_id: None,
        reason: None,
    }
}

/// An active fill grant for `item` at the page, as the dialog's yes leaves one.
fn active_grant(item: &str, now: i64) -> Grant {
    let terms = Terms {
        approval: Approval::Ask,
        ttl_ms: 900_000,
        max_uses: None,
        reason_required: false,
        created_by: CreatedBy::User,
    };
    Grant {
        state: State::Active,
        approver: Some("user".into()),
        ..Grant::from_request("default", fill_request(&[item]), terms, now)
    }
}

/// The switches read on, under the epoch `held` stands at.
fn on(held: &AgentState) -> Access {
    Access {
        epoch: held.access_epoch,
        enabled: true,
        lockbox_on: true,
    }
}

fn locked<'a>(
    store: &'a SqliteStore,
    held: &'a mut AgentState,
    access: &'a Access,
    now: i64,
) -> Locked<'a> {
    Locked {
        store,
        held,
        access,
        writer: "dev",
        now,
    }
}

fn epoch() -> Epoch {
    Session::default().epoch()
}

fn decided(
    store: &SqliteStore,
    held: &mut AgentState,
    access: &Access,
    now: i64,
) -> Result<Prepared, Code> {
    decide(
        locked(store, held, access, now),
        epoch(),
        "default",
        "ext",
        fill_request(&["gh"]),
    )
}

/// The ask the baseline rule makes of the page's one login.
fn asked(store: &SqliteStore, held: &mut AgentState, access: &Access, now: i64) -> Asking {
    match decided(store, held, access, now) {
        Ok(Prepared::Ask(asking)) => *asking,
        _ => panic!("the baseline asks"),
    }
}

/// The log so far: each row's event and detail.
fn rows(store: &SqliteStore) -> Vec<(String, Option<String>)> {
    store
        .audit_rows("dev")
        .unwrap()
        .into_iter()
        .map(|row| (row.event, row.detail))
        .collect()
}

fn row(event: &str, detail: &str) -> (String, Option<String>) {
    (event.into(), Some(detail.into()))
}

fn yes(item_id: &str) -> Verdict {
    Verdict::Answered(Some(GrantAnswer {
        item_id: item_id.into(),
        standing_days: Some(7),
    }))
}

// A grant this client holds for the page is served again, once; the release
// checks the access the request was read under, under the same locks a
// switch change moves the epoch under. A change in between, and the held
// grant — still active — releases nothing.
#[test]
fn a_reuse_is_refused_once_a_switch_changed_since_the_request_was_read() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &github_login("gh"));
    let now = 1_000;
    let mut held = AgentState::default();
    held.grants.push(active_grant("gh", now));
    let access = on(&held);

    let served = decided(&store, &mut held, &access, now);
    assert!(matches!(
        served,
        Ok(Prepared::Answered(reply)) if reply.items.len() == 1 && reply.items[0].id == "gh"
    ));
    assert_eq!(held.grants[0].uses, 1);
    assert_eq!(rows(&store), vec![row("used", r#"{"reuse":true}"#)]);

    // The Agents switch, or a lockbox's access, changed: the epoch moved.
    held.bump_access();
    assert!(matches!(
        decided(&store, &mut held, &access, now),
        Err(Code::GrantDenied)
    ));
    assert_eq!(held.grants[0].uses, 1, "nothing released");
    assert_eq!(held.grants[0].state, State::Active, "nothing else touched");
    assert_eq!(
        rows(&store).last(),
        Some(&row("denied", r#"{"reason":"switch"}"#))
    );
}

// A standing rule allows without asking: the grant is made and served in
// one go — after the same check. A snapshot from before a switch change
// makes no grant at all.
#[test]
fn an_auto_release_is_refused_once_a_switch_changed_since_the_request_was_read() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &github_login("gh"));
    let now = 1_000;
    agents::write_rules(
        &store,
        &[defaults::standing(
            "ext",
            "gh",
            "https://github.com",
            7,
            now,
        )],
    )
    .unwrap();

    let mut held = AgentState::default();
    let stale = on(&held);
    held.bump_access();
    assert!(matches!(
        decided(&store, &mut held, &stale, now),
        Err(Code::GrantDenied)
    ));
    assert!(held.grants.is_empty(), "no grant was made");
    assert_eq!(rows(&store), vec![row("denied", r#"{"reason":"switch"}"#)]);

    let fresh = on(&held);
    assert!(matches!(
        decided(&store, &mut held, &fresh, now),
        Ok(Prepared::Answered(reply)) if reply.state == State::Active
    ));
    assert_eq!(held.grants.len(), 1);
    assert_eq!(held.grants[0].approver.as_deref(), Some("policy"));
    assert_eq!(held.grants[0].uses, 1);
    let events: Vec<String> = rows(&store).into_iter().map(|(event, _)| event).collect();
    assert_eq!(events, ["denied", "requested", "granted", "used"]);
}

// `grant_covers` — the login or its code about to be served under a grant
// in hand — is a release like any other: the lockbox's access going off
// moved the epoch, and the grant, still active, covers nothing.
#[test]
fn a_held_grant_covers_nothing_once_the_lockbox_access_went_off() {
    let (_dir, store, _cipher) = vault();
    let now = 1_000;
    let mut held = AgentState::default();
    held.grants.push(active_grant("gh", now));
    let id = held.grants[0].id.clone();
    let access = on(&held);

    assert!(covered(
        locked(&store, &mut held, &access, now),
        &id,
        "gh",
        Mode::Fill
    ));
    assert_eq!(held.grants[0].uses, 1);
    assert_eq!(rows(&store), vec![row("used", r#"{"via":"grantId"}"#)]);

    // The lockbox's access was switched off: `access_changed` moved the epoch.
    held.bump_access();
    assert!(!covered(
        locked(&store, &mut held, &access, now),
        &id,
        "gh",
        Mode::Fill
    ));
    assert_eq!(
        held.grants[0].state,
        State::Active,
        "the grant itself is not touched here"
    );
    assert_eq!(held.grants[0].uses, 1, "and not used");
    assert_eq!(rows(&store).len(), 1, "no row either");

    // A snapshot that read the access off, under the new epoch: the same.
    let read_off = Access {
        lockbox_on: false,
        ..on(&held)
    };
    assert!(!covered(
        locked(&store, &mut held, &read_off, now),
        &id,
        "gh",
        Mode::Fill
    ));
    // One that read both on under the new epoch serves.
    let fresh = on(&held);
    assert!(covered(
        locked(&store, &mut held, &fresh, now),
        &id,
        "gh",
        Mode::Fill
    ));
    assert_eq!(held.grants[0].uses, 2);
}

// The two orders a switch-off and a release can take, and both deny: the
// snapshot taken before the switch is written and the epoch moved (the epoch
// tells), and the snapshot taken after the switch is written but before the
// epoch moved (the switch tells).
#[test]
fn a_switch_off_and_a_release_deny_in_either_order() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &github_login("gh"));
    let now = 1_000;
    let mut held = AgentState::default();
    held.grants.push(active_grant("gh", now));
    let id = held.grants[0].id.clone();

    // Snapshot; the switch written off; the epoch moved; the release.
    let before = on(&held);
    held.bump_access();
    assert!(!covered(
        locked(&store, &mut held, &before, now),
        &id,
        "gh",
        Mode::Fill
    ));
    assert!(matches!(
        decided(&store, &mut held, &before, now),
        Err(Code::GrantDenied)
    ));

    // The switch written off; the snapshot, which reads it off; the release
    // — before the epoch has moved.
    let after = Access {
        enabled: false,
        ..on(&held)
    };
    assert!(!covered(
        locked(&store, &mut held, &after, now),
        &id,
        "gh",
        Mode::Fill
    ));
    assert!(matches!(
        decided(&store, &mut held, &after, now),
        Err(Code::GrantDenied)
    ));

    assert_eq!(held.grants[0].uses, 0, "nothing released either way");
    assert_eq!(
        rows(&store),
        vec![
            row("denied", r#"{"reason":"switch"}"#),
            row("denied", r#"{"reason":"switch"}"#)
        ]
    );
}

// The dialog's yes is settled under the same check: a switch that changed
// while the dialog was up refuses it, and the row names the switch that is
// off — the switches as they stand now, held to the epoch the request was
// decided under (`Access::since`).
#[test]
fn a_yes_after_a_switch_went_off_is_refused_and_says_which_switch() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &github_login("gh"));
    let now = 1_000;
    let mut held = AgentState::default();

    // The lockbox's access went off while the dialog was up.
    let decided_under = on(&held);
    let asking = asked(&store, &mut held, &decided_under, now);
    held.bump_access();
    let settled_under = Access {
        lockbox_on: false,
        ..on(&held)
    }
    .since(decided_under.epoch);
    assert!(matches!(
        settle(
            locked(&store, &mut held, &settled_under, now),
            "ext",
            asking,
            yes("gh")
        ),
        Err(Code::GrantDenied)
    ));
    assert!(held.grants.is_empty(), "nothing released");
    assert_eq!(
        rows(&store),
        vec![
            ("requested".to_string(), None),
            row("denied", r#"{"reason":"lockbox"}"#)
        ]
    );

    // The Agents switch went off and on again: both read on now, but the
    // epoch moved.
    let decided_under = on(&held);
    let asking = asked(&store, &mut held, &decided_under, now);
    held.bump_access();
    let settled_under = on(&held).since(decided_under.epoch);
    assert!(matches!(
        settle(
            locked(&store, &mut held, &settled_under, now),
            "ext",
            asking,
            yes("gh")
        ),
        Err(Code::GrantDenied)
    ));
    assert_eq!(
        rows(&store).last(),
        Some(&row("denied", r#"{"reason":"switch"}"#))
    );

    // Nothing changed: the yes stands, for the item picked, and "for 7
    // days" writes its rule first.
    let decided_under = on(&held);
    let asking = asked(&store, &mut held, &decided_under, now);
    let Ok(reply) = settle(
        locked(&store, &mut held, &decided_under, now),
        "ext",
        asking,
        yes("gh"),
    ) else {
        panic!("the yes stands")
    };
    assert_eq!(reply.state, State::Active);
    assert_eq!(held.grants.len(), 1);
    assert_eq!(held.grants[0].approver.as_deref(), Some("user"));
    let events: Vec<String> = rows(&store).into_iter().map(|(event, _)| event).collect();
    assert_eq!(
        &events[events.len() - 4..],
        ["requested", "rule_added", "granted", "used"]
    );
}

// A change of the switches that leaves the open lockbox's access on — the
// Agents switch turned on, another lockbox's access either way — moves the
// epoch, so an in-flight request is refused once, and nothing more: the
// grants held stay active, and the log says nothing.
#[test]
fn a_change_that_leaves_the_open_lockbox_on_moves_the_epoch_and_keeps_its_grants() {
    let (_dir, store, _cipher) = vault();
    let now = 1_000;
    let mut held = AgentState::default();
    held.grants.push(active_grant("gh", now));
    let both_on = on(&held);
    assert!(!both_on.cut_off());

    // The Agents switch turned on: the switches now read on.
    agents::switch_changed(
        Some(&store),
        &mut held,
        "dev",
        both_on.cut_off(),
        "switch",
        now,
    );
    assert_eq!(held.access_epoch, 1);
    // Another lockbox's access on, then off: the open one's reads on still.
    agents::switch_changed(
        Some(&store),
        &mut held,
        "dev",
        both_on.cut_off(),
        "lockbox",
        now,
    );
    agents::switch_changed(
        Some(&store),
        &mut held,
        "dev",
        both_on.cut_off(),
        "lockbox",
        now,
    );
    assert_eq!(held.access_epoch, 3);

    assert_eq!(held.grants[0].state, State::Active);
    assert!(rows(&store).is_empty(), "no `revoked` row");
    // And the snapshot from before any of it releases nothing now.
    assert_eq!(both_on.verify(&held), Err("switch"));
}

// A change that cuts the open lockbox off — its own access going off, or
// the Agents switch — ends every active grant, with the row naming which.
#[test]
fn a_change_that_cuts_the_open_lockbox_off_ends_its_grants_and_says_which_switch() {
    let (_dir, store, _cipher) = vault();
    let now = 1_000;

    // The open lockbox's access off.
    let mut held = AgentState::default();
    held.grants.push(active_grant("gh", now));
    let lockbox_off = Access {
        lockbox_on: false,
        ..on(&held)
    };
    assert!(lockbox_off.cut_off());
    agents::switch_changed(
        Some(&store),
        &mut held,
        "dev",
        lockbox_off.cut_off(),
        "lockbox",
        now,
    );
    assert_eq!(held.access_epoch, 1);
    assert_eq!(held.grants[0].state, State::Revoked);
    assert_eq!(
        rows(&store),
        vec![row("revoked", r#"{"reason":"lockbox"}"#)]
    );

    // The Agents switch off: one row per grant that was active, none for
    // one that had ended already.
    let mut held = AgentState::default();
    held.grants.push(active_grant("gh", now));
    held.grants.push(active_grant("gh2", now));
    held.grants.push(Grant {
        state: State::Expired,
        ..active_grant("old", now)
    });
    let switch_off = Access {
        enabled: false,
        ..on(&held)
    };
    assert!(switch_off.cut_off());
    agents::switch_changed(
        Some(&store),
        &mut held,
        "dev",
        switch_off.cut_off(),
        "switch",
        now,
    );
    assert!(held.grants[..2].iter().all(|g| g.state == State::Revoked));
    assert_eq!(
        held.grants[2].state,
        State::Expired,
        "an ended grant is left as it ended"
    );
    assert_eq!(
        rows(&store)[1..],
        [
            row("revoked", r#"{"reason":"switch"}"#),
            row("revoked", r#"{"reason":"switch"}"#)
        ]
    );

    // No lockbox open: the epoch still moves, and there is nowhere to write.
    let mut held = AgentState::default();
    held.grants.push(active_grant("gh", now));
    agents::switch_changed(None, &mut held, "dev", true, "switch", now);
    assert_eq!(held.access_epoch, 1);
    assert_eq!(held.grants[0].state, State::Revoked);
}

// What the answer itself comes to, once the switches have been checked.
#[test]
fn the_answer_names_the_item_picked_or_why_not() {
    let candidate = |id: &str| Candidate {
        item: CandidateItem {
            id: id.into(),
            kind: "login".into(),
            tags: vec![],
        },
        visible: VisibleItem {
            id: id.into(),
            title: id.to_uppercase(),
            username: None,
        },
    };
    let candidates = [candidate("gh"), candidate("gh2")];

    let (picked, days) = outcome(&yes("gh2"), &candidates).expect("picked");
    assert_eq!(picked.item.id, "gh2");
    assert_eq!(days, Some(7));
    assert_eq!(outcome(&yes("nope"), &candidates).err(), Some("item"));
    assert_eq!(
        outcome(&Verdict::Answered(None), &candidates).err(),
        Some("user")
    );
    assert_eq!(
        outcome(&Verdict::Unanswered, &candidates).err(),
        Some("timeout")
    );
}

// --- which logins a page gets, against a store ---------------------------------

fn bank_login(id: &str, website: &str) -> Entry {
    Entry {
        id: id.into(),
        kind: "login".into(),
        title: "Bank".into(),
        website: Some(website.into()),
        username: Some("alice".into()),
        password: Some("s3cret".into()),
        ..Entry::default()
    }
}

fn served(store: &SqliteStore, cipher: &PayloadCipher, page_url: &str) -> Vec<String> {
    let host = super::actions::site_host(page_url).unwrap();
    let mut ids: Vec<String> = super::logins_in(store, cipher, page_url, &host)
        .into_iter()
        .map(|login| login.id)
        .collect();
    ids.sort();
    ids
}

// A login saved for HTTPS is not served to the plain-HTTP page of the same
// host — the page a network attacker can stand up — nor to another port.
#[test]
fn an_https_login_is_not_served_to_an_http_page_or_another_port() {
    let (_dir, store, cipher) = vault();
    seed(
        &store,
        &cipher,
        &bank_login("tls", "https://bank.com/login"),
    );
    seed(
        &store,
        &cipher,
        &bank_login("port", "https://bank.com:8443"),
    );
    seed(&store, &cipher, &bank_login("typed", "bank.com"));

    assert_eq!(
        served(&store, &cipher, "https://bank.com/"),
        ["tls", "typed"]
    );
    assert_eq!(served(&store, &cipher, "http://bank.com/"), ["typed"]);
    assert_eq!(
        served(&store, &cipher, "https://bank.com:8443/"),
        ["port", "tls", "typed"]
    );
}

// The `url_host` column is plaintext metadata that anyone holding the
// SQLCipher key can rewrite. A row re-pointed at another site passes the
// column's pre-filter there, and is still refused: the sealed website is
// what decides.
#[test]
fn a_tampered_url_host_column_does_not_serve_a_login_elsewhere() {
    let (_dir, store, cipher) = vault();
    let entry = bank_login("bank", "https://bank.com");
    let payload = cipher.seal(&entry).unwrap();
    let mut record = migrate::build_record(&entry, payload).unwrap();
    record.url_host = "evil.com".into();
    store.upsert(&record).unwrap();
    assert_eq!(store.list().unwrap()[0].url_host, "evil.com");

    assert!(served(&store, &cipher, "https://evil.com/").is_empty());
    // Nor does the real site get it while the column points away: the
    // column only narrows.
    assert!(served(&store, &cipher, "https://bank.com/").is_empty());
}

// --- framing and the wire ----------------------------------------------------

#[test]
fn serve_ends_the_connection_once_the_host_is_switched_off() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let served = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let host = Mock::unlocked();
        host.enabled.set(false);
        server::serve(stream, host);
    });

    let mut stream = TcpStream::connect(address).unwrap();
    let request = json!({ "action": "change-public-keys", "publicKey": "x", "nonce": "y" });
    frame::write(&mut stream, &serde_json::to_vec(&request).unwrap()).unwrap();
    // No reply: the other end hangs up instead.
    assert_eq!(frame::read(&mut stream).unwrap(), None);
    served.join().unwrap();
}

#[test]
fn serve_answers_frames_on_a_stream_until_it_closes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let served = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        server::serve(stream, Mock::unlocked());
    });

    let mut stream = TcpStream::connect(address).unwrap();
    let extension = Extension::new();
    let request = json!({
        "action": "change-public-keys",
        "publicKey": extension.public_key(),
        "nonce": STANDARD.encode(nonce()),
    });
    frame::write(&mut stream, &serde_json::to_vec(&request).unwrap()).unwrap();
    let reply: Value = serde_json::from_slice(&frame::read(&mut stream).unwrap().unwrap()).unwrap();
    assert_eq!(reply["success"], "true");
    assert!(reply["publicKey"].is_string());

    drop(stream);
    served.join().unwrap();
}

// The signal registry is one per process, so the tests that register with it
// run one at a time: with both up at once, one's broadcasts would land on the
// other's connection, ahead of the frames it is counting.
static SIGNAL_TESTS: Mutex<()> = Mutex::new(());

#[test]
fn a_lock_signal_reaches_every_connection_and_drops_the_gone_ones() {
    let _one_at_a_time = SIGNAL_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let mut extension = TcpStream::connect(address).unwrap();
    let (open, _) = listener.accept().unwrap();
    let _gone_peer = TcpStream::connect(address).unwrap();
    let (gone, _) = listener.accept().unwrap();
    gone.shutdown(std::net::Shutdown::Write).unwrap();

    let open = Arc::new(Mutex::new(open));
    let gone = Arc::new(Mutex::new(gone));
    let open_id = server::register(open.clone());
    server::register(gone.clone());
    server::notify_locked();
    server::notify_unlocked();

    // In order, on the connection that reads.
    let signal: Value =
        serde_json::from_slice(&frame::read(&mut extension).unwrap().unwrap()).unwrap();
    assert_eq!(signal, json!({ "action": "database-locked" }));
    let signal: Value =
        serde_json::from_slice(&frame::read(&mut extension).unwrap().unwrap()).unwrap();
    assert_eq!(signal, json!({ "action": "database-unlocked" }));

    // The gone one's write failed on its own thread, which let the writer go.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while Arc::strong_count(&gone) > 1 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        Arc::strong_count(&gone),
        1,
        "a failed write drops the writer"
    );
    assert_eq!(Arc::strong_count(&open), 2, "a live one stays registered");

    // The connection ending lets its writer go too.
    server::forget(open_id);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while Arc::strong_count(&open) > 1 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(Arc::strong_count(&open), 1, "a forgotten one is let go");
}

#[test]
fn a_peer_that_stops_reading_stalls_no_one_else() {
    let _one_at_a_time = SIGNAL_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    // A peer that never reads: its socket buffer fills, then writes block.
    let _stalled_peer = TcpStream::connect(address).unwrap();
    let (stalled, _) = listener.accept().unwrap();
    let mut extension = TcpStream::connect(address).unwrap();
    let (open, _) = listener.accept().unwrap();

    let stalled_id = server::register(Arc::new(Mutex::new(stalled)));
    let open_id = server::register(Arc::new(Mutex::new(open)));
    // Far more than a socket buffer holds; every one of these would block a
    // shared thread on the stalled peer.
    for _ in 0..4096 {
        server::notify_locked();
    }
    server::notify_unlocked();

    // The reading peer still gets every signal, in order.
    for _ in 0..4096 {
        let signal: Value =
            serde_json::from_slice(&frame::read(&mut extension).unwrap().unwrap()).unwrap();
        assert_eq!(signal, json!({ "action": "database-locked" }));
    }
    let signal: Value =
        serde_json::from_slice(&frame::read(&mut extension).unwrap().unwrap()).unwrap();
    assert_eq!(signal, json!({ "action": "database-unlocked" }));

    server::forget(stalled_id);
    server::forget(open_id);
}

#[test]
fn bind_listens_at_the_root_and_takes_over_a_socket_left_behind() {
    use interprocess::local_socket::{prelude::*, Stream};
    let dir = tempfile::tempdir().unwrap();
    let listener = server::bind(dir.path()).expect("a listener at a fresh root");
    let mut client = Stream::connect(socket_name(dir.path()).unwrap()).unwrap();
    let mut served = listener.accept().unwrap();
    frame::write(&mut client, b"{}").unwrap();
    assert_eq!(
        frame::read(&mut served).unwrap().as_deref(),
        Some(&b"{}"[..])
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join(super::SOCKET_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "owner-only socket file");
    }

    // A crash leaves the file behind with nobody listening: the next launch
    // binds over it rather than refusing.
    std::mem::forget(listener);
    let again = server::bind(dir.path()).expect("a listener over a stale socket file");
    drop(again);
}

#[test]
fn bind_makes_a_missing_socket_directory_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir
        .path()
        .join("Group Containers")
        .join("group")
        .join("dev");
    let listener = server::bind(&root).expect("a listener in a directory not yet made");
    assert!(root.join(super::SOCKET_FILE).exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for made in [&root, root.parent().unwrap()] {
            let mode = std::fs::metadata(made).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "{} is owner-only", made.display());
        }
    }
    drop(listener);
}

#[test]
fn the_group_container_is_the_team_prefixed_one_under_the_home() {
    assert_eq!(
        group_container(std::path::Path::new("/Users/someone")),
        std::path::Path::new(
            "/Users/someone/Library/Group Containers/UFBL3F444A.app.rowel.desktop"
        )
    );
    // The iOS group is `group.`-prefixed, which macOS gates behind a profile.
    assert_ne!(DESKTOP_APP_GROUP, rowel_core::app::APP_GROUP);
    assert!(!DESKTOP_APP_GROUP.starts_with("group."));
}

#[test]
fn the_safari_socket_is_in_the_group_container_while_its_path_fits() {
    use std::path::Path;
    let home = |len: usize| format!("/Users/{}", "a".repeat(len));
    // `sun_path` is 104 bytes on macOS, the NUL included, and the group
    // container's path is long: a short name of up to 29 characters fits a
    // release build's socket, 25 a debug build's (`dev/`).
    for (debug, fits) in [(false, 29), (true, 25)] {
        let dir = group_socket_dir(Path::new(&home(fits)), debug).expect("the longest that fits");
        let expected = group_container(Path::new(&home(fits)));
        assert_eq!(
            dir,
            if debug {
                expected.join("dev")
            } else {
                expected
            }
        );
        assert_eq!(
            dir.join(super::SOCKET_FILE).as_os_str().len() + 1,
            SUN_PATH_MAX
        );

        let too_long = home(fits + 1);
        match group_socket_dir(Path::new(&too_long), debug) {
            Err(SafariSocketError::TooLong { path, bytes }) => {
                assert_eq!(bytes, SUN_PATH_MAX + 1);
                assert!(path.ends_with(super::SOCKET_FILE));
                assert!(path.starts_with(&too_long));
            }
            other => panic!("a short name of {} characters: {other:?}", fits + 1),
        }
    }
}

#[test]
fn the_app_group_is_the_one_the_app_is_entitled_to() {
    let entitlements = include_str!("../../Entitlements.plist");
    assert!(entitlements.contains("<key>com.apple.security.application-groups</key>"));
    assert!(entitlements.contains(&format!("<string>{DESKTOP_APP_GROUP}</string>")));
}

// The Safari extension is Swift (src-tauri/gen/safari) and has to agree with
// this side on where the socket is and how a frame is capped: held to the
// constants here, since nothing else would notice the two drifting apart.
#[test]
fn the_safari_extension_agrees_on_the_group_the_socket_and_the_cap() {
    let entitlements = include_str!("../../gen/safari/rowel_safari/rowel_safari.entitlements");
    assert!(entitlements.contains(&format!("<string>{DESKTOP_APP_GROUP}</string>")));
    let connection = include_str!("../../gen/safari/Sources/HostConnection.swift");
    assert!(connection.contains(&format!("appGroup = \"{DESKTOP_APP_GROUP}\"")));
    assert!(connection.contains(&format!("socketFile = \"{}\"", super::SOCKET_FILE)));
    let framing = include_str!("../../gen/safari/Sources/Frame.swift");
    assert_eq!(frame::MAX_FRAME, 1024 * 1024);
    assert!(framing.contains("maxLength = 1024 * 1024"));
}

#[test]
fn a_browser_launch_is_told_by_what_it_puts_on_the_command_line() {
    fn launched(list: &[&str]) -> bool {
        proxy::launched_by_browser(list.iter().map(|s| s.to_string()))
    }
    assert!(launched(&[
        "chrome-extension://oboonakemofpalcgghocfoadofidjkkk/"
    ]));
    assert!(launched(&["chrome-extension://abc/", "--parent-window=0"]));
    assert!(launched(&[
        "/path/manifest.json",
        "keepassxc-browser@keepassxc.org"
    ]));
    assert!(launched(&["/path/manifest.json", "browser@rowel.app"]));
    assert!(!launched(&["/path/manifest.json", "other@example.org"]));
    assert!(!launched(&[]));
    assert!(!launched(&["/Users/me/backup.rowel"]));
}

// --- manifests ---------------------------------------------------------------

#[test]
fn the_identifier_is_the_one_tauri_builds_with() {
    let conf: Value = serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
    assert_eq!(conf["identifier"], IDENTIFIER);
}

#[test]
fn rowels_manifest_names_its_own_host_and_extension() {
    let exe = std::path::Path::new("/Applications/Rowel.app/Contents/MacOS/rowel");
    let chromium: Value =
        serde_json::from_str(&manifest::manifest(&ROWEL, Family::Chromium, exe)).unwrap();
    assert_eq!(chromium["name"], "app.rowel.browser");
    assert_eq!(chromium["type"], "stdio");
    assert_eq!(chromium["path"], exe.to_string_lossy().as_ref());
    assert_eq!(
        chromium["allowed_origins"],
        json!([
            "chrome-extension://dimghkhcdfaokfingegmgbnpnpcoeofj/",
            "chrome-extension://aajfpjaphnegnekpggjnocmgbhkabeke/"
        ])
    );
    assert!(chromium.get("allowed_extensions").is_none());

    let firefox: Value =
        serde_json::from_str(&manifest::manifest(&ROWEL, Family::Firefox, exe)).unwrap();
    assert_eq!(firefox["name"], "app.rowel.browser");
    assert_eq!(firefox["allowed_extensions"], json!(["browser@rowel.app"]));
    assert!(firefox.get("allowed_origins").is_none());
}

#[test]
fn the_compatibility_manifest_names_keepassxcs_host_and_the_stock_extension() {
    let exe = std::path::Path::new("/Applications/Rowel.app/Contents/MacOS/rowel");
    let chromium: Value =
        serde_json::from_str(&manifest::manifest(&KEEPASSXC, Family::Chromium, exe)).unwrap();
    assert_eq!(chromium["name"], "org.keepassxc.keepassxc_browser");
    assert_eq!(chromium["path"], exe.to_string_lossy().as_ref());
    assert_eq!(
        chromium["allowed_origins"],
        json!([
            "chrome-extension://oboonakemofpalcgghocfoadofidjkkk/",
            "chrome-extension://pdffhmdngciaglkoonimfcmckehcpafo/"
        ])
    );
    assert!(chromium.get("allowed_extensions").is_none());

    let firefox: Value =
        serde_json::from_str(&manifest::manifest(&KEEPASSXC, Family::Firefox, exe)).unwrap();
    assert_eq!(
        firefox["allowed_extensions"],
        json!(["keepassxc-browser@keepassxc.org"])
    );
    assert!(firefox.get("allowed_origins").is_none());
}

#[test]
fn a_manifest_in_place_is_told_apart_from_keepassxcs_own() {
    use manifest::{classify, Found};
    let exe = std::path::Path::new("/Applications/Rowel.app/Contents/MacOS/rowel");
    let ours = manifest::manifest(&KEEPASSXC, Family::Chromium, exe);
    assert_eq!(classify(&KEEPASSXC, &ours, exe), Found::Current);

    let moved = std::path::Path::new("/opt/rowel/rowel");
    assert_eq!(classify(&KEEPASSXC, &ours, moved), Found::Stale);

    assert_eq!(classify(&KEEPASSXC, &keepassxcs_own(), exe), Found::Foreign);
    assert_eq!(classify(&KEEPASSXC, "not json", exe), Found::Foreign);

    let rowel = manifest::manifest(&ROWEL, Family::Chromium, exe);
    assert_eq!(classify(&ROWEL, &rowel, exe), Found::Current);
}

// The places a browser reads manifests from, as a directory: one folder per
// browser, a `<host name>.json` in each. Browsers not `present` are not here.
struct Places {
    dir: tempfile::TempDir,
    present: Vec<&'static str>,
}

impl Places {
    fn new(present: &[&'static str]) -> Self {
        Places {
            dir: tempfile::tempdir().unwrap(),
            present: present.to_vec(),
        }
    }

    fn path(&self, host: &manifest::NativeHost, browser: &str) -> std::path::PathBuf {
        self.dir
            .path()
            .join(browser)
            .join(format!("{}.json", host.name))
    }

    fn read(&self, host: &manifest::NativeHost, browser: &str) -> Option<String> {
        std::fs::read_to_string(self.path(host, browser)).ok()
    }

    fn put(&self, host: &manifest::NativeHost, browser: &str, text: &str) {
        let path = self.path(host, browser);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn status(&self, exe: &std::path::Path, compat: bool, browser: &str) -> manifest::Status {
        manifest::status_in(self, Some(exe), compat)
            .into_iter()
            .find(|status| status.id == browser)
            .unwrap()
    }
}

impl manifest::Places for Places {
    fn detected(&self, browser: &manifest::Browser) -> bool {
        self.present.contains(&browser.id)
    }

    fn existing(&self, host: &manifest::NativeHost, browser: &manifest::Browser) -> Option<String> {
        self.read(host, browser.id)
    }

    fn install(
        &self,
        host: &manifest::NativeHost,
        browser: &manifest::Browser,
        manifest: &str,
    ) -> std::io::Result<()> {
        self.put(host, browser.id, manifest);
        Ok(())
    }

    fn remove(
        &self,
        host: &manifest::NativeHost,
        browser: &manifest::Browser,
    ) -> std::io::Result<()> {
        match std::fs::remove_file(self.path(host, browser.id)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

fn keepassxcs_own() -> String {
    json!({
        "name": KEEPASSXC.name,
        "description": "KeePassXC integration with native messaging support",
        "path": "/Applications/KeePassXC.app/Contents/MacOS/keepassxc-proxy",
        "type": "stdio",
        "allowed_origins": ["chrome-extension://oboonakemofpalcgghocfoadofidjkkk/"]
    })
    .to_string()
}

const EXE: &str = "/Applications/Rowel.app/Contents/MacOS/rowel";

#[test]
fn install_writes_both_names_with_compatibility_on() {
    let exe = std::path::Path::new(EXE);
    let places = Places::new(&["chrome", "firefox"]);

    manifest::install_in(&places, Some(exe), true);

    for (browser, family) in [("chrome", Family::Chromium), ("firefox", Family::Firefox)] {
        assert_eq!(
            places.read(&ROWEL, browser),
            Some(manifest::manifest(&ROWEL, family, exe))
        );
        assert_eq!(
            places.read(&KEEPASSXC, browser),
            Some(manifest::manifest(&KEEPASSXC, family, exe))
        );
        let status = places.status(exe, true, browser);
        assert!(status.installed);
        assert!(!status.conflict);
    }
    // A browser not on this machine is not registered with.
    assert_eq!(places.read(&ROWEL, "edge"), None);
    assert!(!places.status(exe, true, "edge").installed);
}

#[test]
fn install_writes_only_rowels_name_with_compatibility_off() {
    let exe = std::path::Path::new(EXE);
    let places = Places::new(&["chrome"]);

    manifest::install_in(&places, Some(exe), false);

    assert!(places.read(&ROWEL, "chrome").is_some());
    assert_eq!(places.read(&KEEPASSXC, "chrome"), None);
    assert!(places.status(exe, false, "chrome").installed);
}

#[test]
fn turning_compatibility_off_takes_back_only_the_keepassxc_manifest() {
    let exe = std::path::Path::new(EXE);
    let places = Places::new(&["chrome"]);
    manifest::install_in(&places, Some(exe), true);

    manifest::install_in(&places, Some(exe), false);

    assert!(places.read(&ROWEL, "chrome").is_some());
    assert_eq!(places.read(&KEEPASSXC, "chrome"), None);
}

// The layout from before Rowel had a name of its own: one manifest, under
// KeePassXC's name, for an executable since moved. It is rewritten with
// compatibility on and taken back with it off, never left pointing at nothing.
#[test]
fn a_manifest_from_before_the_rename_is_rewritten_or_taken_back() {
    let exe = std::path::Path::new(EXE);
    let old = manifest::manifest(
        &KEEPASSXC,
        Family::Chromium,
        std::path::Path::new("/opt/rowel/rowel"),
    );

    let places = Places::new(&["chrome"]);
    places.put(&KEEPASSXC, "chrome", &old);
    manifest::install_in(&places, Some(exe), true);
    assert_eq!(
        places.read(&KEEPASSXC, "chrome"),
        Some(manifest::manifest(&KEEPASSXC, Family::Chromium, exe))
    );

    let places = Places::new(&["chrome"]);
    places.put(&KEEPASSXC, "chrome", &old);
    manifest::install_in(&places, Some(exe), false);
    assert_eq!(places.read(&KEEPASSXC, "chrome"), None);
    assert!(places.read(&ROWEL, "chrome").is_some());
}

#[test]
fn keepassxcs_own_manifest_is_left_alone_and_reported_only_with_compatibility_on() {
    let exe = std::path::Path::new(EXE);
    let places = Places::new(&["chrome"]);
    places.put(&KEEPASSXC, "chrome", &keepassxcs_own());

    manifest::install_in(&places, Some(exe), true);
    assert_eq!(places.read(&KEEPASSXC, "chrome"), Some(keepassxcs_own()));
    let status = places.status(exe, true, "chrome");
    // Rowel's own extension still reaches it: the clash is KeePassXC-Browser's.
    assert!(status.installed);
    assert!(status.conflict);

    manifest::install_in(&places, Some(exe), false);
    assert_eq!(places.read(&KEEPASSXC, "chrome"), Some(keepassxcs_own()));
    assert!(!places.status(exe, false, "chrome").conflict);

    manifest::remove_in(&places, Some(exe));
    assert_eq!(places.read(&KEEPASSXC, "chrome"), Some(keepassxcs_own()));
}

#[test]
fn remove_takes_back_both_names_from_every_browser() {
    let exe = std::path::Path::new(EXE);
    let mut places = Places::new(&["chrome", "firefox"]);
    manifest::install_in(&places, Some(exe), true);
    // Written while the browser was here; it has gone since.
    places.present.clear();

    manifest::remove_in(&places, Some(exe));

    for browser in ["chrome", "firefox"] {
        assert_eq!(places.read(&ROWEL, browser), None);
        assert_eq!(places.read(&KEEPASSXC, browser), None);
        assert!(!places.status(exe, true, browser).installed);
    }
}

// Windows' flat manifest from before the rename, `<data>/browser/<id>.json`:
// kept while KeePassXC's registry value names it — however that value spells
// the path — and deleted once the value names anything else, or is gone.
#[test]
fn the_legacy_windows_manifest_goes_only_once_the_registry_stops_naming_it() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("browser").join("chrome.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, "{}").unwrap();
    let spelled = legacy.to_string_lossy().into_owned();

    for named in [
        spelled.clone(),
        spelled.to_uppercase(),
        spelled.replace('/', "\\"),
        format!("{spelled}\\"),
    ] {
        manifest::remove_unless_named(&legacy, Some(&named)).unwrap();
        assert!(
            legacy.exists(),
            "deleted while the registry named it as {named}"
        );
    }

    let rewritten = dir
        .path()
        .join("browser")
        .join("chrome")
        .join(format!("{}.json", KEEPASSXC.name));
    manifest::remove_unless_named(&legacy, Some(&rewritten.to_string_lossy())).unwrap();
    assert!(!legacy.exists());

    // Already gone, and no key left: nothing to do, and no error.
    manifest::remove_unless_named(&legacy, None).unwrap();

    std::fs::write(&legacy, "{}").unwrap();
    manifest::remove_unless_named(&legacy, None).unwrap();
    assert!(!legacy.exists());
}

#[test]
fn every_browser_has_a_place_on_every_platform() {
    for browser in manifest::BROWSERS {
        assert!(!browser.id.is_empty());
        assert!(!browser.label.is_empty());
    }
    assert_eq!(str_of(&Map::new(), "missing"), "");
}
