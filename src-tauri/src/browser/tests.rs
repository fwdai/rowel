use std::sync::Mutex;

use serde_json::{json, Map, Value};

use super::manifest::{self, Family, KEEPASSXC, ROWEL};
use super::protocol::str_of;
use super::protocol::Code;
use super::{
    client_covered, clients_in, covered, decide, env_var, frame, group_container, group_socket_dir,
    item_by_name, outcome, proxy, resolved, settle, visible_in, Answer, AskKind, Asking, Candidate,
    Client, ClientKind, GrantAnswer, Lockbox, Locked, NotAsked, Pending, Prepared,
    SafariSocketError, StepUp, Verdict, IDENTIFIER, SUN_PATH_MAX,
};
use crate::agents::{self, AccessState, AgentState};
use crate::crypto::PayloadCipher;
use crate::models::Entry;
use crate::store::{migrate, SqliteStore, VaultStore};
use rowel_broker::agents::switch_changed;
use rowel_broker::grant::{
    Approval, CandidateItem, CreatedBy, Grant, GrantRequest, ItemSelector, Mode, ResolveContext,
    ResolveTarget, State, Target, Terms, VisibleItem,
};
use rowel_broker::lockbox::{live_rules, write_rules};
use rowel_broker::policy::{defaults, Limits};
use rowel_broker::reference::{fields_for, Reference};
use rowel_broker::testing::{seed, vault};
use rowel_broker::Signal;
use rowel_core::app::DESKTOP_APP_GROUP;
use zeroize::Zeroizing;

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

// A grant ask whose switches changed since it was decided never goes up:
// its `prepare` refuses under the slot lock, nothing reaches the screen,
// and the slot is free for the next ask.
#[test]
fn a_grant_ask_whose_access_moved_never_goes_up() {
    let pending: Pending<bool> = Pending::new();
    let mut held = AgentState::default();
    held.initialise(AccessState {
        enabled: true,
        lockbox_on: true,
    });
    let decided_under = held.access_epoch;
    held.bump_access();

    let mut emitted = false;
    let asked = pending.try_ask_with("g", AskKind::Grant { step_up: false }, || {
        held.access_since(decided_under)?;
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
        pending.answer("next", Answer::Pairing(None)),
        "and is still up"
    );
    assert!(matches!(next.try_recv(), Ok(Answer::Pairing(None))));

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
// under the session and agents locks, over the open store and what the
// agents lock guards — the grants held and the switches as the session
// holds them; here they run over a store of their own, with the locks the
// tests' own `&mut`. `switched` is a writer's change of a switch, as
// `change_access` makes it under the same locks.

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

/// Both switches on.
fn on() -> AccessState {
    AccessState {
        enabled: true,
        lockbox_on: true,
    }
}

/// What the agents lock guards in a session whose first request has read
/// the switches, both on.
fn ready() -> AgentState {
    let mut held = AgentState::default();
    held.initialise(on());
    held
}

fn locked<'a>(store: &'a SqliteStore, held: &'a mut AgentState, now: i64) -> Locked<'a> {
    Locked {
        store,
        held,
        writer: "dev",
        now,
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

/// The extension every request below comes in through.
fn extension() -> Client {
    client("Work Chrome", "ext", ClientKind::Extension)
}

/// An agent let into the lockbox.
fn agent() -> Client {
    Client {
        platform: Some("claude-code".into()),
        ..client("Claude Code", "agent", ClientKind::Agent)
    }
}

/// Let `clients` into the store, as `associate` writes them.
fn let_in(store: &SqliteStore, clients: &[Client]) {
    store
        .meta_set("browser_clients", &serde_json::to_string(clients).unwrap())
        .unwrap();
}

fn decided(store: &SqliteStore, held: &mut AgentState, now: i64) -> Result<Prepared, Code> {
    decide(
        locked(store, held, now),
        "default",
        &extension(),
        fill_request(&["gh"]),
        &[],
    )
}

/// The ask the baseline rule makes of the page's one login.
fn asked(store: &SqliteStore, held: &mut AgentState, now: i64) -> Asking {
    match decided(store, held, now) {
        Ok(Prepared::Ask(asking)) => *asking,
        _ => panic!("the baseline asks"),
    }
}

/// A writer's change of a switch, as `change_access` makes it under the
/// locks: the file written and the session's copy given the same value
/// (`change`) in one section, then what follows from it.
fn switched(
    store: &SqliteStore,
    held: &mut AgentState,
    reason: &str,
    change: impl FnOnce(&mut AccessState),
    now: i64,
) {
    let (_, changed) =
        agents::change_under_locks(Some(store), held, "dev", reason, now, |access| {
            if let Some(access) = access {
                change(access);
            }
            Ok(((), true))
        })
        .unwrap();
    assert!(changed);
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

// A grant this client holds for the page is served again, once — while the
// switches the session holds say so, looked at under the same locks a
// change of them is made under. The lockbox's access off is a refusal on
// record; the app's switch off is the extension's `AgentsDisabled`, with no
// row. Either ends the grant held, in the same section as the change, so
// nothing is left to reuse anyway.
#[test]
fn a_reuse_is_served_until_a_switch_goes_off() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &github_login("gh"));
    let now = 1_000;
    let mut held = ready();
    held.grants.push(active_grant("gh", now));

    let served = decided(&store, &mut held, now);
    assert!(matches!(
        served,
        Ok(Prepared::Answered(reply)) if reply.items.len() == 1 && reply.items[0].id == "gh"
    ));
    assert_eq!(held.grants[0].uses, 1);
    assert_eq!(rows(&store), vec![row("used", r#"{"reuse":true}"#)]);

    switched(
        &store,
        &mut held,
        "lockbox",
        |access| access.lockbox_on = false,
        now,
    );
    assert_eq!(held.grants[0].state, State::Revoked);
    assert!(matches!(
        decided(&store, &mut held, now),
        Err(Code::GrantDenied)
    ));
    assert_eq!(held.grants[0].uses, 1, "nothing released");
    assert_eq!(
        rows(&store)[1..],
        [
            row("revoked", r#"{"reason":"lockbox"}"#),
            row("denied", r#"{"reason":"lockbox"}"#)
        ]
    );

    let mut held = ready();
    held.grants.push(active_grant("gh", now));
    switched(
        &store,
        &mut held,
        "switch",
        |access| access.enabled = false,
        now,
    );
    assert!(matches!(
        decided(&store, &mut held, now),
        Err(Code::AgentsDisabled)
    ));
    assert_eq!(held.grants[0].uses, 0, "nothing released");
    assert_eq!(
        rows(&store).len(),
        4,
        "the `revoked` row, and no `denied` one"
    );
}

// A standing rule allows without asking: the grant is made and served in
// one go — after the same look at the switches. A session whose switches
// are not read yet, or read off, makes no grant at all.
#[test]
fn an_auto_release_requires_the_switches_the_session_holds() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &github_login("gh"));
    let now = 1_000;
    write_rules(
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

    // Not read yet: as good as off.
    let mut unread = AgentState::default();
    assert!(matches!(
        decided(&store, &mut unread, now),
        Err(Code::AgentsDisabled)
    ));
    assert!(unread.grants.is_empty());
    assert!(rows(&store).is_empty());

    let mut held = AgentState::default();
    held.initialise(AccessState {
        enabled: true,
        lockbox_on: false,
    });
    assert!(matches!(
        decided(&store, &mut held, now),
        Err(Code::GrantDenied)
    ));
    assert!(held.grants.is_empty(), "no grant was made");
    assert_eq!(rows(&store), vec![row("denied", r#"{"reason":"lockbox"}"#)]);

    let mut held = ready();
    assert!(matches!(
        decided(&store, &mut held, now),
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
// ends the grant in the same section as the change, and the switches the
// session holds are looked at first, whatever the list says.
#[test]
fn a_held_grant_covers_nothing_once_the_lockbox_access_went_off() {
    let (_dir, store, _cipher) = vault();
    let now = 1_000;
    let mut held = ready();
    held.grants.push(active_grant("gh", now));
    let id = held.grants[0].id.clone();

    assert!(covered(
        locked(&store, &mut held, now),
        "ext",
        &id,
        "gh",
        Mode::Fill
    ));
    assert_eq!(held.grants[0].uses, 1);
    assert_eq!(rows(&store), vec![row("used", r#"{"via":"grantId"}"#)]);

    switched(
        &store,
        &mut held,
        "lockbox",
        |access| access.lockbox_on = false,
        now,
    );
    assert_eq!(held.grants[0].state, State::Revoked);
    assert!(!covered(
        locked(&store, &mut held, now),
        "ext",
        &id,
        "gh",
        Mode::Fill
    ));
    assert_eq!(held.grants[0].uses, 1, "not used");
    assert_eq!(
        rows(&store).len(),
        2,
        "the `revoked` row, and no `used` one"
    );

    // The switches alone refuse: a grant back on the list, with the access
    // still off, covers nothing; with it back on, it does.
    held.grants[0].state = State::Active;
    assert!(!covered(
        locked(&store, &mut held, now),
        "ext",
        &id,
        "gh",
        Mode::Fill
    ));
    assert_eq!(held.grants[0].uses, 1);
    switched(
        &store,
        &mut held,
        "lockbox",
        |access| access.lockbox_on = true,
        now,
    );
    assert!(covered(
        locked(&store, &mut held, now),
        "ext",
        &id,
        "gh",
        Mode::Fill
    ));
    assert_eq!(held.grants[0].uses, 2);

    // And a session whose switches are not read yet serves nothing.
    held.access = None;
    assert!(!covered(
        locked(&store, &mut held, now),
        "ext",
        &id,
        "gh",
        Mode::Fill
    ));
    assert_eq!(held.grants[0].uses, 2);
}

// An extension's TOTP step without a `grantId` (a marked extension whose
// own agent mode is off) is served under a grant it was issued or already
// served under, counted on that grant and logged as found by client; never
// under another client's grant it was not served under, nor one for another
// login.
#[test]
fn a_client_s_own_grant_covers_its_totp_step_and_no_one_else_s() {
    let (_dir, store, _cipher) = vault();
    let now = 1_000;
    let mut held = ready();
    held.grants.push(Grant {
        client_key: "agent".into(),
        ..active_grant("gh", now)
    });
    assert!(!client_covered(
        locked(&store, &mut held, now),
        "ext",
        "gh",
        Mode::Fill
    ));
    assert_eq!(held.grants[0].served_to, None, "not claimed");

    held.grants[0].served_to = Some("ext".into());
    held.grants.push(active_grant("gh2", now));
    assert!(!client_covered(
        locked(&store, &mut held, now),
        "ext",
        "gh3",
        Mode::Fill
    ));
    assert!(client_covered(
        locked(&store, &mut held, now),
        "ext",
        "gh",
        Mode::Fill
    ));
    let uses: Vec<u32> = held.grants.iter().map(|g| g.uses).collect();
    assert_eq!(uses, [1, 0], "the use is counted on the covering grant");
    assert_eq!(rows(&store), vec![row("used", r#"{"via":"client"}"#)]);
}

// The reviewer's sequence: a request is decided and its ask goes up; one
// writer turns the Agents switch off — the grants it finds active end, on
// record — and another turns it on again. The switches say on when the
// answer comes, but the epoch moved: the request is refused, and nothing
// is released.
#[test]
fn a_request_decided_before_two_changes_is_refused_and_the_grants_they_found_end() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &github_login("gh"));
    let now = 1_000;
    let mut held = ready();
    held.grants.push(active_grant("gh2", now));
    let decided_under = held.access_epoch;
    let asking = asked(&store, &mut held, now);
    assert_eq!(asking.access_epoch, decided_under);

    switched(
        &store,
        &mut held,
        "switch",
        |access| access.enabled = false,
        now,
    );
    assert_eq!(held.grants[0].state, State::Revoked);
    assert_eq!(rows(&store), vec![row("revoked", r#"{"reason":"switch"}"#)]);
    switched(
        &store,
        &mut held,
        "switch",
        |access| access.enabled = true,
        now,
    );
    assert_eq!(held.access_on(), Ok(()), "on again, for a new request");
    assert_eq!(held.access_epoch, decided_under + 2);

    assert!(matches!(
        settle(
            locked(&store, &mut held, now),
            &extension(),
            asking,
            yes("gh")
        ),
        Err(Code::GrantDenied)
    ));
    assert_eq!(held.grants.len(), 1, "no grant was made");
    assert_eq!(
        rows(&store)[1..],
        [
            ("requested".to_string(), None),
            row("denied", r#"{"reason":"switch"}"#)
        ]
    );
}

// Two writers of the Agents switch, racing: each writes the file and gives
// the session's copy the same value inside the one section
// (`change_under_locks`), so whoever looks under the lock — a release, the
// other writer, a reader — sees the file and the session agree, in any
// order; and a write that fails changes nothing, so they agree then too.
#[test]
fn two_writers_cannot_leave_the_file_and_the_session_disagreeing() {
    let held = Mutex::new(ready());
    let file = Mutex::new(true);
    let now = 1_000;
    std::thread::scope(|scope| {
        for writer in 0..4u32 {
            let (held, file) = (&held, &file);
            scope.spawn(move || {
                for turn in 0..50u32 {
                    let value = (writer + turn) % 2 == 0;
                    let mut held = held.lock().unwrap();
                    agents::change_under_locks(None, &mut held, "dev", "switch", now, |access| {
                        *file.lock().unwrap() = value;
                        if let Some(access) = access {
                            access.enabled = value;
                        }
                        Ok(((), true))
                    })
                    .unwrap();
                    assert_eq!(*file.lock().unwrap(), held.access.unwrap().enabled);
                }
            });
        }
        let (held, file) = (&held, &file);
        scope.spawn(move || {
            for _ in 0..200 {
                let held = held.lock().unwrap();
                assert_eq!(*file.lock().unwrap(), held.access.unwrap().enabled);
                drop(held);
                std::thread::yield_now();
            }
        });
    });
    let mut held = held.into_inner().unwrap();
    assert_eq!(*file.lock().unwrap(), held.access.unwrap().enabled);
    assert_eq!(held.access_epoch, 200, "every change moved the epoch");

    let before = held.access_epoch;
    let failed = agents::change_under_locks(None, &mut held, "dev", "switch", now, |_| {
        Err::<((), bool), _>(crate::error::Error::Other("disk full".into()))
    });
    assert!(failed.is_err());
    assert_eq!(held.access_epoch, before, "nothing changed");
    assert_eq!(*file.lock().unwrap(), held.access.unwrap().enabled);
}

// The session's first request reads the switches from the files and keeps
// them: a change written before it is what it reads, and what it keeps is
// what every later look sees — the files are not consulted again until the
// next session.
#[test]
fn the_first_request_of_a_session_reads_the_switches_and_keeps_them() {
    let mut held = AgentState::default();
    assert_eq!(held.access_on(), Err("switch"), "not read yet");

    // The Agents switch was turned off before the first request.
    let off = AccessState {
        enabled: false,
        lockbox_on: true,
    };
    assert_eq!(held.initialise(off), off);
    assert_eq!(held.access_on(), Err("switch"));
    // A later "first" read does not replace what the session holds.
    assert_eq!(held.initialise(on()), off);
    assert_eq!(held.access_on(), Err("switch"));

    // The next session reads afresh.
    held.reset();
    assert_eq!(held.initialise(on()), on());
    assert_eq!(held.access_on(), Ok(()));
}

// The session ends under a request: what decided before the seal is refused
// at settle (the epoch moved, the switches gone), the ask up in the slot is
// refused and the slot freed, and the next session's first request reads
// the switches afresh and asks — nothing is pending.
#[test]
fn a_seal_refuses_the_ask_up_and_what_decided_before_it() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &github_login("gh"));
    let now = 1_000;
    let mut held = ready();
    held.grants.push(active_grant("gh2", now));
    let asking = asked(&store, &mut held, now);
    let slot: Pending<Answer> = Pending::new();
    let (_, answer) = slot
        .begin("ask", AskKind::Grant { step_up: false }, up)
        .unwrap();

    // The seal, as `agents::clear` makes it: the state reset under the
    // agents lock, the slot after.
    held.reset();
    assert_eq!(slot.refuse_grant(), Some("ask".to_string()));
    assert!(matches!(answer.try_recv(), Ok(Answer::Grant(None))));
    assert!(held.grants.is_empty() && held.access.is_none());

    // The answer, settled in the next session: refused.
    held.initialise(on());
    assert!(matches!(
        settle(
            locked(&store, &mut held, now),
            &extension(),
            asking,
            yes("gh")
        ),
        Err(Code::GrantDenied)
    ));
    assert_eq!(
        rows(&store).last(),
        Some(&row("denied", r#"{"reason":"switch"}"#))
    );
    assert!(held.grants.is_empty());

    // The next request asks, and the slot is its to take.
    assert!(matches!(
        decided(&store, &mut held, now),
        Ok(Prepared::Ask(_))
    ));
    assert!(
        slot.begin("next", AskKind::Grant { step_up: false }, up)
            .is_ok(),
        "nothing pending"
    );
}

// The dialog's yes is settled under the same look at the switches, and the
// epoch the request was decided under: a change while the dialog was up
// refuses it, and the row names the switch that is off — or `switch`, for
// one that is on again.
#[test]
fn a_yes_after_a_switch_went_off_is_refused_and_says_which_switch() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &github_login("gh"));
    let now = 1_000;
    let mut held = ready();

    // The lockbox's access went off while the dialog was up.
    let asking = asked(&store, &mut held, now);
    switched(
        &store,
        &mut held,
        "lockbox",
        |access| access.lockbox_on = false,
        now,
    );
    assert!(matches!(
        settle(
            locked(&store, &mut held, now),
            &extension(),
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
    switched(
        &store,
        &mut held,
        "lockbox",
        |access| access.lockbox_on = true,
        now,
    );

    // The Agents switch went off and on again: both on now, but the epoch
    // moved.
    let asking = asked(&store, &mut held, now);
    switched(
        &store,
        &mut held,
        "switch",
        |access| access.enabled = false,
        now,
    );
    switched(
        &store,
        &mut held,
        "switch",
        |access| access.enabled = true,
        now,
    );
    assert!(matches!(
        settle(
            locked(&store, &mut held, now),
            &extension(),
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
    let asking = asked(&store, &mut held, now);
    let Ok(reply) = settle(
        locked(&store, &mut held, now),
        &extension(),
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

// A change that leaves the open lockbox's access on — the Agents switch
// turned on, another lockbox's access either way — moves the epoch, so a
// request caught in between is refused once, and nothing more: the grants
// held stay active, and the log says nothing.
#[test]
fn a_change_that_leaves_the_open_lockbox_on_moves_the_epoch_and_keeps_its_grants() {
    let (_dir, store, _cipher) = vault();
    let now = 1_000;
    let mut held = ready();
    held.grants.push(active_grant("gh", now));
    let before = held.access_epoch;

    // The Agents switch turned on, which it was; another lockbox's access
    // on, then off — not the open one's, so nothing of the session's to
    // change.
    switched(
        &store,
        &mut held,
        "switch",
        |access| access.enabled = true,
        now,
    );
    switched(&store, &mut held, "lockbox", |_| {}, now);
    switched(&store, &mut held, "lockbox", |_| {}, now);
    assert_eq!(held.access_epoch, before + 3);

    assert_eq!(held.grants[0].state, State::Active);
    assert!(rows(&store).is_empty(), "no `revoked` row");
    assert_eq!(held.access_on(), Ok(()), "a new request goes on");
    assert_eq!(
        held.access_since(before),
        Err("switch"),
        "one decided before any of it settles no more"
    );
}

// A change that cuts the open lockbox off — its own access going off, or
// the Agents switch — ends every active grant, with the row naming which.
#[test]
fn a_change_that_cuts_the_open_lockbox_off_ends_its_grants_and_says_which_switch() {
    let (_dir, store, _cipher) = vault();
    let now = 1_000;

    // The open lockbox's access off.
    let mut held = ready();
    held.grants.push(active_grant("gh", now));
    switched(
        &store,
        &mut held,
        "lockbox",
        |access| access.lockbox_on = false,
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
    let mut held = ready();
    held.grants.push(active_grant("gh", now));
    held.grants.push(active_grant("gh2", now));
    held.grants.push(Grant {
        state: State::Expired,
        ..active_grant("old", now)
    });
    switched(
        &store,
        &mut held,
        "switch",
        |access| access.enabled = false,
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
    let mut held = ready();
    held.grants.push(active_grant("gh", now));
    held.access.as_mut().unwrap().enabled = false;
    switch_changed(None, &mut held, "dev", "switch", now);
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

// --- agents as clients, against a store ----------------------------------------

// A client row from before there were kinds is `{"name","key"}`, and reads
// back as what it was: an extension, let in at time zero, from nowhere in
// particular.
#[test]
fn a_clients_row_from_before_kinds_reads_as_an_extension() {
    let (_dir, store, _cipher) = vault();
    store
        .meta_set("browser_clients", r#"[{"name":"Work Chrome","key":"ext"}]"#)
        .unwrap();
    let clients = clients_in(&store);
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].name, "Work Chrome");
    assert_eq!(clients[0].kind, ClientKind::Extension);
    assert_eq!(clients[0].created_at_ms, 0);
    assert_eq!(clients[0].platform, None);
    assert_eq!(clients[0].last_seen_at_ms, None);
}

/// What an agent asks for: the site, not a page, and no ids of its own.
fn agent_request(origin: &str) -> GrantRequest {
    GrantRequest {
        client_key: "agent".into(),
        agent_id: None,
        acting_for: None,
        items: ItemSelector::Lockbox,
        targets: vec![Target::Origin(origin.into())],
        modes: vec![Mode::Fill],
        run_id: Some("run-1".into()),
        reason: Some("deploying the docs".into()),
    }
}

fn fill_signal(signal: &Signal) -> Option<(&str, &str, &str, bool, i64)> {
    match signal {
        Signal::FillGrant {
            grant_id,
            origin,
            item,
            auto_submit,
            expires_at_ms,
        } => Some((grant_id, origin, &item.id, *auto_submit, *expires_at_ms)),
        Signal::GrantEnded { .. } => None,
    }
}

// Asking is counted only when a dialog goes up (`request_grant`, under the
// slot). A request the slot could not take is told "ask again shortly" and
// is decided again at every retry; none of those reaches a person, so the
// decisions alone must not spend the client's asks — or a dialog another
// client holds for fifteen seconds would rate-limit the one waiting on it.
#[test]
fn a_decision_that_waits_on_the_slot_is_not_an_ask_on_the_clients_record() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &github_login("gh"));
    let_in(&store, &[extension(), agent()]);
    let mut held = ready();
    let limits = Limits::default();
    for attempt in 0..limits.asks_per_minute + 5 {
        let now = 1_000 + i64::from(attempt) * 1_500;
        assert!(
            matches!(
                decide(
                    locked(&store, &mut held, now),
                    "default",
                    &agent(),
                    agent_request("https://github.com"),
                    &[]
                ),
                Ok(Prepared::Ask(_))
            ),
            "attempt {attempt} is decided, not rate-limited"
        );
    }
    assert!(held.rate("agent").check(&limits, 30_000).is_ok());
}

// An agent names a site; the candidates are the lockbox's logins for it,
// found by the plaintext host column, and the baseline asks about them. The
// yes activates the grant and tells every extension — and only the
// extensions — that there is a fill to make, with the page's origin and the
// one item. The rows name the agent as an agent, with its run.
#[test]
fn an_agents_fill_grant_is_signalled_to_the_extensions_and_not_to_the_agent() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &github_login("gh"));
    seed(&store, &cipher, &bank_login("bank", "https://bank.com"));
    let home = client("Home Firefox", "ext2", ClientKind::Extension);
    let_in(&store, &[extension(), home.clone(), agent()]);
    let now = 1_000;
    let mut held = ready();

    let asking = match decide(
        locked(&store, &mut held, now),
        "default",
        &agent(),
        agent_request("https://github.com"),
        &[],
    ) {
        Ok(Prepared::Ask(asking)) => *asking,
        other => panic!("the baseline asks: {:?}", other.is_ok()),
    };
    assert_eq!(asking.client_name, "Claude Code");
    assert_eq!(
        asking
            .candidates
            .iter()
            .map(|c| c.item.id.as_str())
            .collect::<Vec<_>>(),
        ["gh"],
        "the site's logins, not the bank's"
    );
    assert_eq!(asking.grant.reason.as_deref(), Some("deploying the docs"));
    assert!(
        held.signals.is_empty(),
        "nothing is signalled before the yes"
    );

    let reply = settle(
        locked(&store, &mut held, now),
        &agent(),
        asking,
        Verdict::Answered(Some(GrantAnswer {
            item_id: "gh".into(),
            standing_days: None,
        })),
    )
    .expect("the yes stands");
    assert_eq!(reply.state, State::Active);
    assert_eq!(reply.uses, 1);
    let grant = held.grants[0].clone();
    assert_eq!(grant.client_key, "agent");
    assert_eq!(grant.served_to, None);

    for key in ["ext", "ext2"] {
        let queued = held
            .signals
            .get(key)
            .unwrap_or_else(|| panic!("{key} is told"));
        assert_eq!(queued.len(), 1);
        assert_eq!(
            fill_signal(&queued[0]),
            Some((
                grant.id.as_str(),
                "https://github.com",
                "gh",
                false,
                grant.expires_at_ms
            ))
        );
    }
    assert!(
        !held.signals.contains_key("agent"),
        "the agent is never told to fill"
    );

    let rows = store.audit_rows("dev").unwrap();
    let events: Vec<&str> = rows.iter().map(|r| r.event.as_str()).collect();
    assert_eq!(events, ["requested", "granted", "used"]);
    assert!(rows.iter().all(|r| r.actor_kind == "agent"));
    assert!(rows.iter().all(|r| r.run_id.as_deref() == Some("run-1")));
    assert_eq!(rows[0].target.as_deref(), Some("https://github.com"));

    // The extension's own grant, in agent mode, is nobody's to signal: it
    // holds the reply.
    let asking = asked(&store, &mut held, now);
    settle(
        locked(&store, &mut held, now),
        &extension(),
        asking,
        yes("gh"),
    )
    .unwrap();
    assert_eq!(held.signals["ext"].len(), 1, "no second signal");

    // The lockbox's access goes off: the agent's grant ends, and every
    // extension is told to sign out of the site.
    switched(
        &store,
        &mut held,
        "lockbox",
        |access| access.lockbox_on = false,
        now,
    );
    let ended = Signal::GrantEnded {
        grant_id: grant.id.clone(),
        origins: vec!["https://github.com".into()],
    };
    assert!(held.signals["ext"].contains(&ended));
    assert!(held.signals["ext2"].contains(&ended));
    assert!(!held.signals.contains_key("agent"));
}

// A site the lockbox has no login for is a refusal on record, before any
// dialog; an agent's `header` request has no site and is refused the same.
#[test]
fn an_agents_request_for_a_site_with_no_login_is_denied_on_record() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &github_login("gh"));
    let_in(&store, &[extension(), agent()]);
    let now = 1_000;
    let mut held = ready();

    assert!(matches!(
        decide(
            locked(&store, &mut held, now),
            "default",
            &agent(),
            agent_request("https://gitlab.com"),
            &[]
        ),
        Err(Code::GrantDenied)
    ));
    assert_eq!(
        rows(&store),
        vec![row("denied", r#"{"reason":"no_candidates"}"#)]
    );
    assert!(held.grants.is_empty() && held.signals.is_empty());
    assert_eq!(store.audit_rows("dev").unwrap()[0].actor_kind, "agent");
}

// Expiry ends an agent's grant the way the switch does: the row says
// expired, and the extensions are told to sign out.
#[test]
fn an_expired_grant_is_signalled_to_the_extensions() {
    let (_dir, store, _cipher) = vault();
    let_in(&store, &[extension(), agent()]);
    let mut held = ready();
    let grant = Grant {
        client_key: "agent".into(),
        ..active_grant("gh", 1_000)
    };
    held.grants.push(grant.clone());

    assert!(!agents::expire_due(
        Some(&store),
        &mut held,
        "dev",
        grant.expires_at_ms - 1
    ));
    assert!(held.signals.is_empty());
    assert!(agents::expire_due(
        Some(&store),
        &mut held,
        "dev",
        grant.expires_at_ms
    ));
    assert_eq!(held.grants[0].state, State::Expired);
    assert_eq!(rows(&store), vec![row("ended", r#"{"reason":"expired"}"#)]);
    assert_eq!(
        held.signals.get("ext"),
        Some(&vec![Signal::GrantEnded {
            grant_id: grant.id,
            origins: vec!["https://github.com".into()],
        }])
    );
    assert!(!held.signals.contains_key("agent"));
}

// An agent's fill grant goes to every extension, and the first one served
// under it claims it: the same extension may keep using it (the TOTP step,
// a retry), another is refused. The grant's own client is never claimed
// against.
#[test]
fn a_fill_grant_is_claimed_by_the_first_extension_served_under_it() {
    let (_dir, store, _cipher) = vault();
    let_in(&store, &[extension(), agent()]);
    let now = 1_000;
    let mut held = ready();
    held.grants.push(Grant {
        client_key: "agent".into(),
        ..active_grant("gh", now)
    });
    let id = held.grants[0].id.clone();
    let covers = |held: &mut AgentState, key: &str| {
        covered(locked(&store, held, now), key, &id, "gh", Mode::Fill)
    };

    assert!(
        covers(&mut held, "ext"),
        "the first extension served claims it"
    );
    assert_eq!(held.grants[0].served_to.as_deref(), Some("ext"));
    assert!(covers(&mut held, "ext"), "and keeps it");
    assert!(!covers(&mut held, "ext2"), "another is refused");
    assert_eq!(held.grants[0].uses, 2, "a refusal is not a use");
    assert_eq!(held.grants[0].served_to.as_deref(), Some("ext"));
    assert_eq!(
        rows(&store),
        vec![
            row("used", r#"{"via":"grantId"}"#),
            row("used", r#"{"via":"grantId"}"#)
        ]
    );
    assert!(
        store
            .audit_rows("dev")
            .unwrap()
            .iter()
            .all(|r| r.actor_kind == "agent"),
        "a use is the grant holder's"
    );

    // The extension's own grant is claimed by nobody: it is theirs already.
    held.grants.push(active_grant("gh2", now));
    let own = held.grants[1].id.clone();
    assert!(covered(
        locked(&store, &mut held, now),
        "ext",
        &own,
        "gh2",
        Mode::Fill
    ));
    assert_eq!(held.grants[1].served_to, None);
}

// What an agent may ask for by name is what its own rules and grants admit.
// The baseline names every client, so it is left out: it would make the
// whole lockbox visible to anyone let in.
#[test]
fn visible_items_are_the_clients_own_rules_and_grants_never_the_baseline() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &github_login("gh"));
    seed(&store, &cipher, &bank_login("bank", "https://bank.com"));
    seed(&store, &cipher, &github_login("gh2"));
    let now = 1_000;
    let mut held = ready();

    assert!(
        visible_in(&store, &held, &agent(), "default", now)
            .unwrap()
            .is_empty(),
        "the baseline alone shows nothing"
    );

    write_rules(
        &store,
        &[
            defaults::standing("agent", "gh", "https://github.com", 7, now),
            defaults::standing("ext", "bank", "https://bank.com", 7, now),
        ],
    )
    .unwrap();
    held.grants.push(Grant {
        client_key: "agent".into(),
        ..active_grant("gh2", now)
    });
    held.grants.push(Grant {
        client_key: "agent".into(),
        state: State::Expired,
        ..active_grant("bank", now)
    });

    let mut visible = visible_in(&store, &held, &agent(), "default", now).unwrap();
    visible.sort_by(|a, b| a.reference.cmp(&b.reference));
    assert_eq!(
        visible
            .iter()
            .map(|v| v.reference.as_str())
            .collect::<Vec<_>>(),
        ["rowel://default/gh", "rowel://default/gh2"],
        "its own rule and its own active grant; not the extension's rule, not an ended grant"
    );
    assert_eq!(visible[0].title, "GitHub");

    // The switches gate the names as they gate the uses.
    held.access.as_mut().unwrap().lockbox_on = false;
    assert_eq!(
        visible_in(&store, &held, &agent(), "default", now),
        Err(Code::GrantDenied),
        "the lockbox's switch is an explicit refusal"
    );
    held.access = Some(AccessState {
        enabled: false,
        lockbox_on: true,
    });
    assert_eq!(
        visible_in(&store, &held, &agent(), "default", now),
        Err(Code::AgentsDisabled),
        "the app's Agents switch is an explicit refusal"
    );
    held.access = Some(on());
    assert_eq!(visible[0].kind, "login");
    assert_eq!(visible[0].fields, ["password", "otp"]);

    let theirs = visible_in(&store, &held, &extension(), "default", now).unwrap();
    assert_eq!(theirs.len(), 1);
    assert_eq!(theirs[0].reference, "rowel://default/bank");

    assert_eq!(fields_for("apikey"), ["apiKey"]);
    assert_eq!(fields_for("ssh"), ["privateKey"]);
    assert_eq!(fields_for("card"), ["number", "cvc"]);
    assert_eq!(fields_for("env"), ["body"]);
    assert!(fields_for("note").is_empty());
}

// --- secrets for commands and APIs: the sidecar ----------------------------------
//
// A header or env grant is answered with nothing but its id; the value
// leaves only through `resolve`, which a sidecar calls under the grant an
// agent handed it. `resolved` is the whole of that under the locks, over a
// store of its own here.

/// `rowel run` / `rowel proxy`, let into the lockbox.
fn sidecar() -> Client {
    Client {
        platform: Some("cli".into()),
        ..client("rowel run", "side", ClientKind::Sidecar)
    }
}

fn api_key(id: &str, title: &str, key: Option<&str>) -> Entry {
    Entry {
        id: id.into(),
        kind: "apikey".into(),
        title: title.into(),
        api_key: key.map(String::from),
        ..Entry::default()
    }
}

fn env_file(id: &str, body: &str) -> Entry {
    Entry {
        id: id.into(),
        kind: "env".into(),
        title: "CI".into(),
        body: Some(body.into()),
        ..Entry::default()
    }
}

fn card(id: &str) -> Entry {
    Entry {
        id: id.into(),
        kind: "card".into(),
        title: "Visa".into(),
        number: Some("4242424242424242".into()),
        cvc: Some("123".into()),
        ..Entry::default()
    }
}

const DEPLOY: &str = "node deploy.js --prod";

fn stripe(methods: &[&str]) -> Target {
    Target::Http {
        host: "api.stripe.com".into(),
        path: "/v1/*".into(),
        methods: methods.iter().map(|m| m.to_string()).collect(),
    }
}

fn secret_request(
    client_key: &str,
    ids: &[&str],
    targets: Vec<Target>,
    modes: Vec<Mode>,
) -> GrantRequest {
    GrantRequest {
        client_key: client_key.into(),
        agent_id: None,
        acting_for: None,
        items: ItemSelector::Ids(ids.iter().map(|id| id.to_string()).collect()),
        targets,
        modes,
        run_id: Some("run-2".into()),
        reason: None,
    }
}

/// An active header grant of the agent's for `items` at Stripe, `GET` only.
fn header_grant(items: &[&str], now: i64) -> Grant {
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
        ..Grant::from_request(
            "default",
            secret_request("agent", items, vec![stripe(&["GET"])], vec![Mode::Header]),
            terms,
            now,
        )
    }
}

fn call(method: &str, host: &str) -> ResolveContext {
    ResolveContext {
        target: ResolveTarget::Http {
            host: host.into(),
            path: "/v1/balance".into(),
            method: method.into(),
        },
    }
}

fn run(line: &str) -> ResolveContext {
    ResolveContext {
        target: ResolveTarget::Command(line.into()),
    }
}

/// The open lockbox, `default`, named `Work (main)` in the registry.
fn lockbox() -> Lockbox {
    Lockbox {
        id: "default".into(),
        name: Some("Work (main)".into()),
    }
}

fn resolve(
    store: &SqliteStore,
    cipher: &PayloadCipher,
    held: &mut AgentState,
    now: i64,
    grant_id: &str,
    reference: &str,
    ctx: &ResolveContext,
) -> Result<Zeroizing<String>, Code> {
    resolved(
        locked(store, held, now),
        cipher,
        &lockbox(),
        &sidecar(),
        grant_id,
        &Reference::parse(reference).expect("a placeholder"),
        ctx,
    )
}

// The whole path: an agent holds a header grant for its Stripe key, the
// proxy resolves the key for one request under it, and the row says who
// resolved what, where, and through which field.
#[test]
fn a_sidecar_resolves_an_api_key_under_an_agents_header_grant() {
    let (_dir, store, cipher) = vault();
    seed(
        &store,
        &cipher,
        &api_key("stripe", "Stripe", Some("sk_live_x")),
    );
    let_in(&store, &[agent(), sidecar()]);
    let now = 1_000;
    let mut held = ready();
    held.grants.push(header_grant(&["stripe"], now));
    let id = held.grants[0].id.clone();

    let value = resolve(
        &store,
        &cipher,
        &mut held,
        now,
        &id,
        "rowel://default/stripe/apiKey",
        &call("GET", "api.stripe.com"),
    )
    .expect("served");
    assert_eq!(&*value, "sk_live_x");
    assert_eq!(held.grants[0].uses, 1);

    let rows = store.audit_rows("dev").unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.event, "used");
    assert_eq!(row.actor_kind, "sidecar");
    assert_eq!(row.actor_name.as_deref(), Some("rowel run"));
    assert_eq!(row.grant_id.as_deref(), Some(id.as_str()));
    assert_eq!(row.run_id.as_deref(), Some("run-2"));
    assert_eq!(row.item_id.as_deref(), Some("stripe"));
    assert_eq!(row.mode.as_deref(), Some("header"));
    assert_eq!(row.target.as_deref(), Some("api.stripe.com/v1/balance"));
    assert_eq!(
        row.detail.as_deref(),
        Some(r#"{"via":"resolve","field":"apiKey"}"#)
    );
}

// A placeholder may name the item by the slug of its title and the lockbox
// by the slug of its registry name; an id always works; two items with the
// same slug name neither.
#[test]
fn a_placeholder_names_the_item_by_slug_and_the_lockbox_by_its_name() {
    let (_dir, store, cipher) = vault();
    seed(
        &store,
        &cipher,
        &api_key("live", "Stripe (live)", Some("sk_live")),
    );
    seed(&store, &cipher, &api_key("dup1", "Dup", Some("one")));
    seed(&store, &cipher, &api_key("dup2", "dup!", Some("two")));
    let_in(&store, &[agent(), sidecar()]);
    let now = 1_000;
    let mut held = ready();
    held.grants
        .push(header_grant(&["live", "dup1", "dup2"], now));
    let id = held.grants[0].id.clone();
    let get = call("GET", "api.stripe.com");

    let metas = store.list().unwrap();
    assert_eq!(
        item_by_name(&metas, "live").map(|m| m.id.as_str()),
        Some("live")
    );
    assert_eq!(
        item_by_name(&metas, "stripe-live").map(|m| m.id.as_str()),
        Some("live")
    );
    assert_eq!(item_by_name(&metas, "dup"), None, "ambiguous");
    assert_eq!(item_by_name(&metas, "nope"), None);

    let served = resolve(
        &store,
        &cipher,
        &mut held,
        now,
        &id,
        "rowel://work-main/stripe-live/apiKey",
        &get,
    );
    assert_eq!(served.as_deref().map(String::as_str), Ok("sk_live"));
    let served = resolve(
        &store,
        &cipher,
        &mut held,
        now,
        &id,
        "rowel://default/dup2/apiKey",
        &get,
    );
    assert_eq!(served.as_deref().map(String::as_str), Ok("two"));
    assert_eq!(
        resolve(
            &store,
            &cipher,
            &mut held,
            now,
            &id,
            "rowel://default/dup/apiKey",
            &get
        )
        .err(),
        Some(Code::GrantDenied),
        "two Dups name neither"
    );
    assert_eq!(held.grants[0].uses, 2);
    assert_eq!(rows(&store).len(), 2);
}

// Every way a resolve can fail to fit is the one word, and releases
// nothing: no use on the grant, no row on the log.
#[test]
fn a_resolve_that_does_not_fit_the_grant_is_denied_with_no_row_and_no_use() {
    let (_dir, store, cipher) = vault();
    seed(
        &store,
        &cipher,
        &api_key("stripe", "Stripe", Some("sk_live")),
    );
    seed(
        &store,
        &cipher,
        &api_key("other", "Other", Some("sk_other")),
    );
    seed(&store, &cipher, &api_key("blank", "Blank", Some("")));
    seed(&store, &cipher, &github_login("gh"));
    let_in(&store, &[agent(), sidecar()]);
    let now = 1_000;
    let mut held = ready();
    held.grants.push(header_grant(&["stripe", "blank"], now));
    held.grants.push(Grant {
        client_key: "agent".into(),
        ..active_grant("gh", now)
    });
    let header = held.grants[0].id.clone();
    let fill = held.grants[1].id.clone();
    let get = call("GET", "api.stripe.com");
    let stripe_key = "rowel://default/stripe/apiKey";

    let denied =
        |held: &mut AgentState, grant: &str, reference: &str, ctx: &ResolveContext, why: &str| {
            assert_eq!(
                resolve(&store, &cipher, held, now, grant, reference, ctx).err(),
                Some(Code::GrantDenied),
                "{why}"
            );
        };

    held.access.as_mut().unwrap().lockbox_on = false;
    denied(
        &mut held,
        &header,
        stripe_key,
        &get,
        "the lockbox's access is off",
    );
    held.access = Some(AccessState {
        enabled: false,
        lockbox_on: true,
    });
    denied(
        &mut held,
        &header,
        stripe_key,
        &get,
        "the Agents switch is off",
    );
    held.access = Some(on());

    denied(&mut held, "nope", stripe_key, &get, "no such grant");
    denied(
        &mut held,
        &header,
        "rowel://other/stripe/apiKey",
        &get,
        "another lockbox",
    );
    denied(
        &mut held,
        &fill,
        "rowel://default/gh/password",
        &get,
        "a fill grant serves no header",
    );
    denied(
        &mut held,
        &header,
        stripe_key,
        &call("POST", "api.stripe.com"),
        "POST under a GET-only grant",
    );
    denied(
        &mut held,
        &header,
        stripe_key,
        &call("GET", "api.github.com"),
        "another host",
    );
    denied(
        &mut held,
        &header,
        stripe_key,
        &run(DEPLOY),
        "a command under a header grant",
    );
    denied(
        &mut held,
        &header,
        "rowel://default/other/apiKey",
        &get,
        "an item the grant does not admit",
    );
    denied(
        &mut held,
        &header,
        "rowel://default/stripe/password",
        &get,
        "a field the kind has not",
    );
    denied(
        &mut held,
        &header,
        "rowel://default/blank/apiKey",
        &get,
        "an empty value",
    );
    held.grants[0].expires_at_ms = now;
    denied(&mut held, &header, stripe_key, &get, "an expired grant");
    held.grants[0].expires_at_ms = now + 900_000;

    assert!(held.grants.iter().all(|g| g.uses == 0), "nothing counted");
    assert!(rows(&store).is_empty(), "nothing written");

    let served = resolve(&store, &cipher, &mut held, now, &header, stripe_key, &get);
    assert_eq!(
        served.as_deref().map(String::as_str),
        Ok("sk_live"),
        "and the fit is served"
    );
}

// An env grant serves one variable of the file to the one command line the
// grant names, read the way a shell would read the common cases.
#[test]
fn an_env_resolve_serves_one_variable_of_the_file_to_the_command() {
    let (_dir, store, cipher) = vault();
    seed(
        &store,
        &cipher,
        &env_file(
            "ci",
            "# keys\nAWS_KEY=first\nexport AWS_KEY='abc'\nEMPTY=\nTOKEN=\"t k\"\n",
        ),
    );
    let_in(&store, &[sidecar()]);
    let now = 1_000;
    let mut held = ready();
    held.grants.push(Grant {
        state: State::Active,
        approver: Some("user".into()),
        ..Grant::from_request(
            "default",
            secret_request(
                "side",
                &["ci"],
                vec![Target::Command("node deploy.js*".into())],
                vec![Mode::Env],
            ),
            Terms {
                approval: Approval::Ask,
                ttl_ms: 900_000,
                max_uses: None,
                reason_required: false,
                created_by: CreatedBy::User,
            },
            now,
        )
    });
    let id = held.grants[0].id.clone();

    let value = resolve(
        &store,
        &cipher,
        &mut held,
        now,
        &id,
        "rowel://default/ci/body.AWS_KEY",
        &run(DEPLOY),
    )
    .expect("served");
    assert_eq!(&*value, "abc", "the last assignment, unquoted");
    let value = resolve(
        &store,
        &cipher,
        &mut held,
        now,
        &id,
        "rowel://default/ci/body.TOKEN",
        &run(DEPLOY),
    )
    .expect("served");
    assert_eq!(&*value, "t k");
    for reference in [
        "rowel://default/ci/body.EMPTY",
        "rowel://default/ci/body.MISSING",
    ] {
        assert_eq!(
            resolve(
                &store,
                &cipher,
                &mut held,
                now,
                &id,
                reference,
                &run(DEPLOY)
            )
            .err(),
            Some(Code::GrantDenied),
            "{reference}"
        );
    }
    assert_eq!(
        resolve(
            &store,
            &cipher,
            &mut held,
            now,
            &id,
            "rowel://default/ci/body.AWS_KEY",
            &run("node -e \"console.log(process.env)\"")
        )
        .err(),
        Some(Code::GrantDenied),
        "another line"
    );
    assert_eq!(held.grants[0].uses, 2);

    let rows = store.audit_rows("dev").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].mode.as_deref(), Some("env"));
    assert_eq!(rows[0].target.as_deref(), Some(DEPLOY));
    assert_eq!(
        rows[0].detail.as_deref(),
        Some(r#"{"via":"resolve","field":"body.AWS_KEY"}"#)
    );
}

#[test]
fn the_env_parser_reads_the_common_cases_and_no_further() {
    let body = "\n# a comment\nexport A=1\nB = two words \nC=\"quoted\"\nD='single'\nE=\"mismatched'\nA=again\nF=x=y\n  export   G=spaced\nnot an assignment\nH=\"v\" # note\nI=plain # note\nJ=a#b\n";
    assert_eq!(
        env_var(body, "A").as_deref(),
        Some("again"),
        "the last wins, as a shell's would"
    );
    assert_eq!(env_var(body, "B").as_deref(), Some("two words"));
    assert_eq!(env_var(body, "C").as_deref(), Some("quoted"));
    assert_eq!(env_var(body, "D").as_deref(), Some("single"));
    assert_eq!(
        env_var(body, "E").as_deref(),
        Some("\"mismatched'"),
        "only matching quotes come off"
    );
    assert_eq!(
        env_var(body, "F").as_deref(),
        Some("x=y"),
        "split at the first `=`"
    );
    assert_eq!(env_var(body, "G").as_deref(), Some("spaced"));
    assert_eq!(
        env_var(body, "H").as_deref(),
        Some("v"),
        "a comment after quotes"
    );
    assert_eq!(
        env_var(body, "I").as_deref(),
        Some("plain"),
        "a ` #` comment"
    );
    assert_eq!(
        env_var(body, "J").as_deref(),
        Some("a#b"),
        "a `#` inside a word"
    );
    assert_eq!(env_var(body, "a"), None, "names are exact");
    assert_eq!(env_var(body, "comment"), None);
    assert_eq!(env_var(body, "not"), None);
    assert_eq!(env_var("", "A"), None);
}

// A sidecar's `env` request names the command line; the floor keeps it an
// ask under the baseline, and a rule naming the line lets it through
// without one — with `requested` and `granted` on record and no use, since
// the reply carries nothing. `node -e …` is not the line the rule names.
#[test]
fn an_env_request_asks_under_the_baseline_and_goes_auto_under_a_rule_naming_the_line() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &env_file("ci", "AWS_KEY=abc\n"));
    let_in(&store, &[sidecar()]);
    let now = 1_000;
    let mut held = ready();
    let request = |line: &str| {
        secret_request(
            "side",
            &["ci"],
            vec![Target::Command(line.into())],
            vec![Mode::Env],
        )
    };

    let asking = match decide(
        locked(&store, &mut held, now),
        "default",
        &sidecar(),
        request(DEPLOY),
        &[],
    ) {
        Ok(Prepared::Ask(asking)) => *asking,
        other => panic!("the baseline asks: {:?}", other.is_ok()),
    };
    assert_eq!(asking.grant.approval, Approval::Ask);
    assert_eq!(asking.grant.modes, [Mode::Env]);
    assert_eq!(asking.grant.targets, [Target::Command(DEPLOY.into())]);
    assert_eq!(
        asking
            .candidates
            .iter()
            .map(|c| c.item.id.as_str())
            .collect::<Vec<_>>(),
        ["ci"]
    );
    assert!(rows(&store).is_empty(), "an ask is on record once it is up");

    write_rules(
        &store,
        &[defaults::standing_for(
            "side",
            "ci",
            Target::Command("node deploy.js*".into()),
            Mode::Env,
            7,
            now,
        )],
    )
    .unwrap();
    let reply = match decide(
        locked(&store, &mut held, now),
        "default",
        &sidecar(),
        request(DEPLOY),
        &[],
    ) {
        Ok(Prepared::Answered(reply)) => reply,
        other => panic!("the rule allows: {:?}", other.is_ok()),
    };
    assert_eq!(reply.state, State::Active);
    assert_eq!(reply.uses, 0, "nothing was released");
    assert_eq!(reply.items.len(), 1);
    assert_eq!(reply.items[0].id, "ci");
    assert_eq!(held.grants.len(), 1);
    assert_eq!(held.grants[0].approver.as_deref(), Some("policy"));
    assert_eq!(held.grants[0].uses, 0);
    assert!(
        held.signals.is_empty(),
        "no extension is told of an env grant"
    );
    let events: Vec<String> = rows(&store).into_iter().map(|(event, _)| event).collect();
    assert_eq!(events, ["requested", "granted"]);

    // The same request again is served from the grant held, with no use
    // and no row.
    assert!(matches!(
        decide(
            locked(&store, &mut held, now),
            "default",
            &sidecar(),
            request(DEPLOY), &[]),
        Ok(Prepared::Answered(reply)) if reply.grant_id == held.grants[0].id
    ));
    assert_eq!(held.grants[0].uses, 0);
    assert_eq!(rows(&store).len(), 2);

    assert!(
        matches!(
            decide(
                locked(&store, &mut held, now),
                "default",
                &sidecar(),
                request("node -e \"console.log(process.env)\""),
                &[]
            ),
            Ok(Prepared::Ask(_))
        ),
        "the rule names one line"
    );
}

// An agent's header request is an ask under the baseline, with the probes
// of the request checked one method at a time; a card's number at an API
// is a step-up, as a card is anywhere.
#[test]
fn a_header_request_asks_under_the_baseline_and_a_card_number_is_a_step_up() {
    let (_dir, store, cipher) = vault();
    seed(
        &store,
        &cipher,
        &api_key("stripe", "Stripe (live)", Some("sk_live")),
    );
    seed(&store, &cipher, &card("visa"));
    let_in(&store, &[agent()]);
    let now = 1_000;
    let mut held = ready();

    let asking = match decide(
        locked(&store, &mut held, now),
        "default",
        &agent(),
        secret_request(
            "agent",
            &["stripe-live", "stripe"],
            vec![stripe(&["GET", "POST"])],
            vec![Mode::Header],
        ),
        &[],
    ) {
        Ok(Prepared::Ask(asking)) => *asking,
        other => panic!("the baseline asks: {:?}", other.is_ok()),
    };
    assert_eq!(asking.grant.approval, Approval::Ask);
    assert_eq!(asking.client_name, "Claude Code");
    assert_eq!(
        asking.grant.items,
        ItemSelector::Ids(vec!["stripe".into()]),
        "named by id or slug, kept by id, once"
    );
    assert_eq!(asking.grant.targets, [stripe(&["GET", "POST"])]);

    let asking = match decide(
        locked(&store, &mut held, now),
        "default",
        &agent(),
        secret_request("agent", &["visa"], vec![stripe(&[])], vec![Mode::Header]),
        &[],
    ) {
        Ok(Prepared::Ask(asking)) => *asking,
        other => panic!("a card asks: {:?}", other.is_ok()),
    };
    assert_eq!(asking.grant.approval, Approval::StepUp);
}

// A yes "for N days" on an env ask writes a rule for the command line in
// env mode, and the `rule_added` row's target is the line. The grant goes
// active with no use.
#[test]
fn a_yes_for_days_on_an_env_ask_writes_a_rule_for_the_command_line() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &env_file("ci", "AWS_KEY=abc\n"));
    let_in(&store, &[sidecar()]);
    let now = 1_000;
    let mut held = ready();
    let asking = match decide(
        locked(&store, &mut held, now),
        "default",
        &sidecar(),
        secret_request(
            "side",
            &["ci"],
            vec![Target::Command(DEPLOY.into())],
            vec![Mode::Env],
        ),
        &[],
    ) {
        Ok(Prepared::Ask(asking)) => *asking,
        other => panic!("the baseline asks: {:?}", other.is_ok()),
    };

    let reply = settle(
        locked(&store, &mut held, now),
        &sidecar(),
        asking,
        yes("ci"),
    )
    .expect("the yes stands");
    assert_eq!(reply.state, State::Active);
    assert_eq!(reply.uses, 0);
    assert_eq!(held.grants[0].uses, 0);

    let rules = live_rules(&store, now);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].targets, [Target::Command(DEPLOY.into())]);
    assert_eq!(rules[0].modes, [Mode::Env]);
    assert_eq!(rules[0].approval, Approval::Auto);
    assert_eq!(rules[0].items, ItemSelector::Ids(vec!["ci".into()]));

    let rows = store.audit_rows("dev").unwrap();
    let events: Vec<&str> = rows.iter().map(|r| r.event.as_str()).collect();
    assert_eq!(events, ["requested", "rule_added", "granted"]);
    assert!(rows.iter().all(|r| r.target.as_deref() == Some(DEPLOY)));
    assert!(rows.iter().all(|r| r.mode.as_deref() == Some("env")));
    assert!(rows.iter().all(|r| r.actor_kind == "sidecar"));
}

/// The ids of what the baseline would offer, in a fixed order.
fn candidate_ids(asking: &Asking) -> Vec<&str> {
    let mut ids: Vec<&str> = asking
        .candidates
        .iter()
        .map(|c| c.item.id.as_str())
        .collect();
    ids.sort_unstable();
    ids
}

fn env_request(ids: &[&str]) -> GrantRequest {
    secret_request(
        "side",
        ids,
        vec![Target::Command(DEPLOY.into())],
        vec![Mode::Env],
    )
}

// A command runs with every item it named or not at all: a standing rule for
// one of two does not win the request and narrow the grant to it — the
// baseline asks for both — while a rule naming both lets it through.
#[test]
fn a_two_item_env_request_is_asked_for_both_unless_a_rule_names_both() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &env_file("ci", "A=1\n"));
    seed(&store, &cipher, &env_file("ci2", "B=2\n"));
    let_in(&store, &[sidecar()]);
    let now = 1_000;
    let mut held = ready();
    let one = defaults::standing_for(
        "side",
        "ci",
        Target::Command("node deploy.js*".into()),
        Mode::Env,
        7,
        now,
    );
    write_rules(&store, std::slice::from_ref(&one)).unwrap();

    let asking = match decide(
        locked(&store, &mut held, now),
        "default",
        &sidecar(),
        env_request(&["ci", "ci2"]),
        &[],
    ) {
        Ok(Prepared::Ask(asking)) => *asking,
        other => panic!("the baseline asks for both: {:?}", other.is_ok()),
    };
    assert_eq!(asking.grant.approval, Approval::Ask);
    assert_eq!(candidate_ids(&asking), ["ci", "ci2"]);
    assert_eq!(
        asking.grant.items,
        ItemSelector::Ids(vec!["ci".into(), "ci2".into()]),
        "the grant admits both"
    );

    let both = rowel_broker::policy::Rule {
        items: ItemSelector::Ids(vec!["ci".into(), "ci2".into()]),
        ..one
    };
    write_rules(&store, &[both]).unwrap();
    let reply = match decide(
        locked(&store, &mut held, now),
        "default",
        &sidecar(),
        env_request(&["ci", "ci2"]),
        &[],
    ) {
        Ok(Prepared::Answered(reply)) => reply,
        other => panic!("a rule for both allows: {:?}", other.is_ok()),
    };
    assert_eq!(reply.state, State::Active);
    let mut ids: Vec<&str> = reply.items.iter().map(|i| i.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["ci", "ci2"]);
    assert_eq!(
        held.grants[0].items,
        ItemSelector::Ids(vec!["ci".into(), "ci2".into()])
    );
}

// The dialog's yes to a two-item secret ask covers both, whichever id the
// answer carried: once, the grant admits both and the reply lists both;
// for N days, the rule names both with the command line. An id that is
// none of the candidates — a stale dialog — is still a refusal.
#[test]
fn a_yes_to_a_two_item_env_ask_covers_both_and_for_days_names_both() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &env_file("ci", "A=1\n"));
    seed(&store, &cipher, &env_file("ci2", "B=2\n"));
    let_in(&store, &[sidecar()]);
    let now = 1_000;
    let mut held = ready();
    let ask = |held: &mut AgentState| match decide(
        locked(&store, held, now),
        "default",
        &sidecar(),
        env_request(&["ci", "ci2"]),
        &[],
    ) {
        Ok(Prepared::Ask(asking)) => *asking,
        other => panic!("the baseline asks: {:?}", other.is_ok()),
    };
    let both = ItemSelector::Ids(vec!["ci".into(), "ci2".into()]);

    let asking = ask(&mut held);
    assert!(matches!(
        settle(
            locked(&store, &mut held, now),
            &sidecar(),
            asking,
            Verdict::Answered(Some(GrantAnswer {
                item_id: "nope".into(),
                standing_days: None,
            })),
        ),
        Err(Code::GrantDenied)
    ));
    assert_eq!(
        rows(&store).last(),
        Some(&row("denied", r#"{"reason":"item"}"#))
    );
    assert!(held.grants.is_empty());

    let asking = ask(&mut held);
    let reply = settle(
        locked(&store, &mut held, now),
        &sidecar(),
        asking,
        Verdict::Answered(Some(GrantAnswer {
            item_id: "ci2".into(),
            standing_days: None,
        })),
    )
    .expect("the yes stands");
    let mut ids: Vec<&str> = reply.items.iter().map(|i| i.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["ci", "ci2"], "the reply lists both");
    assert_eq!(held.grants[0].items, both);
    assert_eq!(held.grants[0].uses, 0);
    assert!(live_rules(&store, now).is_empty(), "once writes no rule");

    held.grants.clear();
    let asking = ask(&mut held);
    settle(
        locked(&store, &mut held, now),
        &sidecar(),
        asking,
        yes("ci"),
    )
    .expect("the yes stands");
    assert_eq!(held.grants[0].items, both);
    let rules = live_rules(&store, now);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].items, both);
    assert_eq!(rules[0].targets, [Target::Command(DEPLOY.into())]);
    assert_eq!(rules[0].modes, [Mode::Env]);
    let rows = store.audit_rows("dev").unwrap();
    let added = rows.iter().find(|r| r.event == "rule_added").unwrap();
    assert_eq!(added.item_id, None, "several items name none on the row");
    assert_eq!(added.target.as_deref(), Some(DEPLOY));

    // And the rule answers the whole request next time, without asking.
    held.grants.clear();
    assert!(matches!(
        decide(
            locked(&store, &mut held, now),
            "default",
            &sidecar(),
            env_request(&["ci", "ci2"]), &[]),
        Ok(Prepared::Answered(reply)) if reply.state == State::Active && reply.items.len() == 2
    ));
}

// A grant held for one item answers no request that names two: the second
// request asks, and the grant held is left as it was.
#[test]
fn a_held_grant_for_one_item_is_not_reused_for_a_request_naming_two() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &env_file("ci", "A=1\n"));
    seed(&store, &cipher, &env_file("ci2", "B=2\n"));
    let_in(&store, &[sidecar()]);
    let now = 1_000;
    let mut held = ready();
    held.grants.push(Grant {
        state: State::Active,
        approver: Some("user".into()),
        ..Grant::from_request(
            "default",
            env_request(&["ci"]),
            Terms {
                approval: Approval::Ask,
                ttl_ms: 900_000,
                max_uses: None,
                reason_required: false,
                created_by: CreatedBy::User,
            },
            now,
        )
    });

    assert!(
        matches!(
            decide(
                locked(&store, &mut held, now),
                "default",
                &sidecar(),
                env_request(&["ci"]),
                &[]
            ),
            Ok(Prepared::Answered(_))
        ),
        "the one item it admits is served from it"
    );
    let asking = match decide(
        locked(&store, &mut held, now),
        "default",
        &sidecar(),
        env_request(&["ci", "ci2"]),
        &[],
    ) {
        Ok(Prepared::Ask(asking)) => *asking,
        other => panic!("both are asked for: {:?}", other.is_ok()),
    };
    assert_eq!(candidate_ids(&asking), ["ci", "ci2"]);
    assert_eq!(held.grants.len(), 1);
    assert_eq!(held.grants[0].uses, 0);
    assert!(rows(&store).is_empty());
}

// A secret request naming something that is not there — or that has no
// field to serve — is refused whole, not answered with the rest.
#[test]
fn a_secret_request_naming_an_item_that_is_not_there_is_denied_no_candidates() {
    let (_dir, store, cipher) = vault();
    seed(&store, &cipher, &env_file("ci", "A=1\n"));
    seed(
        &store,
        &cipher,
        &Entry {
            id: "memo".into(),
            kind: "note".into(),
            title: "Memo".into(),
            ..Entry::default()
        },
    );
    let_in(&store, &[sidecar()]);
    let now = 1_000;
    let mut held = ready();

    assert!(
        matches!(
            decide(
                locked(&store, &mut held, now),
                "default",
                &sidecar(),
                env_request(&["ci", "ci"]),
                &[]
            ),
            Ok(Prepared::Ask(_))
        ),
        "a name that is there, twice, is still there"
    );
    assert!(rows(&store).is_empty());

    // Each refusal counts as a denial on the client's record, which is what
    // slows a client down after three in a row.
    for ids in [&["ci", "nope"][..], &["ci", "memo"], &["ci", "ci-2"]] {
        assert!(
            matches!(
                decide(
                    locked(&store, &mut held, now),
                    "default",
                    &sidecar(),
                    env_request(ids),
                    &[]
                ),
                Err(Code::GrantDenied)
            ),
            "{ids:?}"
        );
    }
    assert!(held.grants.is_empty());
    assert_eq!(
        rows(&store),
        vec![row("denied", r#"{"reason":"no_candidates"}"#); 3]
    );
    assert!(
        matches!(
            decide(
                locked(&store, &mut held, now),
                "default",
                &sidecar(),
                env_request(&["ci"]),
                &[]
            ),
            Err(Code::RateLimited)
        ),
        "three in a row, and the client is slowed down"
    );
}

// What the dialog is told about where: a site for a page (never the page's
// path), the request pattern and the command line as written, each tagged
// by its kind so the frontend can tell them apart.
#[test]
fn the_ask_names_its_targets_by_kind() {
    let targets: Vec<crate::events::AskTarget> = [
        Target::Origin("https://github.com/login?next=/x".into()),
        stripe(&["GET"]),
        Target::Command(DEPLOY.into()),
    ]
    .iter()
    .map(super::ask_target)
    .collect();
    assert_eq!(
        serde_json::to_value(&targets).unwrap(),
        json!([
            { "kind": "origin", "origin": "https://github.com" },
            { "kind": "http", "host": "api.stripe.com", "path": "/v1/*", "methods": ["GET"] },
            { "kind": "command", "line": DEPLOY },
        ])
    );
}

// A request whose targets are not the kind of place its modes are
// performed at has no one shape to be, and is refused on record.
#[test]
fn a_request_whose_targets_do_not_fit_its_modes_is_denied_no_target() {
    let (_dir, store, cipher) = vault();
    seed(
        &store,
        &cipher,
        &api_key("stripe", "Stripe", Some("sk_live")),
    );
    seed(&store, &cipher, &github_login("gh"));
    let_in(&store, &[agent()]);
    let now = 1_000;
    let mut held = ready();
    let origin = Target::Origin("https://github.com".into());
    let misfits = [
        secret_request(
            "agent",
            &["stripe"],
            vec![origin.clone()],
            vec![Mode::Header],
        ),
        secret_request("agent", &["gh"], vec![stripe(&[])], vec![Mode::Fill]),
        secret_request("agent", &["stripe"], vec![stripe(&[])], vec![Mode::Token]),
        secret_request(
            "agent",
            &["gh", "stripe"],
            vec![origin, stripe(&[])],
            vec![Mode::Fill, Mode::Header],
        ),
        secret_request("agent", &["stripe"], vec![], vec![Mode::Header]),
    ];
    for request in misfits {
        assert!(matches!(
            decide(
                locked(&store, &mut held, now),
                "default",
                &agent(),
                request,
                &[]
            ),
            Err(Code::GrantDenied)
        ));
    }
    assert!(held.grants.is_empty());
    assert_eq!(
        rows(&store),
        vec![row("denied", r#"{"reason":"no_target"}"#); 5]
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
