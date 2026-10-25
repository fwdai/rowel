//! The agent half of Vault Health (`spec/09`, Tier 1): what agents did with
//! the lockbox's secrets this week, the standing rules and clients that have
//! gone stale, and — on demand — whether every writer's chain of rows still
//! holds. Reads only: the rows are the broker's to write (`agents::record`).
//!
//! What a panel shows is read from the open lockbox under the session lock
//! and shaped outside it. The shaping and the walk are free functions over
//! what the store handed back, so they are tested without an app around them.

use std::collections::HashMap;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use rowel_broker::audit::{self, Reason};

use crate::agents::{self, RuleRow};
use crate::browser::{self, Client};
use crate::error::{store_err, Result};
use crate::state::AppState;
use crate::storage;
use crate::store::{AuditRecord, Result as StoreResult, SqliteStore};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// How far back the activity list reaches.
pub const EVENT_WINDOW_MS: i64 = 7 * DAY_MS;
/// The most rows the list shows.
pub const EVENT_CAP: usize = 200;
/// A standing rule this old is worth a second look.
pub const STALE_RULE_MS: i64 = 30 * DAY_MS;
/// A client unseen this long is probably gone.
pub const IDLE_CLIENT_MS: i64 = 90 * DAY_MS;

/// The events the list is about: a secret put to use, refused, or exposed.
/// The bookkeeping around them (`requested`, `granted`, `ended`, …) is on the
/// log for the auditor, not for the panel.
const SHOWN_EVENTS: [&str; 3] = ["used", "denied", "exposed"];

/// What the panel shows: the week's rows, and what has gone stale.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AgentActivity {
    pub events: Vec<AgentEvent>,
    pub stale_rules: Vec<RuleRow>,
    pub idle_clients: Vec<Client>,
}

/// One row as the panel shows it. `event`, `actor_kind`, `mode` and
/// `approval` are the columns' words, which are the wire's
/// (`Event::as_str`, `Mode::as_str`, …): nothing is re-spelled on the way.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvent {
    pub id: String,
    pub ts_ms: i64,
    pub writer_id: String,
    /// Whether this install wrote the row.
    pub this_device: bool,
    pub event: String,
    pub actor_kind: String,
    pub actor_name: Option<String>,
    pub run_id: Option<String>,
    pub grant_id: Option<String>,
    pub item_id: Option<String>,
    /// The item's title in the open lockbox; `None` once the item is gone.
    pub item_title: Option<String>,
    pub target: Option<String>,
    pub mode: Option<String>,
    pub approval: Option<String>,
    /// `detail.reason` of a `denied` or `exposed` row; `None` for a `used`
    /// one, whose detail says how it was served, not why.
    pub reason: Option<String>,
}

/// Every writer's chain walked in `seq` order, and the first row of each
/// that does not hold.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuditVerification {
    pub ok: bool,
    pub writers: Vec<WriterChain>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WriterChain {
    pub writer_id: String,
    pub this_device: bool,
    pub rows: u32,
    pub first_seq: i64,
    pub last_seq: i64,
    /// `break` on the wire, a word Rust keeps for itself.
    #[serde(rename = "break")]
    pub first_break: Option<ChainBreak>,
}

/// [`audit::Break`] as the wire has it: the broker's `Reason` has no serde
/// form, and the panel's words for it are camelCase.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainBreak {
    pub seq: i64,
    pub reason: BreakReason,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BreakReason {
    Writer,
    Fork,
    Gap,
    PrevHash,
    Hash,
}

impl From<audit::Break> for ChainBreak {
    fn from(b: audit::Break) -> Self {
        let reason = match b.reason {
            Reason::Writer => BreakReason::Writer,
            Reason::Fork => BreakReason::Fork,
            Reason::Gap => BreakReason::Gap,
            Reason::PrevHash => BreakReason::PrevHash,
            Reason::Hash => BreakReason::Hash,
        };
        ChainBreak { seq: b.seq, reason }
    }
}

/// The agent half of Vault Health for the open lockbox. Rejects while locked,
/// as `agent_rules` does. The rules come from `agents::list_rules`, which
/// reads them under its own section — it also drops the expired ones as it
/// goes, which is its business — and the rows, titles and clients from one
/// more; the only thing that can come between the two is a seal, and the
/// second refuses then.
#[tauri::command]
pub fn audit_agent_activity(app: AppHandle) -> Result<AgentActivity> {
    let state = app.state::<AppState>();
    let now = agents::now_ms();
    let this_device = storage::install_id(&app);
    let rules = agents::list_rules(&app)?;
    let (rows, titles, clients) = {
        let session = state.session.lock().unwrap();
        let store = session.store()?;
        let rows = recent_rows(store, now).map_err(store_err)?;
        (rows, agents::titles_in(store), browser::clients_in(store))
    };
    Ok(AgentActivity {
        events: recent_events(rows, &titles, &this_device, now),
        stale_rules: rules.into_iter().filter(|r| is_stale(r, now)).collect(),
        idle_clients: clients.into_iter().filter(|c| is_idle(c, now)).collect(),
    })
}

/// Walk every writer's chain in the open lockbox's log. Rejects while locked.
/// The read is one query under the lock; the hashing — a year of rows is a
/// year of hashes — runs on the blocking pool, off the IPC thread.
#[tauri::command]
pub async fn audit_verify(app: AppHandle) -> Result<AuditVerification> {
    let this_device = storage::install_id(&app);
    let rows = {
        let state = app.state::<AppState>();
        let session = state.session.lock().unwrap();
        session.store()?.audit_all().map_err(store_err)?
    };
    super::blocking(move || Ok(verify_chains(&rows, &this_device))).await
}

/// The week before `now_ms` of the events the list shows, newest first, at
/// most [`EVENT_CAP`]: chosen by event in the query, before the cap, so a busy
/// week's bookkeeping rows cannot crowd the uses out of it.
fn recent_rows(store: &SqliteStore, now_ms: i64) -> StoreResult<Vec<AuditRecord>> {
    store.audit_recent(now_ms - EVENT_WINDOW_MS, &SHOWN_EVENTS, EVENT_CAP as u32)
}

/// The rows the list shows, shaped: those stamped within the week before
/// `now_ms` whose event is one of [`SHOWN_EVENTS`], newest first, at most
/// [`EVENT_CAP`] of them. The store hands the week back newest first already;
/// sorting here keeps the function true to its word whatever it is handed,
/// and a stable sort keeps the store's order among rows of one moment.
pub(crate) fn recent_events(
    mut rows: Vec<AuditRecord>,
    titles: &HashMap<String, String>,
    this_device: &str,
    now_ms: i64,
) -> Vec<AgentEvent> {
    let since = now_ms - EVENT_WINDOW_MS;
    rows.retain(|r| r.ts_ms >= since && SHOWN_EVENTS.contains(&r.event.as_str()));
    rows.sort_by_key(|r| std::cmp::Reverse(r.ts_ms));
    rows.truncate(EVENT_CAP);
    rows.into_iter()
        .map(|row| event_of(row, titles, this_device))
        .collect()
}

fn event_of(row: AuditRecord, titles: &HashMap<String, String>, this_device: &str) -> AgentEvent {
    let reason = match row.event.as_str() {
        "denied" | "exposed" => reason_in(row.detail.as_deref()),
        _ => None,
    };
    AgentEvent {
        this_device: row.writer_id == this_device,
        item_title: row.item_id.as_ref().and_then(|id| titles.get(id).cloned()),
        reason,
        id: row.id,
        ts_ms: row.ts_ms,
        writer_id: row.writer_id,
        event: row.event,
        actor_kind: row.actor_kind,
        actor_name: row.actor_name,
        run_id: row.run_id,
        grant_id: row.grant_id,
        item_id: row.item_id,
        target: row.target,
        mode: row.mode,
        approval: row.approval,
    }
}

/// `detail.reason`, when `detail` is the small JSON object a `denied` or
/// `exposed` row carries. A detail that is anything else says nothing: the
/// row is still shown, just without a why.
fn reason_in(detail: Option<&str>) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(detail?).ok()?;
    value.get("reason")?.as_str().map(str::to_string)
}

/// A standing rule written [`STALE_RULE_MS`] or longer ago. Whether it is
/// still in force is the caller's to know: `agents::list_rules` lists no
/// other.
pub(crate) fn is_stale(rule: &RuleRow, now_ms: i64) -> bool {
    rule.created_at_ms <= now_ms - STALE_RULE_MS
}

/// A client not seen for [`IDLE_CLIENT_MS`] or longer; one never seen since
/// it was let in counts from then. One with neither time — let in before
/// clients carried one (`created_at_ms` 0) and not seen since — has no age to
/// judge and is not listed, rather than shown as let in on 1 January 1970.
pub(crate) fn is_idle(client: &Client, now_ms: i64) -> bool {
    let known = client
        .last_seen_at_ms
        .or((client.created_at_ms > 0).then_some(client.created_at_ms));
    known.is_some_and(|at| at <= now_ms - IDLE_CLIENT_MS)
}

/// Every writer's chain in `rows` — which arrive as `audit_all` hands them
/// back, writer by writer in `seq` order — through [`audit::verify`], and
/// whether all of them held.
pub(crate) fn verify_chains(rows: &[AuditRecord], this_device: &str) -> AuditVerification {
    let writers: Vec<WriterChain> = rows
        .chunk_by(|a, b| a.writer_id == b.writer_id)
        .map(|chain| WriterChain {
            writer_id: chain[0].writer_id.clone(),
            this_device: chain[0].writer_id == this_device,
            rows: chain.len() as u32,
            first_seq: chain[0].seq,
            last_seq: chain[chain.len() - 1].seq,
            first_break: audit::verify(chain).err().map(ChainBreak::from),
        })
        .collect();
    AuditVerification {
        ok: writers.iter().all(|w| w.first_break.is_none()),
        writers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rowel_broker::actions::ClientKind;
    use rowel_broker::audit::{ActorKind, Draft, Event, Writer};
    use rowel_broker::grant::{Approval, Mode};

    const NOW: i64 = 1_000 * DAY_MS;
    const THIS: &str = "device-a";
    const OTHER: &str = "device-b";

    fn draft(event: Event, item: &str, detail: Option<&str>) -> Draft {
        Draft {
            actor_name: Some("Claude Code".into()),
            run_id: Some("run-7".into()),
            grant_id: Some("g1".into()),
            item_id: Some(item.into()),
            target: Some("https://github.com/login".into()),
            mode: Some(Mode::Fill),
            approval: Some(Approval::Ask),
            detail: detail.map(str::to_string),
            ..Draft::new(ActorKind::Agent, event)
        }
    }

    fn write(store: &SqliteStore, writer: &str, event: Event, ts: i64, detail: Option<&str>) {
        Writer::new(writer, store)
            .record(draft(event, "gh", detail), ts)
            .unwrap();
    }

    fn week(store: &SqliteStore) -> Vec<AuditRecord> {
        recent_rows(store, NOW).unwrap()
    }

    fn titles() -> HashMap<String, String> {
        HashMap::from([("gh".to_string(), "GitHub".to_string())])
    }

    // The list is the week's uses, denials and exposures and nothing else —
    // not the bookkeeping around them, not what is older than a week, with
    // the day's edge inside the window — newest first.
    #[test]
    fn the_list_keeps_the_weeks_uses_denials_and_exposures_newest_first() {
        let (_dir, store, _cipher) = rowel_broker::testing::vault();
        write(&store, THIS, Event::Used, NOW - 1_000, None);
        write(&store, THIS, Event::Requested, NOW - 500, None);
        write(&store, THIS, Event::Granted, NOW - 400, None);
        write(&store, THIS, Event::Used, NOW - 8 * DAY_MS, None);
        write(
            &store,
            THIS,
            Event::Exposed,
            NOW - EVENT_WINDOW_MS,
            Some(r#"{"reason":"exposure"}"#),
        );
        write(
            &store,
            THIS,
            Event::Denied,
            NOW - 3_000,
            Some(r#"{"reason":"no_rule"}"#),
        );
        write(
            &store,
            THIS,
            Event::Ended,
            NOW - 2_000,
            Some(r#"{"reason":"expired"}"#),
        );

        let events = recent_events(week(&store), &titles(), THIS, NOW);
        assert_eq!(
            events
                .iter()
                .map(|e| (e.event.as_str(), e.ts_ms))
                .collect::<Vec<_>>(),
            [
                ("used", NOW - 1_000),
                ("denied", NOW - 3_000),
                ("exposed", NOW - EVENT_WINDOW_MS),
            ]
        );

        // Handed the rows in any order, the list is still newest first.
        let mut rows = week(&store);
        rows.reverse();
        let reversed = recent_events(rows, &titles(), THIS, NOW);
        assert_eq!(reversed, events);
    }

    // A row carries the title the lockbox has for its item — none once the
    // item is gone — says whether this install wrote it, and names the reason
    // a denial or an exposure gives; a use has no reason, whatever its detail
    // says, and a detail that is not the JSON it should be gives none.
    #[test]
    fn a_row_is_shaped_with_its_title_its_device_and_the_reason_it_carries() {
        let (_dir, store, _cipher) = rowel_broker::testing::vault();
        write(
            &store,
            THIS,
            Event::Used,
            NOW - 100,
            Some(r#"{"reuse":true}"#),
        );
        write(
            &store,
            OTHER,
            Event::Denied,
            NOW - 200,
            Some(r#"{"reason":"no_candidates"}"#),
        );
        write(&store, OTHER, Event::Denied, NOW - 300, Some("not json"));
        Writer::new(THIS, &store)
            .record(
                draft(Event::Exposed, "gone", Some(r#"{"reason":"exposure"}"#)),
                NOW - 400,
            )
            .unwrap();

        let events = recent_events(week(&store), &titles(), THIS, NOW);
        assert_eq!(events.len(), 4);

        let used = &events[0];
        assert_eq!(used.event, "used");
        assert!(used.this_device);
        assert_eq!(used.writer_id, THIS);
        assert_eq!(used.item_id.as_deref(), Some("gh"));
        assert_eq!(used.item_title.as_deref(), Some("GitHub"));
        assert_eq!(used.reason, None, "a use has no reason to give");
        assert_eq!(used.actor_kind, "agent");
        assert_eq!(used.actor_name.as_deref(), Some("Claude Code"));
        assert_eq!(used.run_id.as_deref(), Some("run-7"));
        assert_eq!(used.grant_id.as_deref(), Some("g1"));
        assert_eq!(used.target.as_deref(), Some("https://github.com/login"));
        assert_eq!(used.mode.as_deref(), Some("fill"));
        assert_eq!(used.approval.as_deref(), Some("ask"));

        let denied = &events[1];
        assert!(!denied.this_device);
        assert_eq!(denied.writer_id, OTHER);
        assert_eq!(denied.reason.as_deref(), Some("no_candidates"));

        assert_eq!(events[2].reason, None, "a malformed detail says nothing");

        let exposed = &events[3];
        assert_eq!(exposed.reason.as_deref(), Some("exposure"));
        assert_eq!(exposed.item_id.as_deref(), Some("gone"));
        assert_eq!(exposed.item_title, None, "the item is gone");

        // The wire has the names the webview reads.
        let json = serde_json::to_value(used).unwrap();
        assert_eq!(json["thisDevice"], true);
        assert_eq!(json["itemTitle"], "GitHub");
        assert_eq!(json["tsMs"], NOW - 100);
        assert!(json["reason"].is_null());
    }

    #[test]
    fn the_list_stops_at_two_hundred_rows() {
        let (_dir, store, _cipher) = rowel_broker::testing::vault();
        write(&store, THIS, Event::Used, NOW, None);
        let one = week(&store).remove(0);
        let rows: Vec<AuditRecord> = (0..EVENT_CAP as i64 + 5)
            .map(|i| AuditRecord {
                id: format!("row-{i}"),
                seq: i + 1,
                ts_ms: NOW - i,
                ..one.clone()
            })
            .collect();

        let events = recent_events(rows, &titles(), THIS, NOW);
        assert_eq!(events.len(), EVENT_CAP);
        assert_eq!(events[0].ts_ms, NOW, "the newest are kept");
        assert_eq!(events[EVENT_CAP - 1].ts_ms, NOW - (EVENT_CAP as i64 - 1));
    }

    // A busy week's bookkeeping is newer and more plentiful than its uses, and
    // must not crowd them out of the list: the query picks the events first.
    #[test]
    fn bookkeeping_rows_do_not_crowd_the_uses_out_of_the_week() {
        let (_dir, store, _cipher) = rowel_broker::testing::vault();
        write(&store, THIS, Event::Used, NOW - DAY_MS, None);
        for i in 0..EVENT_CAP as i64 + 50 {
            write(&store, THIS, Event::Granted, NOW - i, None);
        }
        let rows = week(&store);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].event, "used");
        assert_eq!(recent_events(rows, &titles(), THIS, NOW).len(), 1);
    }

    fn rule(created_at_ms: i64) -> RuleRow {
        RuleRow {
            id: "r".into(),
            client_key: "agent".into(),
            client_name: "Claude Code".into(),
            item_id: None,
            item_title: None,
            origin: None,
            target: None,
            mode: Mode::Fill,
            expires_at_ms: None,
            created_at_ms,
        }
    }

    fn client(created_at_ms: i64, last_seen_at_ms: Option<i64>) -> Client {
        Client {
            name: "Claude Code".into(),
            key: "agent".into(),
            kind: ClientKind::Agent,
            platform: None,
            created_at_ms,
            last_seen_at_ms,
            agent_id: None,
        }
    }

    // Thirty days and ninety days are inclusive edges: at the day, stale and
    // idle; a millisecond short of it, neither. A client never seen counts
    // from when it was let in.
    #[test]
    fn a_rule_is_stale_at_thirty_days_and_a_client_idle_at_ninety() {
        assert!(is_stale(&rule(NOW - STALE_RULE_MS), NOW));
        assert!(!is_stale(&rule(NOW - STALE_RULE_MS + 1), NOW));
        assert!(is_stale(&rule(NOW - 400 * DAY_MS), NOW));

        assert!(is_idle(&client(NOW - IDLE_CLIENT_MS, None), NOW));
        assert!(!is_idle(&client(NOW - IDLE_CLIENT_MS + 1, None), NOW));
        assert!(is_idle(&client(0, Some(NOW - IDLE_CLIENT_MS)), NOW));
        assert!(
            !is_idle(&client(0, Some(NOW - IDLE_CLIENT_MS + 1)), NOW),
            "seen since it was let in"
        );
        assert!(
            !is_idle(&client(0, None), NOW),
            "let in before clients carried a time, and not seen since: no age"
        );
    }

    // Two writers, two chains: a row altered in one breaks that chain at
    // that row and leaves the other whole — and the whole log is not ok.
    #[test]
    fn verification_names_the_altered_row_and_its_writer_alone() {
        let (_dir, store, _cipher) = rowel_broker::testing::vault();
        for ts in [1_000, 2_000, 3_000] {
            write(&store, THIS, Event::Used, ts, None);
        }
        for ts in [1_500, 2_500] {
            write(&store, OTHER, Event::Used, ts, None);
        }

        let rows = store.audit_all().unwrap();
        let sound = verify_chains(&rows, THIS);
        assert!(sound.ok);
        assert_eq!(sound.writers.len(), 2);
        assert_eq!(sound.writers[0].writer_id, THIS);
        assert!(sound.writers[0].this_device);
        assert_eq!(
            (
                sound.writers[0].rows,
                sound.writers[0].first_seq,
                sound.writers[0].last_seq
            ),
            (3, 1, 3)
        );
        assert_eq!(sound.writers[1].writer_id, OTHER);
        assert!(!sound.writers[1].this_device);
        assert_eq!(
            (
                sound.writers[1].rows,
                sound.writers[1].first_seq,
                sound.writers[1].last_seq
            ),
            (2, 1, 2)
        );

        let mut altered = rows.clone();
        let at = altered
            .iter()
            .position(|r| r.writer_id == OTHER && r.seq == 2)
            .unwrap();
        altered[at].target = Some("https://elsewhere.example".into());
        let broken = verify_chains(&altered, THIS);
        assert!(!broken.ok);
        assert_eq!(broken.writers[0].first_break, None, "the other chain holds");
        assert_eq!(
            broken.writers[1].first_break,
            Some(ChainBreak {
                seq: 2,
                reason: BreakReason::Hash
            })
        );

        // A missing row is a gap at the number that is missing; the wire
        // names the field `break` and spells the reason as the webview does.
        let mut gapped = rows.clone();
        gapped.remove(1);
        let gapped = verify_chains(&gapped, THIS);
        assert_eq!(
            gapped.writers[0].first_break,
            Some(ChainBreak {
                seq: 2,
                reason: BreakReason::Gap
            })
        );
        let json = serde_json::to_value(&gapped.writers[0]).unwrap();
        assert_eq!(json["break"]["seq"], 2);
        assert_eq!(json["break"]["reason"], "gap");
        assert_eq!(
            serde_json::to_value(BreakReason::PrevHash).unwrap(),
            "prevHash"
        );
    }

    #[test]
    fn an_empty_log_verifies_with_no_writers() {
        let (_dir, store, _cipher) = rowel_broker::testing::vault();
        let verification = verify_chains(&store.audit_all().unwrap(), THIS);
        assert!(verification.ok);
        assert!(verification.writers.is_empty());
    }
}
