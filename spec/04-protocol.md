# 04 — The agent protocol

## 1. One channel, three kinds of client

Rowel already has a sealed, authenticated request/reply channel: the browser
extension speaks KeePassXC-Browser's protocol to the app over a local socket,
every message a NaCl box under per-session keys, every reply nonce-bound to its
request, every privileged action gated on an association the user approved. It
has tests, a mock host, a native-messaging relay, and a way to push a clear,
content-free signal to every connected client.

The agent protocol is that channel with three additions: a client says what
*kind* of thing it is, a client of kind `agent` has a vocabulary that never
returns a secret, and the channel can run over a WebSocket to a server as well
as over the local socket. The MCP server, the Claude Code hook, the CLI and the
egress proxy are all thin clients of it. There is no second protocol to secure.

```
   ┌──────────────┐  native msg   ┌────────┐  socket   ┌──────────────────┐
   │ extension    │──────────────▶│ relay  │──────────▶│                  │
   └──────────────┘               └────────┘           │   broker (Host)  │
   ┌──────────────┐  socket / named pipe               │                  │
   │ agent (MCP,  │──────────────────────────────────▶ │  Tier 1: the app │
   │ hook, CLI)   │                                    │  Tier 2: server  │
   └──────────────┘                                    │                  │
   ┌──────────────┐  WebSocket + TLS (Tier 2/3)        │                  │
   │ remote agent │──────────────────────────────────▶ │                  │
   │ remote ext.  │                                    └──────────────────┘
   └──────────────┘
```

## 2. What an agent can say

An agent can introduce itself, ask for a grant, wait for a pending grant, end a
grant, and ask what it is allowed to see. It cannot list the vault, read an
item, or get a TOTP code. The reply to "may I log into github.com?" is *granted*,
*denied*, or *pending*, never a password.

A sidecar (the CLI or proxy that performs `header` and `env` modes) is a separate
kind with one more verb: resolve a placeholder under an active grant. It is the
only client besides the extension that ever receives a value. The `rowel`
binary holds two client identities for this reason: `rowel mcp` is an agent,
`rowel run` and `rowel proxy` are sidecars, and the model-facing surface can
never resolve.

## 3. Placeholders

Agents refer to secrets by name, never by value:

```
rowel://<lockbox>/<item>/<field>

  lockbox  the lockbox's id, or its registry name as a slug
  item     the item's id, or its title as a slug (unique within the lockbox)
  field    password | otp | apiKey | privateKey | body.<KEY> (one env variable)
           | number | cvc   (cards: step_up floor applies)
```

A placeholder is not a capability. Knowing the name of a secret buys nothing
without a grant that covers it, and `list-visible` only ever returns names
policy marks as visible to that agent.

*As built in I3* (`rowel_broker::reference`): `Reference::parse` is strict —
the exact scheme, exactly three non-empty segments, a field the protocol
names (`body.` needs a key), no query or fragment anywhere; what is not a
placeholder is none rather than nearly one. `lockbox` and `item` are opaque
to the parser; the host resolves them (`03`). A slug is the title
lowercased with every run of non-alphanumerics collapsed to one `-`
(`reference::slug`: "Stripe (live)" → `stripe-live`); two items with the
same slug are named by neither, and ids always work.

---

### Technical details

**Framing and crypto (✅, unchanged).** `src-tauri/src/browser/frame.rs`
(length-prefixed JSON frames), `protocol.rs` (`Session`, `change-public-keys`,
nonce increment, `error_reply`), `actions.rs` (`Connection<H: Host>::handle`).
These move to the `rowel-broker` crate verbatim (`11`, PR 1). Over WebSocket a
binary message carries exactly one frame's JSON body; the length prefix is
dropped because the transport frames for us.

**Hello and kinds (🟡).** `associate` and `test-associate` accept an optional
`hello` object inside the sealed message:

```json
{ "action": "associate", "key": "<id pubkey b64>", "idKey": "...",
  "hello": { "kind": "agent", "platform": "claude-code", "version": "1.2.0",
             "capabilities": ["grants", "placeholders"] },
  "proof": { "kind": "oidc", "token": "<jwt>" } }
```

`kind` defaults to `extension` when absent, so the unmodified KeePassXC-Browser
extension keeps working; a `hello` that does not parse is `AssociationFailed`;
`platform`, `version` and each capability are clamped to a short length. The
reply to both actions gains `brokerCapabilities: ["grants","signals"]` and,
for a kind other than `extension`, `clientId` (the identification key), so a
reconnecting CLI, which only ever `test-associate`s, learns them too.

**Vocabularies (✅ I2).** The kind that gates an action is the stored
record's, checked right after the association check
(`ClientKind::permits`, `Connection::admit`), with `NotPermittedForKind` for
a known client at the wrong door:

| action | extension | agent | sidecar |
| --- | --- | --- | --- |
| `get-databasehash`, `generate-password`, `lock-database` | ✓ | ✓ | ✓ |
| `get-logins`, `get-totp`, `set-login`, `passkeys-register`, `passkeys-get`, `get-signals`, `report-exposure` | ✓ | ✗ | ✗ |
| `request-grant`, `wait-grant`, `end-grant`, `grant-status`, `list-visible` | ✗ | ✓ | ✓ |
| `resolve` (✅ I3) | ✗ | ✗ | ✓ |

A test walks every secret-carrying action as an agent, and the extension's
actions as a sidecar, and asserts the clear refusal carries no `entries`,
`password`, `totp`, `response` or `value`. `brokerCapabilities` is
`["grants", "signals", "resolve"]` since I3: an action added later adds a
word, so a client can tell a host that serves it from one that will refuse.

**Agent actions (✅ I2 but `resolve`).** All inside the sealed channel, all
after association. Names follow the existing kebab style; the selector,
target and mode shapes are the serde forms of `rowel_broker::grant`.

| action | request | reply | notes |
| --- | --- | --- | --- |
| `request-grant` | `{ runId ≤ 64, reason? ≤ 300, items: "lockbox" \| {ids:[…]} \| {tag} \| {kind}, targets: [ {origin} \| {http:{host,path,methods}} \| {command} ], modes: ["fill",…] }` | `GrantReply`: `{ grantId, state, expiresAtMs, retryAfterMs, uses, items: [ { id, title, username } ] }` | Tier 1 blocks for the dialog (≤ 60 s) and answers the final state inside the sealed reply; `pending` + `retryAfterMs` is a *state* there (only the extension's `agentMode` fill path turns it into the `GrantPending` refusal). A `runId` over 64 chars, a missing or malformed selector, an empty `targets` or `modes` is `GrantDenied`; `reason` is clamped. `acting_for` comes with Tier 2. |
| `wait-grant` | `{ grantId, timeoutMs ≤ 120000 }` | `GrantReply` | Long poll on the server; in Tier 1 it is `grant-status`. |
| `grant-status` | `{ grantId }` | `GrantReply` with `uses` | The grant's own client only; another's id is `GrantDenied`. |
| `end-grant` | `{ grantId }` | `{ ok: true }` | Own grant only. Triggers session end (`05`). |
| `list-visible` | `{ lockbox? }` | `{ items: [ { ref: "rowel://<lockbox id>/<item id>", title, kind, fields: [...] } ] }` | In Tier 1: the items this client's *own* live rules admit (`AgentSelector::Ids` naming its key) and the items of its active grants — never the baseline rule, which would make the whole lockbox's titles visible to any agent let in. Never secrets. |
| `resolve` | `{ grantId, ref: "rowel://…", context: { target: { http: { host, path, method } } \| { command: "<full line>" } } }` | `{ value }` | **sidecar kind only (✅ I3).** The target is concrete — one request, one command line — and its kind says the mode (`http` → `header`, `command` → `env`; `ResolveTarget::mode`). The host checks the grant covers the item at that place in that mode, the lockbox and the field, and the sidecar binding (`03`), and counts a use. A `ref` or `context` that does not read, and every check that fails, is `GrantDenied` in one word; a missing `grantId` is `NoValidUuidProvided` like the other grant actions; a non-sidecar is `NotPermittedForKind`. The value lives in the sealed reply exactly as long as a fill's password does. |

`request-grant` since I3 takes `targets` of any kind — `{origin}` for
`fill`/`passkey`, `{http: {host, path, methods}}` for `header`, `{command}`
for `env` — and every target must fit every mode, else `GrantDenied`
(`no_target` on the row); the host turns each into probes for policy (`03`).

`Code` variants (`protocol.rs`): `GrantDenied = 40`, `GrantPending = 41`,
`GrantExpired = 42`, `NotPermittedForKind = 43`, `PolicyRequiresApproval = 44`,
`RateLimited = 45`, `AgentsDisabled = 46` (the app's Agents switch is off).
Numbers above KeePassXC's range so an upstream merge never collides;
`Code::from_number` maps a raw `errorCode` back for clients.

**Host trait (✅ I3 as built).**

```rust
pub trait Host {
    fn enabled(&self) -> bool;
    fn database_hash(&self) -> Option<String>;
    fn clients(&self) -> Vec<Client>;
    fn associate(&self, key: &str, who: &Hello) -> Result<Client, Code>;
    fn client_seen(&self, _key: &str) {}
    fn logins_for(&self, page_url: &str, host: &str) -> Vec<Login>;
    fn totp(&self, id: &str) -> Option<String>;
    fn generate_password(&self) -> Option<String>;
    fn save_login(&self, id: Option<&str>, url: &str, host: &str, username: &str, password: &str) -> Result<(), Code>;
    fn passkey_register(&self, client: &Client, registration: Registration) -> Result<Value, Code>;
    fn passkey_get(&self, client: &Client, assertion: Assertion) -> Result<Value, Code>;
    fn lock(&self);
    fn unlock_requested(&self);
    fn request_grant(&self, client: &Client, req: GrantRequest) -> Result<GrantReply, Code>;
    fn wait_grant(&self, client: &Client, id: &str, timeout_ms: u64) -> Result<GrantReply, Code>;
    fn grant_status(&self, client: &Client, id: &str) -> Result<GrantReply, Code>;
    fn end_grant(&self, client: &Client, id: &str) -> Result<(), Code>;
    fn visible_items(&self, client: &Client, lockbox: Option<&str>) -> Vec<VisibleRef>;
    fn take_signals(&self, client: &Client) -> Vec<Signal>;
    /// A yes is a use; for a grant issued to another client it is also the
    /// `served_to` claim (`05`).
    fn grant_covers(&self, client: &Client, grant_id: &str, item_id: &str, mode: Mode) -> bool;
    fn report_exposure(&self, client: &Client, grant_id: &str);
    /// I3. Defaults to `Err(GrantDenied)`, so a host that has not implemented
    /// the sidecar's verb serves nothing by omission.
    fn resolve(&self, client: &Client, grant_id: &str, reference: &Reference, ctx: &ResolveContext) -> Result<Zeroizing<String>, Code>;
}
```

`AppHost` implements them against the open lockbox; `OrgHost` (Tier 2) against
an org lockbox. The `Mock` in the broker crate's `testing` module has the same
methods, so the protocol tests cover grants without an app,
`Mock::grant_covers` is the reference for the `served_to` claim, and
`Mock::resolve` (secrets by item and field in the one lockbox `LOCKBOX`,
grants carrying the targets they were requested with) is what the CLI's
tests resolve against.

**The client half (✅ I2, `rowel_broker::client`).** The extension's side of
the sealed channel as production code: `Channel` (our X25519 key pair, the
exchange, `seal`/`open` with the extension's checks: nonce + 1, `success`,
`action`), `Refused` (a clear refusal: `Code` or raw number, text,
`retryAfterMs`), `Transport` (one frame out, one in — a socket, a pipe, or an
in-process `Connection` under `test-support`) and `Peer` (`connect` runs the
exchange, `call(action, message)` seals, sends, opens). The test `Extension`
is rebuilt on it, so there is one implementation; the CLI is its first
production user.

**Signals to the extension (🟡).** The host already pushes `database-locked`
and `database-unlocked` to every connected client **in the clear and
unsolicited** (`server.rs::signal`, one queue thread per connection sharing
the connection's writer); the extension handles them in
`client.js::onNativeMessage` before generic reply handling, and the Safari
shim treats them as a hint to poll (`HostConnection.swift::isSignal`). Grant
signals carry an item title, which is personal, so they are not sent that way.
Instead:

1. the host pushes a third clear signal, `{"action":"rowel-signal"}`, with no
   other fields, through the same queue;
2. the extension answers it with a sealed `get-signals` request, and the host
   replies `{ signals: [...] }` with the queued `Signal`s for that client,
   each tagged by `signal`: `fill-grant { grantId, origin, item: { id, title,
   username }, autoSubmit, expiresAtMs }` and `grant-ended { grantId,
   origins }` (`rowel_broker::signal::Signal`; `ROWEL_SIGNAL` is the clear
   wake-up's bytes). There is no `set-agent-mode` signal: the extension
   derives agent mode from the fill grants it holds (`05`), which keeps the
   host out of a per-profile setting it cannot see.

`get-signals` is in the extension's vocabulary only, and a queue is drained
without an access check: a `grant-ended` has to reach the extension even
after the Agents switch went off. No polling. An open native-messaging port keeps the MV3 service worker alive
(Chrome 105 and later), so the wake-up reaches a live worker. If the port is
down, the extension reconnects as it does today (key exchange, then
`test-associate`) and sends one `get-signals` right after, so a signal queued
while it was away is still delivered. Fork changes: `client.js::onNativeMessage`
routes `rowel-signal` to the new `grants.js`; `HostConnection.swift::isSignal`
adds the action. One request, one reply stays the rule for everything that
carries content.

**Extension-side actions (🟡).** The extension keeps `get-logins`, `get-totp`,
`set-login`, passkeys. Two changes:

- `get-logins` accepts two optional fields beside `url`, `submitUrl`,
  `httpAuth` (`keepass.js::retrieveCredentials` builds the message). With
  `agentMode: "true"` the extension itself is the requester: the host turns
  the page's candidates into a `GrantRequest` (mode `fill`, target the page's
  origin), runs policy and the ask, and replies with the one admitted entry
  plus a top-level `grantId`; a refusal is `GrantDenied`, `GrantPending` (the
  consent slot is busy) or `AgentsDisabled`. With `grantId` (an agent's grant
  from `request-grant`, phase I2) the host serves only the items that grant
  covers. Without either, the behaviour is today's. While the extension is in
  agent mode (`05`, `06`) it *must* send one of the two.
- `get-totp` accepts an optional `grantId` and, when present, serves the code
  only if the grant covers that login.
- `report-exposure { grantId }`: the content script found a filled secret
  showing in the page's text; the host ends the grant and writes an `exposed`
  audit row.
- `get-signals` as above.

**Transports.**

- Local: unchanged (`browser.sock` owner-only, Windows named pipe, the macOS
  App Group socket for Safari). Agents connect directly; no native-messaging
  relay needed. Socket discovery (`browser::socket_name`, `root_dir`) moves to
  the broker crate so the CLI finds the same socket.
- Remote (Tier 2/3): `wss://<server>/v1/agent`. TLS protects the transport; the
  NaCl channel still runs inside it so the server's TLS terminator never sees
  plaintext requests, and so the same code serves both transports. The
  association's `proof` replaces the human dialog (`02`).
- *As built in I5.* `rowel serve` listens on the data directory's socket
  and on `ws://host:port` (or `wss://` with TLS from PEM files); a binary
  message carries exactly one frame's JSON body, and the clear signals
  (`database-locked`, `database-unlocked`, `rowel-signal`) arrive as binary
  messages too. The CLI's `transport` reaches it when `ROWEL_BROKER` names
  it (`ROWEL_BROKER_CA` adds a CA), and on either transport skips the clear
  signals when reading a reply, which the socket path did not do before.
  There is no `proof` yet: a client reaching the WebSocket is let in only
  through an association ask answered with `rowel approve`.
- Remote extension (cloud browser, Tier 2/3, phase 4): upstream
  KeePassXC-Browser already has a WebSocket transport to `ws://localhost:<port>`
  (`client.js::connectToWebSocket`, same `onNativeMessage` handler). The
  remote transport generalises that URL to `wss://…/v1/extension`, enabled by
  an **enrollment code** the agent platform obtained via `request-grant` with
  mode `fill` and passed to the browser session. The code is one-time, bound
  to the grant, and expires with it.

**Thin clients (🟡, the `rowel-cli` binary, `11`).** The crate is
`src-tauri/crates/cli` (`rowel-cli`); its binary ships beside the app as
`rowel-cli` and is reached as `rowel` through the symlink Settings installs
(`06`). `status` and `mcp` landed in I2; the rest is I3.

- `rowel mcp` (✅ I2) — a stdio MCP server (newline-delimited JSON-RPC 2.0)
  exposing tools that map 1:1 onto the agent actions:
  `rowel_request_login(site, reason?)`, `rowel_wait(grant_id)`,
  `rowel_list_visible()`, `rowel_end(grant_id)`; `rowel_request_secret(ref,
  mode, target, reason?)` is listed only from I3, when the host can serve a
  header or env grant — a listed tool that always fails teaches an agent to
  retry it. A
  tool result is the opened reply's named fields only (`grantId`, `state`,
  `expiresAtMs`, `retryAfterMs`, `uses`, `items` / `ok`), never a value, by
  construction: the client is of kind `agent`, with its own key, associated
  lazily at the first tool call so `initialize` and `tools/list` work with
  the app closed. `runId` is minted once per process. A `pending` with
  `retryAfterMs` (the consent slot was busy; no grant was issued) is asked
  again by the server itself, for up to a minute, before the agent sees it;
  `rowel_wait` is for a `pending` without one, a routed approval (Tier 2).
  `list-visible` answers empty while either switch is off, as every use is
  refused. `rowel status` says whether the app is reachable, the vault open
  and the key associated.
- `rowel_request_secret(ref, mode, target, reason?)` (✅ I3) — the agent's
  half of `env` and `header` mode: `ref` a placeholder, `mode` `env` with
  `target` the full command line as `rowel run` writes it (`05`), or `header`
  with `target` `[METHOD[,METHOD] ]https://host/path-glob` (path defaults to
  `/*`; a query string is refused). The request is `{ items: {ids: [the
  ref's item]}, targets: [the one target], modes: [mode] }`; the result is
  the grant fields plus `use`, the one line that uses it — the `rowel run
  --grant …` invocation, or the `X-Rowel-Grant` header for the proxy.
- `rowel hook pre|post` (✅ I3) — Claude Code / Codex / Cursor hooks: *pre*
  rewrites a plain `NAME=rowel://… cmd args` into `rowel run --env … -- cmd
  args` (so the agent's own shell never sees the value); *post* is the CLI's
  exposure check (`05`).
- `rowel run`, `rowel redact`, `rowel hook post` and `rowel proxy` are the
  sidecar, client kind `sidecar`, with a second key (`keys::Role::Sidecar`,
  `sidecar.json` beside `agent.json`, `0600`; the OS secure store per Q9).

**Versioning.** The host reports `VERSION = "2.7.7"` for KeePassXC
compatibility; agent capabilities are negotiated through `hello.capabilities`
and `brokerCapabilities`, not the version string. Add a new capability string
when adding an action; never change an existing action's shape.
