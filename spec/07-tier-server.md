# 07 — Tier 2: the self-hosted Rowel Server

## 1. What a company gets

A single binary a company runs inside its network. It holds the lockboxes their
agents use, verifies agents by the identity their platforms already give them,
decides by policy, routes approvals to people's Rowel apps, and writes an audit
log it can export. Employees keep using the Rowel app they already have; an org
lockbox appears beside their personal ones, behind the same master password,
synced through the server instead of through a personal cloud account.

Nothing about the agent's side changes between a laptop and production. The
same placeholders, the same `request-grant`, the same proxy and `run` commands.
What changes is the broker's address and how the agent proves who it is.

## 2. Shape

```
  ┌──────────────────────────────── company network ─────────────────────────────────┐
  │                                                                                   │
  │   CI job (GitHub OIDC)      k8s pod (SA token)        cloud browser farm          │
  │   ┌──────────────┐          ┌──────────────┐          ┌──────────────────┐        │
  │   │ agent        │          │ agent        │          │ browser + Rowel  │        │
  │   │ + rowel proxy│          │ + sidecar    │          │ extension (WS)   │        │
  │   └──────┬───────┘          └──────┬───────┘          └────────┬─────────┘        │
  │          │ wss /v1/agent           │ wss /v1/agent              │ wss /v1/extension│
  │   ┌──────▼──────────────────────────▼────────────────────────────▼─────────────┐  │
  │   │ rowel-server                                                               │  │
  │   │  identity verifiers · OrgHost (Host over org lockboxes) · policy · grants  │  │
  │   │  approval router · audit · pack endpoints for devices · admin API          │  │
  │   │  ┌───────────────┐  ┌───────────────────────────┐  ┌──────────────────┐    │  │
  │   │  │ metadata DB   │  │ org lockboxes (SQLCipher  │  │ root key: KMS or │    │  │
  │   │  │ (Postgres /   │  │ files, one per lockbox)   │  │ sealed key file  │    │  │
  │   │  │  SQLite)      │  └───────────────────────────┘  └──────────────────┘    │  │
  │   │  └───────────────┘                                                         │  │
  │   └──────────────────────────────▲─────────────────────────▲──────────────────┘  │
  │                                  │ https /v1/members        │ https /v1/admin     │
  │                       ┌──────────┴──────────┐     ┌─────────┴─────────┐           │
  │                       │ employee's Rowel app│     │ admin console     │           │
  │                       │ approvals · sync of │     │ (web, later; the  │           │
  │                       │ org lockboxes       │     │ app's Settings    │           │
  │                       └─────────────────────┘     │ first)            │           │
  │                                                   └───────────────────┘           │
  └───────────────────────────────────────────────────────────────────────────────────┘
```

## 3. The five jobs of the server

1. **Hold org lockboxes it can open.** An org lockbox is the same SQLCipher file
   the app writes, keyed by a random master the server wraps under its root key
   and under each member device's key. The server opens it to serve grants and
   to merge what devices push.
2. **Know who is asking.** Members sign in with the company's identity provider
   and enroll devices. Agents present workload tokens matched to trust
   policies (`02`).
3. **Decide.** The same policy evaluator as the app, over rules admins keep per
   lockbox or per tenant (`03`).
4. **Ask the right person.** Pending approvals are routed and delivered to
   members' apps; answers resolve the waiting agent (`03`).
5. **Remember.** Every decision and every use is an audit row, exportable (`09`).

## 4. How employees' apps join

A member connects the app to the server once (sign in through the identity
provider, enroll the device). The server lists the org lockboxes the member may
hold; each becomes a lockbox in the app, synced through the server as the
provider. From then on the app is also the member's approval device.

---

### Technical details

**Binary and crate (❌).** The server is not a separate binary. It is the
`rowel` CLI's `serve` subcommand in **server mode** (`rowel serve --server`),
built from a `rowel-serve` library crate (`src-tauri/crates/serve`) that the
CLI links. The same `serve` without `--server` is the headless broker for one
person's device-held lockbox on a VM (`PLAN.md`, I5); server mode switches on
tenants, members, workload identity, routing and the pack endpoints. Rust,
`axum` + `tokio`, `sqlx` for metadata (SQLite for the single-node self-host
default so the first run is one binary and one data directory, like the app;
Postgres as an alternative). Depends on `rowel-core` and `rowel-broker`; never
on Tauri. Configuration by file and environment: listen address, data
directory, root key source, OIDC issuer for members, allowed workload issuers.
In the diagram above, `rowel-server` is this binary in server mode.

**Root key and wrapping.**

```
  root key  ──HKDF("tenant", tenant_id)──▶ tenant key ──AES-GCM wrap──▶ lockbox master (32 B)
     │                                                                        │
     ├─ self-host: a 32-byte key file the operator holds (0600, outside the   │
     │  data dir) or a KMS key (AWS KMS / GCP KMS / Vault transit) via a      │
     │  `RootKey` trait with `wrap`/`unwrap`                                   │
     └─ cloud: HSM-backed KMS, one key per region                              │
                                                                               ▼
  per member device:  crypto_box::seal(device_x25519_pub, lockbox master)  (sealed box)
```

Lockbox masters are unwrapped into memory on first use and held in a
`Zeroizing` LRU keyed by lockbox id with an idle timeout (default 10 min), so
a quiet server holds no plaintext keys. Every unwrap is an audit row.

**OrgHost (❌).** `impl Host for OrgHost` over `rowel_core::store::SqliteStore`
opened with the lockbox's SQLCipher subkey, exactly as `sync::engine::WorkspaceVault`
opens a lockbox the app does not have active. Method by method:

- `enabled` → the lockbox is not frozen by an admin.
- `database_hash` → sha256 of the lockbox id (the same scheme as
  `AppHost::database_hash`, so extensions key associations the same way).
- `clients` / `associate` → the agent and client tables; `associate` verifies
  `proof` through `IdentityVerifier` instead of asking a person.
- `logins_for`, `totp`, `passkey_*` → the same core functions over the store,
  gated by an active grant (the server never serves a fill without one).
- `request_grant` / `wait_grant` → policy from the tenant store; pending ones
  into `pending_approvals`; `wait_grant` parks on a `tokio::sync::Notify`.
- `resolve` → sidecar kind only, as in Tier 1.
- `save_login` → allowed for extension clients under a grant with mode `fill`
  and `save: true` (new accounts an agent created); otherwise refused.

One `Connection<OrgHost>` per WebSocket; the broker crate's connection code is
unchanged. Concurrency: the lockbox store is behind a per-lockbox `Mutex`
(SQLite writes serialize anyway); reads for grants are short.

**Device sync through the server (🟡).** The app gains `Provider::RowelServer`
beside Google Drive, Dropbox and iCloud (`sync/provider.rs`, `sync/store.rs`
`AnyStore`). Its `VaultStore` talks to:

```
  GET  /v1/lockboxes/{id}/pack           → bytes + ETag (revision)
  HEAD /v1/lockboxes/{id}/pack           → ETag only            (head_revision)
  PUT  /v1/lockboxes/{id}/pack  If-Match → 200 new ETag | 412   (upload with CAS)
```

The server does **not** store the device's pack as-is. On `PUT` it runs the
receiving half of the engine over its own store: `engine::decode_pack` (the
pack is sealed under the same master, so the server opens it exactly as a
second device would), then `SqliteStore::merge` (last-writer-wins per entry,
tombstones, attachment reconciliation, audit union), then answers with a new
revision. On `GET` it serves `PackKeys::pack` of its current state, so the
device's next pull is the union. The device side keeps running the full
`engine::sync` loop unchanged; only the `Remote` is new. This makes the server
the merge authority and lets agent writes (a `save_login`) and device writes
interleave safely: every server-side write bumps the revision, so a device's
compare-and-swap fails with 412 and the engine retries (`MAX_ATTEMPTS`). The
pack format is unchanged, including the KDF descriptor in the header, which for
an org lockbox is `KdfParams::External { kid, gen }` (`01`). The interleaving
cases (agent write during a device push, two devices and one agent, a device
offline past the tombstone TTL) get a test matrix on the `two_devices.rs`
harness with the server as a third party (Q19).

**Member API (❌).**

```
  POST /v1/members/enroll         { oidc_id_token, device_pubkey, device_name }  → device_id (pending)
  GET  /v1/members/me/devices     → [ { id, name, state: pending|approved|revoked } ]
  POST /v1/members/devices/{id}/approve      from an approved device of the same member, or an admin
  GET  /v1/members/me/lockboxes   → [ { id, name, kdf: External descriptor,
                                        wrapped_master (sealed box to this device; approved devices only) } ]
  GET  /v1/members/me/approvals/stream   (SSE)  → pending asks for this member
  POST /v1/approvals/{id}         { answer: GrantAnswer }
```

On the device, an org lockbox is one more non-primary workspace: the app
unseals the master with its enrollment key, puts it in `appkey::Keyring`, and
writes the `WRAPPED_KEY_FILE` sidecar under the app key exactly as it does for
any other workspace, with the server's `External` descriptor as the KDF sidecar.
From then on it opens with `VaultKey::from_material` whenever the vault is
unlocked. The master password still gates everything: without an unlocked
vault neither the app key nor the enrollment key is available.

**Agent and admin API (❌).**

```
  WS   /v1/agent            the sealed channel; `associate` carries `proof`
  WS   /v1/extension        same channel for remote extensions; `associate`
                            carries an enrollment code minted for a fill grant
  GET  /v1/agent/challenge  nonce + audience for proof binding

  /v1/admin/agents          CRUD agent definitions + trust policies
  /v1/admin/lockboxes       create (mints and wraps a master), freeze, members
  /v1/admin/rules           policy rules per lockbox / tenant
  /v1/admin/audit/export    JSONL stream, filters by time / agent / lockbox
```

Admin UI: start inside the Rowel app's Settings for members with the admin
role (the app already has the lockbox and rule concepts); a web console is a
later addition, not a prerequisite.

**Metadata schema (sketch).** `tenants`, `members`, `devices`, `lockboxes`
(id, tenant, name, wrapped_master, frozen), `lockbox_members` (lockbox, member,
role, wrapped_master_for_device per device), `agents`, `trust_policies`,
`clients`, `rules`, `grants`, `pending_approvals`, `audit` (also mirrored
inside each lockbox per `09`, so a lockbox restored elsewhere carries its own
history).

**Operational notes.** Single binary, `rowel serve --server` and `rowel
admin …`. Health endpoint. Backups = metadata DB dump + lockbox files + the
root key held separately. Horizontal scale: lockboxes are the unit; shard by
lockbox id when one node is not enough (`10`). Outage behaviour: agents fail
closed (no cached values anywhere); employees' apps keep working on their
local copies of org lockboxes and sync when the server is back.
