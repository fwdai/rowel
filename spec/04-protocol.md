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

  lockbox  the lockbox's slug or id
  item     the item's slug (title, kebab-cased, unique within the lockbox) or id
  field    password | otp | apiKey | privateKey | body.<KEY> (one env variable)
           | number | cvc   (cards: step_up floor applies)
```

A placeholder is not a capability. Knowing the name of a secret buys nothing
without a grant that covers it, and `list-visible` only ever returns names
policy marks as visible to that agent.

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
extension keeps working. The reply gains `brokerCapabilities` and, for agents,
`clientId`.

**Agent actions (❌).** All inside the sealed channel, all requiring an
association of kind `agent` (or `sidecar` where noted). Names follow the
existing kebab style.

| action | request | reply | notes |
| --- | --- | --- | --- |
| `request-grant` | `{ runId, reason?, items: selector, targets: [...], modes: [...], acting_for?: delegation }` | `{ grantId, state: "active"\|"pending"\|"denied", expiresAtMs?, retryAfterMs?, items: [ { id, title, username? } ] }` | Tier 1 blocks for the dialog (≤ 60 s) and answers the final state; `pending` + `retryAfterMs` only when another ask holds the consent slot. Tier 2 answers `pending` for routed approvals. `items` are the admitted candidates' non-secret fields. |
| `wait-grant` | `{ grantId, timeoutMs ≤ 120000 }` | same as above | Long poll on the server; in Tier 1 it returns at once with the current state. |
| `end-grant` | `{ grantId }` | `{ ok }` | Triggers session end (`05`). |
| `list-visible` | `{ lockbox? }` | `{ items: [ { ref: "rowel://…", title, kind, fields: [...] } ] }` | Only items a rule marks `visible` for this agent. Never secrets. |
| `grant-status` | `{ grantId }` | `{ state, uses, expiresAt }` | |
| `resolve` | `{ grantId, ref: "rowel://…", context: { target } }` | `{ value }` | **sidecar kind only.** Checks the grant covers `ref`, the mode is `header` or `env`, `context.target` matches, and the sidecar binding (`03`). Increments `uses`. Refused with `NotPermittedForKind` for any other kind. |

New `Code` variants (`protocol.rs`): `GrantDenied = 40`, `GrantPending = 41`,
`GrantExpired = 42`, `NotPermittedForKind = 43`, `PolicyRequiresApproval = 44`,
`RateLimited = 45`, `AgentsDisabled = 46` (the app's Agents switch is off).
Numbers above KeePassXC's range so an upstream merge never collides.

**Host trait additions (❌).**

```rust
pub trait Host {
    // ... existing methods unchanged ...
    fn request_grant(&self, client: &Client, req: GrantRequest) -> Result<GrantReply, Code>;
    fn wait_grant(&self, client: &Client, id: &str, timeout: Duration) -> Result<GrantReply, Code>;
    fn end_grant(&self, client: &Client, id: &str) -> Result<(), Code>;
    fn visible_items(&self, client: &Client, lockbox: Option<&str>) -> Vec<VisibleItem>;
    fn resolve(&self, client: &Client, id: &str, reference: &Reference, ctx: &ResolveContext)
        -> Result<Zeroizing<String>, Code>;
    /// The sealed signals queued for this client since it last asked (see below).
    fn take_signals(&self, client: &Client) -> Vec<Signal>;
}
```

`AppHost` implements them against the open lockbox; `OrgHost` (Tier 2) against
an org lockbox. The `Mock` in `browser/tests.rs` grows the same methods so the
protocol tests cover grants without an app.

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
   replies with the queued `Signal`s for that client:
   `fill-grant { grantId, origin, item: { id, title, username }, autoSubmit,
   expiresAt }`, `grant-ended { grantId, origins }`, `set-agent-mode { on }`.

No polling. An open native-messaging port keeps the MV3 service worker alive
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
- Remote extension (cloud browser, Tier 2/3, phase 4): upstream
  KeePassXC-Browser already has a WebSocket transport to `ws://localhost:<port>`
  (`client.js::connectToWebSocket`, same `onNativeMessage` handler). The
  remote transport generalises that URL to `wss://…/v1/extension`, enabled by
  an **enrollment code** the agent platform obtained via `request-grant` with
  mode `fill` and passed to the browser session. The code is one-time, bound
  to the grant, and expires with it.

**Thin clients (❌, `rowel` CLI binary, `11`).**

- `rowel mcp` — a stdio MCP server exposing tools that map 1:1 onto the agent
  actions: `rowel_request_login(site, reason)`, `rowel_request_secret(ref,
  mode, reason)`, `rowel_wait(grant_id)`, `rowel_list_visible()`,
  `rowel_end(grant_id)`. Tool results never include values. Client kind
  `agent`, with its own key.
- `rowel hook` — Claude Code / Codex / Cursor hooks: *pre-tool* rewrites a shell
  command containing `rowel://` placeholders to run under `rowel run`
  (requesting a grant in `env` mode, so the agent's own shell never sees the
  value); *post-tool* pipes output through `rowel redact` (`05`).
- `rowel run --grant <id> -- <cmd>` and `rowel proxy` are the sidecar, client
  kind `sidecar`, with a second key. Both keys live in the OS secure store
  where available, else `0600` files in the config directory (Q9).

**Versioning.** The host reports `VERSION = "2.7.7"` for KeePassXC
compatibility; agent capabilities are negotiated through `hello.capabilities`
and `brokerCapabilities`, not the version string. Add a new capability string
when adding an action; never change an existing action's shape.
