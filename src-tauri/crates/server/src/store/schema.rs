//! The DDL history. Append an `M::up` for each schema change, never edit
//! one: `user_version` counts them, and a database above the count was
//! written by a newer build and is refused (`Store::open`).

use rusqlite_migration::{Migrations, M};

pub(super) fn migrations() -> Migrations<'static> {
    Migrations::new(migration_list())
}

pub(super) fn schema_version() -> i64 {
    migration_list().len() as i64
}

fn migration_list() -> Vec<M<'static>> {
    vec![
        // Tokens are stored as SHA-256 hex, never as presented; a refresh
        // token's hash moves to `spent_refresh_hash` when it is used, which
        // is how a second presentation of it is recognised and the session
        // revoked (RFC 6819 §5.2.2.3). Masters are wrapped under the tenant
        // key; the copy sealed to a device sits in `device_masters` with the
        // generation it was sealed at, so a rotation is seen as a stale
        // generation. The audit mirror is keyed as a lockbox's own chain is,
        // per writer and sequence, so a row mirrored twice is one row.
        //
        // The schema has shipped in no build yet, so a table or a column
        // added while the first server iteration is being built
        // (`server_events`, `trust_policies.client_kind`) goes into this
        // first migration rather than a second one; from the first release
        // on, the rule at the top of this file holds.
        M::up(
            "CREATE TABLE tenant (
               id            TEXT PRIMARY KEY,
               name          TEXT NOT NULL,
               created_at_ms INTEGER NOT NULL
             );
             CREATE TABLE members (
               id             TEXT PRIMARY KEY,
               email          TEXT NOT NULL UNIQUE COLLATE NOCASE,
               name           TEXT,
               role           TEXT NOT NULL,
               created_at_ms  INTEGER NOT NULL,
               disabled_at_ms INTEGER
             );
             CREATE TABLE devices (
               id             TEXT PRIMARY KEY,
               member_id      TEXT NOT NULL REFERENCES members(id),
               name           TEXT NOT NULL,
               public_key     TEXT NOT NULL,
               state          TEXT NOT NULL,
               created_at_ms  INTEGER NOT NULL,
               approved_at_ms INTEGER,
               approved_by    TEXT
             );
             CREATE INDEX devices_member ON devices(member_id);
             CREATE TABLE sessions (
               id                   TEXT PRIMARY KEY,
               member_id            TEXT NOT NULL REFERENCES members(id),
               device_id            TEXT REFERENCES devices(id),
               access_hash          TEXT NOT NULL UNIQUE,
               refresh_hash         TEXT UNIQUE,
               spent_refresh_hash   TEXT,
               access_expires_at_ms INTEGER NOT NULL,
               created_at_ms        INTEGER NOT NULL,
               revoked_at_ms        INTEGER
             );
             CREATE INDEX sessions_spent_refresh ON sessions(spent_refresh_hash);
             CREATE TABLE auth_codes (
               code_hash      TEXT PRIMARY KEY,
               member_id      TEXT NOT NULL REFERENCES members(id),
               code_challenge TEXT NOT NULL,
               redirect_uri   TEXT NOT NULL,
               expires_at_ms  INTEGER NOT NULL
             );
             CREATE TABLE lockboxes (
               id             TEXT PRIMARY KEY,
               name           TEXT NOT NULL,
               kdf            TEXT NOT NULL,
               wrapped_master BLOB NOT NULL,
               gen            INTEGER NOT NULL,
               frozen         INTEGER NOT NULL DEFAULT 0,
               agents         INTEGER NOT NULL DEFAULT 0,
               revision       INTEGER NOT NULL DEFAULT 0,
               created_at_ms  INTEGER NOT NULL
             );
             CREATE TABLE lockbox_members (
               lockbox_id TEXT NOT NULL REFERENCES lockboxes(id) ON DELETE CASCADE,
               member_id  TEXT NOT NULL REFERENCES members(id) ON DELETE CASCADE,
               role       TEXT NOT NULL,
               PRIMARY KEY (lockbox_id, member_id)
             );
             CREATE TABLE device_masters (
               lockbox_id TEXT NOT NULL REFERENCES lockboxes(id) ON DELETE CASCADE,
               device_id  TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
               gen        INTEGER NOT NULL,
               sealed     TEXT NOT NULL,
               PRIMARY KEY (lockbox_id, device_id)
             );
             CREATE TABLE agents (
               id            TEXT PRIMARY KEY,
               name          TEXT NOT NULL,
               lockbox_id    TEXT NOT NULL REFERENCES lockboxes(id),
               enabled       INTEGER NOT NULL DEFAULT 1,
               created_at_ms INTEGER NOT NULL
             );
             CREATE TABLE trust_policies (
               id             TEXT PRIMARY KEY,
               agent_id       TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
               issuer         TEXT NOT NULL,
               claims         TEXT NOT NULL,
               fixed_audience INTEGER NOT NULL DEFAULT 0,
               client_kind    TEXT NOT NULL DEFAULT 'agent',
               created_at_ms  INTEGER NOT NULL
             );
             CREATE INDEX trust_policies_issuer ON trust_policies(issuer);
             CREATE TABLE clients (
               key             TEXT PRIMARY KEY,
               agent_id        TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
               name            TEXT NOT NULL,
               kind            TEXT NOT NULL,
               platform        TEXT,
               created_at_ms   INTEGER NOT NULL,
               last_seen_at_ms INTEGER
             );
             CREATE TABLE rules (
               id            TEXT PRIMARY KEY,
               lockbox_id    TEXT REFERENCES lockboxes(id) ON DELETE CASCADE,
               rule          TEXT NOT NULL,
               created_at_ms INTEGER NOT NULL
             );
             CREATE TABLE pending_approvals (
               id                 TEXT PRIMARY KEY,
               lockbox_id         TEXT NOT NULL REFERENCES lockboxes(id) ON DELETE CASCADE,
               agent_id           TEXT,
               client_key         TEXT NOT NULL,
               approval           TEXT NOT NULL,
               state              TEXT NOT NULL,
               created_at_ms      INTEGER NOT NULL,
               expires_at_ms      INTEGER NOT NULL,
               answered_at_ms     INTEGER,
               answered_by_device TEXT
             );
             CREATE INDEX pending_approvals_lockbox ON pending_approvals(lockbox_id, state);
             CREATE TABLE audit (
               lockbox_id TEXT NOT NULL,
               writer_id  TEXT NOT NULL,
               seq        INTEGER NOT NULL,
               id         TEXT NOT NULL,
               ts_ms      INTEGER NOT NULL,
               prev_hash  BLOB NOT NULL,
               hash       BLOB NOT NULL,
               actor_kind TEXT NOT NULL,
               actor_id   TEXT,
               actor_name TEXT,
               run_id     TEXT,
               acting_for TEXT,
               grant_id   TEXT,
               item_id    TEXT,
               target     TEXT,
               mode       TEXT,
               approval   TEXT,
               approver   TEXT,
               event      TEXT NOT NULL,
               detail     TEXT,
               PRIMARY KEY (lockbox_id, writer_id, seq)
             );
             CREATE INDEX audit_ts ON audit(ts_ms);
             CREATE TABLE server_events (
               id        TEXT PRIMARY KEY,
               ts_ms     INTEGER NOT NULL,
               event     TEXT NOT NULL,
               member_id TEXT,
               device_id TEXT,
               detail    TEXT
             );
             CREATE INDEX server_events_ts ON server_events(ts_ms);
             CREATE TABLE replay (
               jti           TEXT PRIMARY KEY,
               expires_at_ms INTEGER NOT NULL
             );",
        ),
    ]
}
