# The Rowel Server API

A Rowel Server is the company's broker: it holds org lockboxes, verifies
agents by the identity their platforms give them, decides by policy, routes
approvals to members' Rowel apps and keeps the audit log (spec `07`). This
document is the contract between a server and everything that talks to one —
the app, `rowel-cli`, the browser extension — so any server, ours or one you
write yourself, can be checked against it. `rowel serve --server`
(`src-tauri/crates/server`, [docs/server.md](./server.md)) implements it;
the app's half, `src-tauri/src/org/client.rs`, `org/approvals.rs` and
`sync/providers/rowel`, speaks it, and the in-process server the app's tests
run against, `org/fake_server.rs`, answers it the same way. Each endpoint is
marked ✅ when a shipped build serves and speaks it and ❌ when it is planned.

## Transport rules

- **One base URL.** A server is `https://host[:port][/prefix]`. The app
  normalizes it (`connections::normalize_url`): lowercase scheme and host,
  the port only when it is not the scheme's default, the path with no
  trailing slash. Every endpoint below is `<base>/v1/…`, so a server may sit
  under a path prefix behind a reverse proxy.
- **https, or http to loopback only.** `http://` is accepted for
  `localhost`, `127.0.0.0/8` and `[::1]` — a server on this machine — and
  refused everywhere else before a request is built. A bearer token, a
  wrapped master or a pack never crosses a network in the clear.
- **Bearer tokens.** Every member call carries `Authorization: Bearer
  <access_token>` except the two that produce one (`authorize`, `token`).
- **JSON, camelCase.** Bodies and replies are JSON with camelCase keys. The
  one exception is the token endpoint, which is RFC 6749's and keeps its
  snake_case form and reply. A client ignores fields it does not know, so a
  server may add some; it may not add values to a closed vocabulary (a
  device `state`, a grant `mode`, a target `kind`), which a client cannot
  read and treats as an error.
- **Status codes.** Any 2xx is success. A **401 on any member call means the
  session is over**: the app refreshes once through the token endpoint,
  keeps the new session, and retries the call once; a second 401 ends the
  sign-in and the member is told to sign in again. Any other failure is
  shown to the member with the status and the first 1 MiB of the body, so
  put the reason there in plain words. JSON replies are read up to 1 MiB; a
  pack up to the app's pack cap (`MAX_PACK_BYTES`, 256 MiB).
- **WebSocket.** On `/v1/agent` a binary message carries exactly one frame's
  JSON body; text messages are ignored. The sealed channel inside it is the
  broker's, unchanged (`docs/serve.md`).

## Member API ✅

The app is an OAuth 2.0 public client, `client_id` `rowel-app`, authorization
code with PKCE (S256) and a loopback redirect. The server is the authorization
server; it federates to the company's identity provider on its own side, so
the app never sees the IdP.

### `GET /v1/members/auth/authorize` ✅

Unauthenticated; opened in the browser. Query: `response_type=code`,
`client_id=rowel-app`, `redirect_uri` (an `http://127.0.0.1:<port>/…` the app
is listening on), `state`, `code_challenge`, `code_challenge_method=S256`.
The server takes the person through sign-in and redirects to `redirect_uri`
with `code` and the same `state`. A request with a client id, response type
or challenge method other than these is a 400, and so is a `redirect_uri`
that is not `http://127.0.0.1:<port>/…` or `http://localhost:<port>/…`: no
redirect is registered with the server, so anything but the app's own
listener would let a crafted link send a member's code elsewhere. A sign-in
the server refuses after the person went through the IdP — an address
outside the allowed domains, a disabled member, an unknown one where the
server does not provision — comes back to `redirect_uri` too, as RFC 6749
§4.1.2.1 has it: `error=access_denied`, `error_description` in words, and
the same `state`, never a `code`. A server with no IdP configured answers
503.

### `POST /v1/members/auth/token` ✅

Unauthenticated; `application/x-www-form-urlencoded`, RFC 6749. Two grants:

```
grant_type=authorization_code  code  code_verifier  redirect_uri  client_id=rowel-app
grant_type=refresh_token       refresh_token                      client_id=rowel-app
```

The code is single-use and bound to the challenge sent with it: the server
checks `BASE64URL(SHA-256(code_verifier))` against it and answers 400 to a
mismatch, with an RFC 6749 §5.2 error body. A refresh token the server no
longer holds is a 401. The reply, 200:

```json
{ "access_token": "…", "refresh_token": "…", "expires_in": 3600, "token_type": "Bearer" }
```

`refresh_token` and `expires_in` are optional. The server may rotate the
refresh token on each refresh; the app keeps whichever the reply names. A
session that enrolled a device stays bound to that device across refreshes.

### `GET /v1/members/me` ✅

```json
{ "member": { "id": "m-pat", "email": "pat@acme.example", "name": "Pat" },
  "tenant": { "name": "Acme" } }
```

`name` is optional. The app calls this once at sign-in and keeps the answer
in the connection record.

### `POST /v1/members/enroll/challenge` ✅

```json
{ "devicePubkey": "<X25519 enrollment public key, base64>" }
```

Reply 200 `{ "challenge": "<crypto_box sealed box to devicePubkey, base64>" }`.
An enrollment proves that the app holds the secret half of the key it
enrolls; X25519 signs nothing, so the proof is opening a box sealed to the
key. The box holds, in order:

```
ENROLL_CHALLENGE_CONTEXT   30 bytes: "rowel enrollment challenge v1" and a NUL (rowel_core::crypto)
publicUrl                  the server's public URL, normalized, as UTF-8
0x00
secret                     32 random bytes
```

The app answers only a challenge with that context in front, so a wrapped
master, 32 bytes sealed to the same key, can never be passed off as a
challenge and handed back in the clear. And it answers only one whose URL is
the one it is enrolling at, spelled as `connections::normalize_url` spells
it (the server's `normalize_public_url` spells every address the same way).
Any other URL is refused before a proof is sent, so a server the member also
signs in to cannot relay this one's challenge to the app and enroll here as
the member's device with the answer. A server's `publicUrl` must therefore be
exactly the address its members connect to. The challenge is issued to this
session for this key, lives five minutes, and is held by the server only as
the SHA-256 of the 32 bytes, one per session: asking again, for any key, replaces it. A
`devicePubkey` that is not 32 bytes of base64 is a 400.

### `POST /v1/members/enroll` ✅

```json
{ "devicePubkey": "<X25519 enrollment public key, base64>", "deviceName": "Pat's MacBook",
  "proof": "<the 32 bytes the challenge opened to, base64>" }
```

Reply 200 `{ "deviceId": "…", "state": "pending" }`. The session that
enrolled the device is from now on that device's, and no session is a
device's any other way. A key the member enrolled before is the same device
and comes back as it stands, approved or not, so the app enrolls after every
sign-in, new install or not, and a renewed session picks its device and its
approval back up.

That is why the proof is required every time, for a new key and a known one
alike: every device's public key is on the member's device list, and without
it any session of the member could enroll under an approved device's key and
be handed that device's masters. A `proof` that is missing, or that is not
the secret of the live challenge issued to **this session for this key**, is
a 403, and nothing is enrolled or bound. The challenge is spent by any
answer, right or wrong, so a refused enrollment asks for a new one. A missing
`devicePubkey` or `deviceName` is a 400.

### `GET /v1/members/me/devices` ✅

```json
[ { "id": "d1", "name": "Pat's MacBook", "state": "approved",
    "publicKey": "<base64>", "createdAtMs": 1700000000000 } ]
```

`state` is one of `pending`, `approved`, `revoked`. `publicKey` is the
enrollment key verbatim, so every device of the member computes the same
fingerprint before approving another.

### `POST /v1/members/devices/{id}/approve` ✅

No body. Allowed from an approved device of the same member (or an admin,
through the admin API); from a pending device it is a 403, for an unknown
device a 404. Reply 204.

### `GET /v1/members/me/lockboxes` ✅

```json
[ { "id": "lb1", "name": "Deploy",
    "kdf": "{\"algo\":\"external\",\"kid\":\"…\",\"gen\":1}",
    "wrappedMaster": "<crypto_box sealed box to this device's enrollment key, base64>",
    "etag": "\"42\"" } ]
```

`id` is the lockbox's id at the server and its vault id on every device.
`kdf` is the `KdfParams::External` descriptor as one JSON string, carried
verbatim into the sidecar and every pack header; a higher `gen` than the
device holds is a rotation and the device re-keys. `wrappedMaster` is the
lockbox master sealed to the calling device's enrollment key, and is present
**only when the session's device is approved**; a pending or revoked device,
or a session with no device, gets the listing without it. `etag` is the
current revision of the lockbox's pack and is absent until a device has
pushed one; the sync adapter lists only lockboxes that carry it, so no `HEAD`
per lockbox is needed.

### `GET /v1/members/me/approvals/stream` ✅

`Accept: text/event-stream`; the reply is `Content-Type: text/event-stream`
and stays open. The server sends every approval pending for this member,
then each new one as it is queued, as:

```
event: approval
data: {"id":"ap1","lockboxId":"lb1",…}

```

The app frames by the SSE rules: a line ends at `\r\n`, `\n` or a lone `\r`,
a frame at an empty line; `data:` lines are joined with `\n`; a frame
without `event: approval` or without data — a comment, or an event of a kind
a later server adds — is skipped. A frame larger than 256 KiB ends the
stream. **Send a `:` keepalive comment at least every 30 s**: the app's read
timeout is 60 s, a quiet stream is taken for dead and reconnected with
backoff (5 s doubling to 60 s), and the keepalive is also the moment the
app's loop checks whether it should stop. An approval whose `expiresAtMs` is
already past is dropped unread. The stream is opened only from a device the
app holds as approved; a 401 refreshes the session once per attempt.

The `Approval`:

```json
{ "id": "ap1",
  "lockboxId": "lb1", "lockboxName": "Deploy",
  "agentName": "release-bot",
  "mode": "env",
  "targets": [ { "kind": "command", "line": "stripe balance retrieve" } ],
  "candidates": [ { "id": "i1", "title": "Stripe", "username": null } ],
  "reason": "deploying 1.2",
  "stepUp": false,
  "expiresAtMs": 1700000000000 }
```

`mode` is `fill`, `passkey`, `header`, `token` or `env`. A target is one of
`{ "kind": "origin", "origin": "https://…" }`, `{ "kind": "http", "host",
"path", "methods": [] }` or `{ "kind": "command", "line" }`. `candidates` are
`rowel_broker::grant::VisibleItem`: what the person picks from, titles and
usernames only. `reason` is optional, the agent's own words, at most 300
characters. `stepUp` says the rule wants the master password (or biometrics)
before the answer counts. No value, placeholder or field ever rides in an
approval.

### `POST /v1/approvals/{id}` ✅

```json
{ "answer": { "itemId": "i1", "standingDays": null } }
```

`answer` is `rowel_broker::agents::GrantAnswer` — the item picked and, for a
standing grant, for how many days; `forRun` (optional, default `false`) is
Tier 1's "for this run", which the app does not offer for a routed ask — or
`null` to refuse. Reply 204. An
approval the person left unanswered posts nothing: the server's own deadline
decides, and it must not be told "no". First answer wins; a late one may be
refused with any 4xx, which the app logs and does not retry. The server's
log is the record of its grants; the device writes none.

## Pack endpoints ✅

An org lockbox syncs through the server as its provider. There are no
folders: the listing above says which lockboxes there are, and each has one
pack resource. The device runs the full sync engine unchanged; only the
remote is new.

```
HEAD /v1/lockboxes/{id}/pack           → 200, ETag, Content-Length        | 403, 404
GET  /v1/lockboxes/{id}/pack           → 200, bytes, ETag                 | 403, 404
PUT  /v1/lockboxes/{id}/pack
     If-None-Match: *                   → 200, ETag   create              | 412 exists
     If-Match: "<etag>"                 → 200, ETag   update              | 412 moved, 404 gone,
                                                                            409, 400, 423, 413
     neither                            → 428
DELETE                                  → 405
```

- **Who.** The member must hold the lockbox, in any role, and call from an
  approved device of theirs: a pending or revoked device, or a session that
  never enrolled one, is a 403, since it holds no master and even the
  ciphertext tells size and timing. A lockbox the member does not hold is a
  404, exactly as one that does not exist, so nobody learns of a lockbox
  they were not given.
- **The ETag is the revision**, compared verbatim, quotes included. The
  device never reads it; it only hands it back. `HEAD` is `GET` without the
  body, so its `Content-Length` is the pack's. A server that makes a
  lockbox's file only at the first push answers 404 to `GET` and `HEAD`
  until then, and the first push creates it; the Rowel Server makes the
  file with the lockbox, so every lockbox it lists has a pack and an ETag,
  and a 404 there is only a lockbox that does not exist for the caller.
- **The PUT is a compare-and-swap.** `If-None-Match: *` creates and fails
  with 412 if a pack exists (always, at the Rowel Server; the device's next
  run then finds the pack, pulls, merges and updates); `If-Match` updates
  and fails with 412 unless the ETag is the current one; an `If-None-Match`
  carrying anything but `*` is a 400. **A PUT with neither precondition is
  refused** (428): the device never sends one, and a server must never take
  one, because a blind PUT would put a device's copy over a write it never
  merged. On 412 the sync run fails with its retry message and the next run
  pulls and merges. A body over `MAX_PACK_BYTES` is a 413.
- **The server merges; it does not store.** A pack is sealed under the
  lockbox master the server holds, so on `PUT` it opens the pack as a
  second device would, merges it into its own copy (last-writer-wins per
  entry, tombstones, attachments, audit rows as a union; the pushed name is
  ignored, since an org lockbox is named at the server) and answers with
  the new revision, a new one for every accepted push even when the merge
  changed nothing; on `GET` it serves a pack of its current state. The
  server is the merge authority, which is what lets an agent's writes (the
  rows its grants log) and two devices' pushes interleave. **Every
  server-side write bumps the revision**, an agent's and an unwrap of the
  lockbox's key included, so a device holding the old one fails its CAS and
  merges first. The revision a `PUT` answers with names the server's merged
  state, which is not always the pack pushed (a row of it may have lost to
  one the server held): a device learns what that revision holds by
  pulling it, as the engine does at the start of every run.
- **What a PUT is refused for, after the precondition.** A body that is not
  a pack, or one that does not open under the lockbox's key, is a 400. A
  pack under any descriptor but the lockbox's current one is a 409 ("this
  pack is under another key generation; refresh the lockbox key"): until
  rotation keeps retired generations at the server, it can open no other,
  and the device re-keys to the listing's generation before it pushes
  again. A pack written by a newer app than the server reads is a 409 too.
- **A frozen lockbox** takes no push (423 Locked) and is still served to
  `GET` and `HEAD`: its members already hold the master and a copy, so
  withholding the pack would take nothing from them and fail every sync
  run. Serving it writes nothing into it.
- A device never deletes an org lockbox; a `DELETE` is refused on the device
  and is 405 at the server.

## Agent API ✅

Agents reach the server the way they reach `rowel serve`: the CLI's
`ROWEL_BROKER=wss://host/v1/agent`, the sealed channel inside the WebSocket.
What changes is association. On a device a person answers a dialog; at the
server the agent proves who it is with a workload token, bound to the
channel it is on.

`<origin>` below is the `wss://` agent URL with the scheme swapped to
`https://`.

### `GET <origin>/v1/agent/challenge` ✅

Unauthenticated.

```json
{ "nonce": "<opaque>", "audience": "rowel:<tenant id>" }
```

The nonce is single-use and short-lived.

### `WS <origin>/v1/agent` ✅

The sealed channel, unchanged: `change-public-keys`, then the sealed
`associate`. Its `hello` object gains `proof`:

```json
{ "action": "associate", "key": "<session pubkey b64>", "idKey": "<id pubkey b64>",
  "hello": { "kind": "agent", "platform": "cli", "version": "1.2.0",
             "capabilities": ["grants", "placeholders"],
             "proof": { "kind": "oidc", "token": "<jwt>", "nonce": "<challenge nonce>" } } }
```

**Binding.** Before asking for the token the agent has its session key; it
requests the token with `aud` = `"<audience>:<hex>"`, `<hex>` the lowercase
hex SHA-256 of the raw 32 bytes of the session public key it sent in
`change-public-keys`. The server recomputes that from the key the channel
was opened with, so a token lifted from one channel fails `aud` on any
other. The CLI takes the token from GitHub Actions
(`ACTIONS_ID_TOKEN_REQUEST_URL` / `ACTIONS_ID_TOKEN_REQUEST_TOKEN`, asking
for that audience) or from a file named by `ROWEL_WORKLOAD_TOKEN_FILE`.

**Verification.** The server fetches the issuer's JWKS and checks the
signature, `exp`, and `aud`. For a platform whose tokens have a fixed
audience, `aud` must equal the bare audience, the echoed `nonce` must be the
one the challenge issued, the token's lifetime may be at most 10 minutes, and
its `jti` goes into a replay cache for that window. The replay cache runs for
every verifier regardless. The claims are then matched to a **trust policy**
— `{ issuer, claims: { repository: "acme/deploy", ref: "refs/heads/main",
workflow: "release.yml" } }`, exact match per claim, glob where an admin
allows it — which names the agent definition the client becomes. Only the
policies whose `clientKind` is the `hello`'s `kind` are candidates: a policy
admits an `agent` or a `sidecar`, never both, so a sidecar (the one kind that
receives values) proves an identity of its own, and a token that matches
only the agent's policy is refused when it says `sidecar`.

**Reply.** The usual association reply, `{ "hash", "id", "brokerCapabilities",
"clientId" }`; the stored client carries `agentId`, the definition it
resolved to, and is looked up again on every request, so disabling the
definition ends its sessions at their next request. A proof that does not
verify is `AssociationFailed`; the server never falls back to asking a person.

Everything after association is the broker's protocol as `docs/cli.md`
describes it — `request-grant`, `wait-grant`, `resolve` from the sidecar —
with the approval, when a rule asks for one, routed to a member's app through
the stream above.

`WS <origin>/v1/extension` ❌ is the same channel for a remote browser's
extension, let in by a one-time enrollment code minted for a fill grant.

## Admin API ✅

Member calls under the same bearer, for a member with the admin role: no
bearer, or one the server does not hold, is a 401 as everywhere; any other
member is a 403 on every route. JSON, camelCase. A create answers 201 with
the row, an update 200 with the row as it now is, a removal 204. A path
naming a row that does not exist is a 404; a body naming one (`lockboxId`)
is a 400; a body that does not read is a 400 saying why. Every change is
written down as a tenant event naming the admin (`GET /v1/admin/events`).
The first admin UI is the app's Settings for members with that role; a web
console comes later.

### Members and devices ✅

```
GET   /v1/admin/members                 → [Member]
POST  /v1/admin/members                 { email, name?, role: member|admin }  → 201 Member   | 409 taken
PATCH /v1/admin/members/{id}            { role?, disabled?: true }            → Member
GET   /v1/admin/members/{id}/devices    → [Device]
POST  /v1/admin/devices/{id}/approve    → Device                               | 409 revoked
POST  /v1/admin/devices/{id}/revoke     → 204
```

```json
{ "id": "m1", "email": "pat@acme.example", "name": "Pat", "role": "member",
  "createdAtMs": 1700000000000, "disabledAtMs": null }
{ "id": "d1", "memberId": "m1", "name": "Pat's MacBook", "publicKey": "<base64>",
  "state": "approved", "createdAtMs": 1700000000000,
  "approvedAtMs": 1700000001000, "approvedBy": "<device id, or an admin's member id>" }
```

A member who signs in for the first time is provisioned as `member` when the
server names allowed domains and the address is in one; with no domains
named, an admin adds each member here first. `disabled: true` offboards: the
member is marked (their sessions stop at the next request), every session is
revoked, and every device revoked with the masters sealed to it. A disabled
member is not enabled again (409). An admin cannot demote or disable
themselves (409), so the tenant always keeps an admin. Approving here is how
a member's first device gets in; a revoked device is never approved again.

### Lockboxes ✅

```
GET    /v1/admin/lockboxes                             → [Lockbox]
POST   /v1/admin/lockboxes                             { name }              → 201 Lockbox
PATCH  /v1/admin/lockboxes/{id}                        { frozen?, agents? }  → Lockbox
PUT    /v1/admin/lockboxes/{id}/members/{memberId}     { role: reader|approver|admin }  → 204
DELETE /v1/admin/lockboxes/{id}/members/{memberId}     → 204
```

```json
{ "id": "lb1", "name": "Deploy", "kdf": "{\"algo\":\"external\",…}", "gen": 1,
  "frozen": false, "agents": false, "revision": 0, "createdAtMs": 1700000000000,
  "members": [ { "memberId": "m1", "role": "approver" } ] }
```

Creating mints the master, wraps it under the tenant key and writes the
lockbox file; the wrapped master is never in a reply. A member given a role
gets the master sealed to each of their approved devices at their next
listing. `approver` and `admin` are who an agent's ask is routed to.

### Agents and trust policies ✅

```
GET    /v1/admin/agents                 → [Agent]
POST   /v1/admin/agents                 { name, lockboxId }    → 201 Agent
PATCH  /v1/admin/agents/{id}            { name?, enabled? }    → Agent
DELETE /v1/admin/agents/{id}            → 204   (its trust policies and clients go with it)
POST   /v1/admin/agents/{id}/trust      { issuer, claims: { "<claim>": "<pattern>" }, fixedAudience?,
                                          clientKind?: agent|sidecar }
                                        → 201 TrustPolicy
DELETE /v1/admin/trust/{id}             → 204
```

```json
{ "id": "a1", "name": "release-bot", "lockboxId": "lb1", "enabled": true,
  "createdAtMs": 1700000000000,
  "trust": [ { "id": "t1", "agentId": "a1", "issuer": "https://token.actions.githubusercontent.com",
               "claims": { "repository": "acme/deploy", "ref": "refs/heads/main" },
               "fixedAudience": false, "clientKind": "agent", "createdAtMs": 1700000000000 } ] }
```

A trust policy names at least one claim: one with none would admit every
token the issuer mints. `fixedAudience` (default `false`) is for a platform
that cannot set a token's audience (see "Verification"). `clientKind`
(default `agent`) is what a matching token may associate as; `sidecar` is a
policy for the sidecar that resolves values for the agent, matched to the
sidecar's own workload identity, and `extension` is a 400. An agent switched
off is refused at its clients' next request.

### Rules ✅

```
GET    /v1/admin/rules         → [ { lockboxId, rule } ]
POST   /v1/admin/rules         { lockboxId?, rule: Rule }   → 201 { lockboxId, rule }   | 409 id taken
DELETE /v1/admin/rules/{id}    → 204
```

`rule` is `rowel_broker::policy::Rule` as it serializes, with its own `id`;
`lockboxId` absent or `null` is a rule for every lockbox. The server stamps
`createdAtMs` and makes `createdBy` `{ "admin": "server" }`, whatever the
body says.

### Audit export ✅

```
GET /v1/admin/audit/export ?since&until&lockbox&actor   → 200, application/x-ndjson
```

One JSON object per line, in `(lockboxId, writerId, seq)` order: the
lockbox's audit row as its chain holds it, with the hashes in lowercase hex,
so each `(lockboxId, writerId)` run verifies as the lockbox does. `since` is
inclusive and `until` exclusive, both ms; `lockbox` is a lockbox id; `actor`
matches `actorId`.

```json
{ "lockboxId": "lb1", "id": "…", "writerId": "…", "seq": 7, "tsMs": 1700000000000,
  "prevHash": "<64 hex>", "hash": "<64 hex>", "actorKind": "agent", "actorId": "a1",
  "actorName": "release-bot", "runId": null, "actingFor": null, "grantId": "g1",
  "itemId": "i1", "target": "api.stripe.com/v1/charges", "mode": "header",
  "approval": "ask", "approver": null, "event": "used", "detail": null }
```

### Tenant events ✅

```
GET /v1/admin/events ?since&limit   → [ServerEvent]   (oldest first; limit 100 by default, at most 1000)
```

```json
{ "id": "e1", "tsMs": 1700000000000, "event": "lockbox_created", "memberId": "m-ada",
  "deviceId": null, "detail": "{\"lockboxId\":\"lb1\",\"name\":\"Deploy\"}" }
```

What happened at the tenant rather than inside one lockbox, kept apart from
the lockboxes' audit chains (which sync to devices): `signed_in`,
`device_enrolled`, `device_approved`, `device_revoked`, `member_added`,
`member_updated`, `lockbox_created`, `lockbox_updated`, `lockbox_member_set`,
`lockbox_member_removed`, `agent_added`, `agent_updated`, `agent_removed`,
`trust_added`, `trust_removed`, `rule_added`, `rule_removed`. `memberId` is
who acted; `deviceId` the device the event is about; `detail` compact JSON
naming what else it touched. No token or key is ever in one.

## What a conforming server must never do

- Hand a wrapped master to a device that is not approved — not to a pending
  one, not to a revoked one, not to a session that never enrolled.
- Store a device's pack as it came. The server opens and merges it; a pack
  it cannot open is refused, never kept.
- Accept a pack `PUT` without a precondition, or answer a stale `If-Match`
  with anything but 412.
- Serve a value to an agent without an active grant, or anything at all
  from a frozen lockbox.
- Log a secret, a TOTP code, passkey material, a bearer or refresh token, a
  workload token, or a full URL with its query string. The audit row carries
  the decision and the target's scheme, host and path; reasons are cut at
  300 characters.
- Listen for members or agents on plain `http://` or `ws://` anywhere but a
  loopback address. The app refuses the URL; a server should not offer it.
- Let a person stand in for a missing or failed workload proof on
  `/v1/agent`: the dialog is a device's; the server's answer is the policy's.
