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

**Schema (❌, `rowel_core::store`, migration adding `audit`).**

```sql
CREATE TABLE audit (
  id          TEXT PRIMARY KEY,      -- 16 random bytes hex
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
  detail      TEXT,                  -- small JSON, no secrets
  UNIQUE (writer_id, seq)
);
CREATE INDEX audit_ts ON audit(ts_ms);
```

Rows are **insert-only**. Each writer keeps its own chain: `prev_hash` is the
hash of the same writer's previous row (found by `writer_id, seq - 1`), so
devices and the server never break each other's chains, and no `meta` head is
needed. Verification walks each writer's chain in `seq` order and checks the
`UNIQUE (writer_id, seq)` has no gaps below the newest row.

**Sync (🟡, engine + store).** Today a pack carries the whole SQLCipher
snapshot, but the merge takes only entries, attachments and the name out of it
(`sync::engine::records_from_snapshot` → `Snapshot { records, attachments,
name, name_ms }`, `SqliteStore::merge`), and `meta` is ignored. Audit rows must
travel and merge as a **union by `id`**, never last-writer-wins and never
tombstoned. Changes:

- `Snapshot` gains `audit: Vec<AuditRow>`; `SqliteStore::export_for_sync`
  gains the audit rows (or a sibling `export_audit_for_sync`), and
  `SqliteStore::merge` inserts them with `INSERT OR IGNORE`.
- `store::hash::vault_digest` must include the audit rows, or a device whose
  only change is new audit rows would never push (`engine::sync` pushes on a
  digest difference).
- A restore from a pack on a fresh install already carries the whole file,
  audit included.

**Retention.** Default 365 days, pruned at sync time like tombstones
(`TOMBSTONE_TTL_MS` has the pattern): a writer deletes its own rows older than
the cutoff and writes a `pruned_through` row whose `detail` names the cutoff
`seq`, so its chain stays verifiable from that row on. Tiers 2/3 keep the
server copy per tenant policy and export before pruning.

**Writers.** `rowel_broker::audit::Writer` with `fn record(&self, Row)`; the
`Host` implementations call it at every state change in `03`'s lifecycle.
`writer_id` is the workspace install id in Tier 1 (`workspace.rs`, the id
`AppHost::database_hash` is derived from) and the server's node id in Tier 2.
The `Mock` host collects rows so protocol tests can assert on them.

**Details as built (I1).** `detail` is a small JSON object: `denied` carries
`{"reason": …}` with `no_target`, `lockbox`, `no_rule`, `no_candidates`,
`item`, `user` or `timeout`; `used` carries nothing for the fill that followed
an ask, `{"reuse":true}` for a grant served again before policy ran, and
`{"via":"grantId"}` for the `grantId` paths; `rule_added` carries
`{"ruleId","days"}`; `revoked` carries `{"reason":"user"|"switch"|"lockbox"}`;
`ended` `{"reason":"expired"}`; `exposed` `{"reason":"exposure"}`. The
`target` column is the page reduced to scheme, host and path.

**Surfaces.**

- Tier 1: `src/components/Main/Sidebar/Settings/Sections/Audit/` (Vault Health)
  gains an *Agent activity* panel: last 7 days of `used`/`denied`/`exposed`
  events, standing rules older than 30 days, agents unseen for 90 days, and
  a *Verify log* action that walks the chains and reports the first break.
- Tier 2/3: `GET /v1/admin/audit/export` (JSONL, filters), webhook and OTLP
  (`08`). Each exported row carries `writer_id`, `seq`, `prev_hash`, `hash`.

**Never in the log.** Secret values, TOTP codes, passkey private material,
full URLs with query strings (the target is scheme+host+path), or reasons
longer than 300 characters. The redactor (`05`) runs over `detail` before
insert as a belt-and-braces check.
