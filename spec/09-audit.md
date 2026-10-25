# 09 — Audit

## 1. What the log is for

The question a person asks after the fact is "what did my agents do with my
secrets?", and the question a company's auditor asks is "who approved it, and
can you prove this log is complete?". Both are answered by one log that every
tier writes the same way: one row per decision and per use, with the agent,
the run, the principal, the item, the target, the mode, the approval, the
approver and the outcome, chained so that a removed or altered row is
detectable.

In Tier 1 it lives inside the lockbox, syncs with it, and shows up in Vault
Health. In Tiers 2 and 3 it also lands in the server's database and streams out
to the company's tools. A lockbox restored on a new device carries its own
history with it.

## 2. What a row says

```
  2026-10-24 09:14:03  agent "Claude Code" (run 7f3a…)  for alex
    requested  github.com login "alex@…"   mode fill   policy: ask
    approved by alex (Touch ID) at 09:14:09
  2026-10-24 09:14:10  filled github.com, submitted, no exposure
  2026-10-24 09:29:10  grant ended (expired), signed out of github.com
```

---

### Technical details

**Schema (✅ I1, `rowel_core::store`, migration adding `audit`).**

```sql
CREATE TABLE audit (
  id          TEXT NOT NULL,         -- 16 random bytes hex (not a key: see Sync)
  writer_id   TEXT NOT NULL,         -- the device (workspace install id) or server that wrote it
  seq         INTEGER NOT NULL,      -- this writer's running counter
  ts_ms       INTEGER NOT NULL,
  prev_hash   BLOB NOT NULL,         -- 32 bytes; this writer's previous row's hash, zeros first
  hash        BLOB NOT NULL,         -- sha256(prev_hash || canonical row json without hash)
  actor_kind  TEXT NOT NULL,         -- human | extension | agent | sidecar | policy | server
  actor_id    TEXT,                  -- client key / agent id / member id
  actor_name  TEXT,
  run_id      TEXT,
  acting_for  TEXT,
  grant_id    TEXT,
  item_id     TEXT,
  target      TEXT,                  -- origin / host+path / command
  mode        TEXT,                  -- fill | passkey | header | token | env
  approval    TEXT,                  -- auto | ask | step_up
  approver    TEXT,
  event       TEXT NOT NULL,         -- requested | granted | denied | used | exposed |
                                     -- ended | revoked | associated | revoked_client |
                                     -- rule_added | rule_removed | key_unwrapped |
                                     -- rate_limited | pruned_through
  detail      TEXT                   -- small JSON, no secrets
);
CREATE INDEX audit_ts ON audit(ts_ms);
CREATE INDEX audit_chain ON audit(writer_id, seq);
```

Rows are **insert-only**. Each writer keeps its own chain: `prev_hash` is the
hash of the same writer's previous row (found by `writer_id, seq - 1`), so
devices and the server never break each other's chains, and no `meta` head is
needed. A writer never writes two rows at one `seq` (`audit_insert` refuses a
taken one, a loud failure for two writers racing on one chain); the table
does not forbid it, because a fork arriving through sync has to be kept (see
Sync). Verification walks each writer's chain in `seq` order and checks it has
no gaps below the newest row and no `seq` held twice (`Reason::Fork`).

**Sync (✅ I4, engine + store).** A pack carries the whole SQLCipher snapshot,
so audit rows always rode in it; what I4 added is reading them out, merging
them and counting them. Audit rows travel and merge as a **union of rows**,
never last-writer-wins and never tombstoned:

- `Snapshot.audit: Vec<AuditRecord>`, filled by `records_from_snapshot` from
  `SqliteStore::audit_all()` (every row, `ORDER BY writer_id, seq`);
  `SqliteStore::merge(recs, atts, audit)` inserts them in the same
  transaction as the entries and attachments. A row is its **whole contents**
  — `store::hash::audit_fingerprint`, every column (the stored `prev_hash`
  and `hash` among them) in the fixed injective encoding, computed from the
  columns wherever it is needed and never stored, since a row edited in place
  would keep a stale stored one. The merge inserts each incoming row whose
  fingerprint the vault does not hold, so the same row seen twice lands once
  and anything else is kept beside what the device holds, never instead of
  it: a second row at a writer's `seq` (a forked chain, as two installs
  sharing an install id write) or a copy of a known row altered behind its
  unchanged `id` and `hash` (how tampering looks). Every device then holds
  the same rows to the byte, the digests agree only then and the runs
  settle, and `verify` names the fork (`Reason::Fork`) or the altered copy
  (`Reason::Hash`, each row's own hash being checked before its relations)
  at the same `seq` everywhere — on a fresh restore from the account too.
  Among a fork's sides the readers order by `(id, hash)`, which is all
  `audit_last` and the prune's cut read. (Review of PR #658: with `UNIQUE
  (writer_id, seq)` each device kept its own side of a fork and pushed it on
  every run; keyed by `(id, hash)`, a copy altered behind its hash was taken
  for the original and the digests agreed over different rows.) The merge's returned count (`SyncOutcome.merged`, "the entry
  list changed") leaves audit rows out.
- `store::hash::vault_digest(recs, atts, audit)` includes the audit rows as
  their fingerprints, sorted — never the stored chain hash alone, which an
  altered row keeps — following the attachments pattern: a vault without audit rows
  digests exactly as before, and a device whose only change is new audit rows
  pushes (`engine::sync` pushes on a digest difference).
- A restore from a pack on a fresh install already carries the whole file,
  audit included.

**Retention (✅ I4).** `AUDIT_RETENTION_MS` = 365 days beside
`TOMBSTONE_TTL_MS`; the engine passes both cutoffs to `LocalVault::pack` as a
`Reclaim { tombstones_before, audit_before, at_ms }`, so the prune runs where
the tombstone purge runs: only on a push (a digest-equal run returns before
`pack`), and inside `SessionVault`'s key-checked reclaim step. Two things
differ from the first draft, both forced by the union merge:

- **Every device reclaims every writer's aged rows**, not only its own. With a
  union merge a device that pruned only its own chain would be handed the rows
  back by any peer that still held them, and an iOS device — which writes no
  rows and so had nothing of its own to prune — would hold them forever and
  never agree with the desktop's digest. `SqliteStore::audit_prune(before)`
  reclaims every chain's prefix in one transaction and reports each writer
  with the highest `seq` it lost; the set converges once each peer has pushed.
- **A prefix, not the old rows alone.** The cut is the chain's leading run of
  rows stamped before the cutoff, up to the first younger one: never a hole
  that verifies as a gap forever, and never a young row. (Review of PR #658:
  cutting through the *newest* aged row let one row with an old stamp — a
  reset clock, or a row forged into a synced pack — take the whole recent
  chain below it on every device, still verifying as pruned. A row stamped
  out of order now waits until the rows before it have aged too.)

Only the chain's own writer notes the cut: `rowel_broker::audit::prune` (desktop,
where the broker is linked) writes a `pruned_through` row with actor `policy`
and detail `{"seq":<highest seq reclaimed>,"beforeMs":<cutoff>}` when its own
rows were among those reclaimed; iOS (`writer_id: None`) reclaims and writes
nothing. `verify` treats a chain that starts past `seq` 1 as pruned and checks
it from where it begins, so another writer's chain losing its prefix with no
note stays verifiable. **A chain reclaimed whole does not start over**: the
store reports each cut as `AuditCut { writer_id, seq, hash }` (the newest row
that went), and the note chains from that `(seq, hash)` as if the row had
stayed — a note that restarted at `seq` 1 would collide with the old rows a
peer still holds, be dropped by the union merge, and leave the two digests
never agreeing; chained on, a peer reads its old copy and the note as one
unbroken chain. Tiers 2/3 keep the server copy per tenant policy and export
before pruning.

**Writers.** `rowel_broker::audit::Writer` with `fn record(&self, Row)`; the
`Host` implementations call it at every state change in `03`'s lifecycle.
`writer_id` is the per-install id in Tier 1 (`storage::install_id`: minted
once into an owner-only file, cached for the process; never the workspace id,
which is "default" on every install and would collide after a sync or
restore) and the server's node id in Tier 2.
The `Mock` host collects rows so protocol tests can assert on them.

**Details as built (I1).** `detail` is a small JSON object: `denied` carries
`{"reason": …}` with `no_target`, `lockbox`, `no_rule`, `no_candidates`,
`item`, `user` or `timeout`; `used` carries nothing for the fill that followed
an ask, `{"reuse":true}` for a grant served again before policy ran, and
`{"via":"grantId"}` for the `grantId` paths; `rule_added` carries
`{"ruleId","days"}`; `revoked` carries `{"reason":"user"|"switch"|"lockbox"}`;
`ended` `{"reason":"expired"}`; `exposed` `{"reason":"exposure"}`. The
`target` column is the page reduced to scheme, host and path.

**Details as built (I2).** `actor_kind` follows the client's kind
(`extension`, `agent`, `sidecar`), and `run_id` is the agent's `runId`.
`associated` is written when any client is let in, `revoked_client` when one
is forgotten in Settings; `revoked` gains `{"reason":"agent"}` for an
`end-grant`; a passkey ceremony writes `used` with `mode: passkey` and the
page's origin as target (a refused one `denied {"reason":"user"}`); an
agent's fill for a site without a login is `denied {"reason":"no_candidates"}`.

**Details as built (I3).** A secret grant (`header`, `env`) writes
`requested` and `granted` at activation and no `used`; each `resolve` writes
`used` with the *sidecar* as actor, the grant's `run_id`, `mode` from the
target's kind, `item_id` the resolved id, `target` as `host/path` (query
stripped) or the command line clamped to 300 characters, and detail
`{"via":"resolve","field":"<field as the placeholder writes it>"}`. A
request whose targets do not fit its modes is `denied {"reason":"no_target"}`.
A `rule_added` for an env rule carries the command line as its target.

**Surfaces.**

- Tier 1 (✅ I4): `src/components/Main/Sidebar/Settings/Sections/Audit/AgentActivity/`
  (Vault Health, desktop only) shows the last 7 days of `used`/`denied`/`exposed`
  events (newest first, 20 on the first screen, at most 200; the store picks
  the events in the query before the limit — `audit_recent(since, events,
  limit)` — so a busy week's bookkeeping rows cannot crowd them out), standing rules
  older than 30 days with *Remove*, clients not seen in 90 days with *Forget*,
  and *Verify the log*, which walks every writer's chain and names the first
  break in words ("device <first 8 hex of the writer id>" for a peer, "this
  device" for the install). The panel is rendered only when the Agents switch
  is on or there is something to show, so a person who never uses agents sees
  nothing about them. Commands (`src-tauri/src/commands/activity.rs`):
  `audit_agent_activity` → `{ events, staleRules, idleClients }` where an event
  carries `thisDevice` (the writer is this install), the item's current title
  (null when gone), the `target`, `mode`, `approval` and the `reason` parsed
  out of a `denied`/`exposed` row's detail; `audit_verify` → `{ ok, writers:
  [{ writerId, thisDevice, rows, firstSeq, lastSeq, break: { seq, reason }
  | null }] }`. A client never seen since it was let in is idle from its
  `created_at_ms`.
- Tier 2/3: `GET /v1/admin/audit/export` (JSONL, filters), webhook and OTLP
  (`08`). Each exported row carries `writer_id`, `seq`, `prev_hash`, `hash`.

**Never in the log.** Secret values, TOTP codes, passkey private material,
full URLs with query strings (the target is scheme+host+path), or reasons
longer than 300 characters. The redactor (`05`) running over `detail` before
insert was proposed as a belt-and-braces check and is not built in Tier 1:
every `detail` the app writes is spelled from a fixed vocabulary (a reason
word, a rule id, a field name, a seq), so there is no free text to scan. It
stays on the list for the server, where `detail` may carry a tenant's own
reasons.
