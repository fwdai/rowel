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

*As built in I5* (`src-tauri/crates/serve`, `rowel-serve`, linked by the
CLI): the headless mode without `--server`. `HeadlessHost: Host` runs the
broker's engine (`rowel_broker::agents`) over a device-held lockbox the
operator copied to the box, under its own session → agents → approvals locks,
with the lockbox's `agents` flag from the registry as the one switch (the
app's Agents switch is implied on) and the data directory's `install_id` as
the audit writer, so the chain a device started continues. Unlock: the master
password at the terminal (no echo; one line from stdin when it is not a
terminal), `--key-file` (the vault's key material, raw bytes, `0600`, written
by `rowel admin key-file`; read again at every open, so an idle lock is lifted
by the next request) or `--key-fd` (read once; an idle lock is refused in that
mode). Config: `serve.json` beside the lockbox — `rules` as `policy::Rule`
serializes, forced to `CreatedBy::Admin("config")`, `askTimeoutMs` (120 000),
`idleLockMs`, `listen { socket, ws, tls { cert, key } }` — re-read on mtime.
Approvals: a queue, not a slot (`03 §4`); each ask is announced once on
stderr with the command that answers it, and answered by `rowel approve <id>`
over an owner-only control socket (`serve.sock`, frames of JSON: `pending`,
`answer`, `lock`, `status`) or refused at its deadline; associations are asks
too, never automatic, for the WebSocket's sake; a step-up ask takes the
master password at `rowel approve`, compared with the held key in constant
time. Listeners: the data directory's socket through `rowel_broker::server`,
and a
WebSocket (tokio + tungstenite; TLS from PEM files through `tokio-rustls` or
plain behind a reverse proxy) where a binary message is one frame and each
connection is a reader task, a std thread owning the `Connection`, and a
writer task the lock signals reach through the server's sink. What a
stranger at the port can cost is bounded before association: 256 open
connections, a 10 s handshake deadline, the broker's 1 MiB frame cap per
message, and a queue of four requests per connection past which the reader
stops taking from the socket; a connection's place under the ceiling is the
serving thread's, so hanging up mid-ask releases nothing. One broker runs
per data directory: the app and `rowel serve` take the same exclusive lock,
`rowel_broker::server::Lock` on `broker.lock`, before binding anything,
whatever they listen on, so neither can unlink the other's live socket.
`--key-fd` is for a descriptor a wrapper hands in (`3<
file`); a systemd `LoadCredential=` is a file under `$CREDENTIALS_DIRECTORY`
and is passed with `--key-file`, so the idle lock works under systemd.
SIGTERM and SIGINT seal and exit 0, also while the idle-lock password prompt
waits (the prompt runs on a thread of its own). `generate-password` is
refused (the generator's settings are the app's). Server mode, tenants,
`OrgHost`, workload identity and the member and admin APIs below remain I7.

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

*As built in I6* (`sync/providers/rowel`, the device half): `Provider::
RowelServer` is in `Provider::ALL` (its token file is `auth/rowel-server.swftx`)
but never in `Provider::available()` — an org lockbox is added from Settings
› Lockboxes › Rowel Server, not from the account pickers, since there is no
password to restore it under. The adapter's listing is `GET
/v1/members/me/lockboxes` filtered to the lockboxes that carry an `etag`
(a pack-less lockbox is created by the first push); the handle is the lockbox
id, the name `layout::vault_file_name(id)`, the revision the ETag verbatim.
`create` is `PUT` with `If-None-Match: *`, `update` is `PUT` with `If-Match:
<etag>`; a 412 fails the run with the retry message, as Dropbox's
compare-and-swap does, and the next run pulls and merges. The server must
refuse a `PUT` that carries neither precondition. `delete` is refused on a
device: an org lockbox is removed at the server. Rotation on the device: a
listing whose `gen` is above the local sidecar's re-keys the local copy to
the new master through the existing re-key saga, retiring the old epoch so a
pack pushed under it still opens (`opened_under_retired`).

**Member API (🟡 the app's side is built in I6, `src-tauri/src/org/client.rs`
is the contract; the server implements it in I7).** camelCase JSON, except
the token endpoint, which is RFC 6749's and keeps its snake_case.

```
  GET  /v1/members/auth/authorize ?response_type=code&client_id=rowel-app&redirect_uri&state
                                  &code_challenge&code_challenge_method=S256
                                  the server sends the browser through the company IdP and back
  POST /v1/members/auth/token     form: grant_type=authorization_code|refresh_token, code,
                                  code_verifier, redirect_uri | refresh_token, client_id
                                  → { access_token, refresh_token?, expires_in?, token_type }
  GET  /v1/members/me             → { member: { id, email, name? }, tenant: { name } }
  POST /v1/members/enroll         { devicePubkey, deviceName }  → { deviceId, state }   (pending)
  GET  /v1/members/me/devices     → [ { id, name, state: pending|approved|revoked, publicKey, createdAtMs } ]
  POST /v1/members/devices/{id}/approve      from an approved device of the same member, or an admin
  GET  /v1/members/me/lockboxes   → [ { id, name, kdf: "<External descriptor JSON, verbatim>",
                                        wrappedMaster?: "<sealed box to this device, base64>"  (approved devices only),
                                        etag?: "<current pack revision>"  (absent until a device pushed one) } ]
  GET  /v1/members/me/approvals/stream   SSE: `event: approval` + `data: <Approval>`; a `:` keepalive
                                         comment at least every 30 s (the app's loop wakes on it)
  POST /v1/approvals/{id}         { answer: GrantAnswer | null }
```

`Approval { id, lockboxId, lockboxName, agentName, mode, targets, candidates:
[VisibleItem], reason?, stepUp, expiresAtMs }`, `targets` in the shape the
app's grant dialog already renders (`events::AskTarget`). A 401 on any member
call means the session is over: the app refreshes once through the token
endpoint and retries, and otherwise tells the member to sign in again.

On the device, an org lockbox is one more non-primary workspace: the app
unseals the master with its enrollment key, puts it in `appkey::Keyring`, and
writes the `WRAPPED_KEY_FILE` sidecar under the app key exactly as it does for
any other workspace, with the server's `External` descriptor as the KDF sidecar.
From then on it opens with `VaultKey::from_material` whenever the vault is
unlocked. The master password still gates everything: without an unlocked
vault neither the app key nor the enrollment key is available. *As built in
I6:* the member session is held **once per install**, not per lockbox — in
`servers.sealed` beside the enrollment key, sealed under the app key
(`org::connections`), since the session is the member's and two sealed copies
of one refreshable token would drift apart on the first rotation; an org
lockbox's own token file holds `Tokens { provider: RowelServer, account:
Some(server url) }` and nothing else, as iCloud's holds only the account, and
the sync adapter looks the session up by that name. The vault id of an org
lockbox *is* the server's lockbox id. The registry's `Workspace.server` names
the server, so Settings can say which lockboxes are org lockboxes while they
are locked. The app is the member's approval device on desktop: one stream
per connection with an approved device, started when the vault becomes live
and stopped on lock; each approval goes to the consent slot as a grant ask
carrying the tenant's name and no session end, the person is verified for a
step-up as for a local ask, and the answer is posted back; a timeout posts
nothing and the server's own deadline decides. The server's log is the
record of its grants; the device writes no local audit row for them.

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
