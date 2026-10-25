# Rowel for Agents — implementation plan (checklist)

This is the living checklist for building what the spec describes. It is
organised as **iterations**. Every iteration ends with a working, shippable
system that is a strict superset of the one before; stopping after any of them
leaves a coherent product with the remaining features absent, not broken.
Architecture is not repeated here: each item points at the spec section that
defines it.

**How to use this file**

- Work one iteration at a time, in order. Inside an iteration, items are listed
  in dependency order; parallel work is noted.
- Tick an item (`- [x]`) when it is merged with its tests and the gates are
  green. Tick the iteration's header box when every item is ticked and the
  acceptance demo has been run and recorded.
- When the code settles on something different from the spec, update the spec
  in the same PR and record the decision in the project context. Do not let
  this file and the spec drift.
- Gates for every PR: `bun run typecheck` · `bun run lint` (0 warnings) ·
  `bun run test` · `cargo test` · `cargo clippy -D warnings`; for extension
  changes also `bun run extension:lint && extension:test && extension:build`.
- Before starting an iteration, settle the open questions it lists under
  *Decide first* (`10-open-questions.md`).

## Reuse strategy

One core, two binaries, no second implementation of anything.

- **`rowel-core`** (`src-tauri/crates/core`, exists): crypto, store, entries,
  passkeys, OTP, host matching. Unchanged by this plan.
- **`rowel-broker`** (new in I0, extracted from `src-tauri/src/browser/`): the
  sealed channel, the `Host` trait, grants, policy, audit, and the lockbox-side
  functions every host shares. Spec `00 §4`, `04`, `06`.
- **The desktop app** embeds the broker crate and *is* the broker on a laptop
  (`AppHost`). It is deliberately **not** split into a UI plus a daemon: it
  already holds the keys in one trusted process, and a daemon would move them
  into a second process and add an IPC trust boundary for no security gain
  (`10`, Q26).
- **The `rowel` CLI** (new in I2) embeds the same crates. Its client
  subcommands (`mcp`, `hook`, `run`, `proxy`, `redact`) talk to whichever
  broker is reachable. Its **`serve`** subcommand (I5) is the headless broker:
  on a developer's VM it serves a device-held lockbox without a GUI; in
  **server mode** (I7) it is the self-hosted Rowel Server; inside a Cloudflare
  Container (I9) it is Rowel Cloud. Same binary, same code path, more
  capabilities switched on. Spec `07`, `08`.
- **The sync engine** is reused as the device↔server transport (`07`), the
  **consent dialog** is reused for every ask in every tier (`03`), and the
  **extension** gains modes rather than forks (`05`).

```
  laptop                 dev VM / CI box          company               Rowel Cloud
  ──────                 ────────────────         ───────               ───────────
  Rowel app (AppHost)    rowel serve              rowel serve --server  rowel serve --server
  + rowel mcp/run/proxy  + rowel mcp/run/proxy    + members, OIDC,      in a Cloudflare
                                                   policy, audit        Container, behind a
                                                                         Worker + Durable Object
  I1–I4                  I5                       I6–I8                 I9–I11
```

## Iteration map

| # | Deliverable if we stop here | Tier | Spec | Needs |
| --- | --- | --- | --- | --- |
| I0 | App unchanged; broker code in its own crate | 1 | 06, 11 §2 PR1 | — |
| I1 | Agent mode for agentic browsers: every fill is an approved grant | 1 | 01, 03, 05 §2, 06 | I0 |
| I2 | Coding agents use web logins through MCP; sessions end with the grant | 1 | 02, 04, 05, 06 | I1 |
| I3 | API keys and env secrets for agents via the sidecar; nothing in the transcript | 1 | 03, 04, 05 §4–5 | I2 |
| I4 | Readable, synced audit; Tier 1 documented and released | 1 | 09, 11 §2 PR8 | I3 |
| I5 | `rowel serve`: the broker without the GUI, on any box | 1½ | 06, 07 | I3 |
| I6 | The app can hold org lockboxes and answer remote approvals | 2 (app side) | 01, 02, 07 | I4 |
| I7 | Self-hosted Rowel Server, single node, GitHub Actions agents | 2 | 02, 03 §4, 07 | I5, I6 |
| I8 | Tier 2 at scale: admin in the app, routing, rotation, exports | 2 | 03, 07, 09 | I7 |
| I9 | Rowel Cloud MVP on Cloudflare: hosted Tier 2 with billing | 3 | 08 | I7 |
| I10 | Push approvals, free-tier relay, SSO/SCIM, SIEM, residency | 3 | 08 | I9 |
| I11 | Remote browsers, OAuth tokens, more identities, policy as code | 4 | 02, 04, 05 | I8, I10 |

---

## I0 — Extract the broker crate  `[ ]`  *(in review: PR #652, branch `refactor/broker-crate`)*

**Deliverable.** Nothing visible changes. The protocol, framing, `Host` trait,
passkey glue and socket helpers live in `rowel-broker` with their tests, ready
to be linked by the app, the CLI and the server. Spec `06` (Crate extraction),
`11 §2 PR 1`.

- [ ] Create `src-tauri/crates/broker`; move `protocol.rs`, `frame.rs`,
      `actions.rs`, `passkeys.rs`, the socket-name and root-dir helpers, and
      the protocol tests; `passkeys.rs` imports `rowel_core::passkey` directly.
- [ ] Re-export from `src-tauri/src/browser/mod.rs`; no call-site changes.
- [ ] Factor the lockbox-side functions `AppHost` uses into the broker crate
      as functions over `SqliteStore` + `PayloadCipher`, so a headless host (I5)
      and `OrgHost` (I7) call the same code. Behaviour unchanged. *Outcome:*
      `save_login_in` moved (`rowel_broker::lockbox`); `logins_in` and the TOTP
      lookup stayed in the app because code generation lives in the app's
      `commands::generator` (core's `otp` only parses seeds). They move with
      the first item of I5.
- [ ] Workspace member in `src-tauri/Cargo.toml`; crate list in `docs/development.md`.
- [ ] **Acceptance:** `cargo test -p rowel-broker` passes the moved tests; app
      builds on macOS, Windows, Linux, iOS; the extension connects and fills.

---

## I1 — Agent mode for agentic browsers  `[ ]`  *(in review: PR #653, branch `feat/agent-mode`, stacked on #652)*

**Deliverable.** A person using Claude in Chrome, Comet or any agentic browser
turns on agent mode in the Rowel extension; from then on every fill is a grant
the person approves in Rowel's dialog, the extension's own UI is hidden, a page
that echoes a secret is cleared, and a standing rule removes the dialog for a
site for N days. No new clients, no CLI yet. Spec `01` (agent-access flag),
`03`, `05 §2`, `06`.

*Decide first:* none (names in Q3 can stay provisional).

- [ ] Per-lockbox agent-access flag: `Workspace.agents` in the registry, a
      command, and the toggle with its plain-language copy in lockbox settings
      (`01`, `06 §3`).
- [ ] `rowel_broker::{grant, policy}`: `Grant`, `Rule`, `Target`, `evaluate`,
      `Floors`, `Limits`, with unit tests for specificity, both floors, rate
      limits and deny-by-default (`03`).
- [ ] `rowel_broker::audit::Writer` and the `audit` table migration in core;
      rows written for every grant event (`09`, not yet synced or shown).
- [ ] `Answer::Grant` in the consent slot; `agent_grant` event;
      `BrowserConsent/GrantDialog/` with once · this run · N days · deny, item
      picker, step-up via biometrics or master password (`03`, `06`).
- [ ] Rules and standing grants as sealed `meta` JSON; Settings › Browser &
      agents lists rules and active grants with revoke (`03`, `06`).
- [ ] `settings.browser.agents.enabled` (default off) gating all of the above.
- [ ] Extension: agent-mode toggle in the popup, `agentMode`/`grantId` on
      `get-logins` and `get-totp`, in-page UI and autocomplete suppressed,
      exposure check after fill and after submit with clear-on-failure (`05 §2`).
- [ ] Host: in agent mode a fill runs the grant flow (policy → ask → serve the
      one admitted item); outside agent mode behaviour is unchanged.
- [ ] Grant expiry via `timer::Timer` armed to the earliest deadline.
- [ ] Tests: broker protocol tests through `Mock`; Playwright fixture page for
      approved fill, denied fill, exposure; e2e spec for the dialog.
- [ ] Threat model: one paragraph noting that agent mode makes fills ask
      (`01`, full rewrite comes in I4).
- [ ] **Acceptance:** with agent mode on, reaching a login page in an agentic
      browser raises the dialog; approve fills, deny fills nothing; a 7-day
      rule fills without a dialog next time and is listed and revocable; the
      exposure fixture is cleared and logged.

---

## I2 — Agent clients and the MCP server  `[ ]`  *(in review: PR #655, branch `feat/agent-clients`, stacked on #653)*

**Deliverable.** Claude Code, Codex or Cursor configured with `rowel mcp` can
ask to log into a site; the person approves; the extension fills the open tab;
the grant ends and the site's cookies go with it. Tool results never contain a
secret. Spec `02` (local association), `04`, `05` (signals, session end), `06`
(CLI).

- [ ] `Client { kind, platform, created_at, last_seen_at }` with `Extension`
      default on read; `hello` on `associate`/`test-associate`; the associate
      dialog shows kind and platform; Settings shows kinds;
      `browser_forget_client` revokes either (`02`, `11 §2 PR 2`).
- [ ] Agent actions `request-grant` (blocking ask, `pending` + `retryAfterMs`
      when the slot is busy), `wait-grant`, `end-grant`, `grant-status`,
      `list-visible`; new `Code`s; implemented in `Mock` and `AppHost` (`04`).
- [ ] Signals: clear `rowel-signal` push through `server::signal`, sealed
      `get-signals`, `fill-grant` queued for every extension client and
      claimed by the first one served (`Grant.served_to`); `get-signals` once
      after reconnect; Safari `isSignal` learns the action (`04`).
- [ ] Extension `grants.js`: handles the wake-up, finds the tab by origin,
      fills through the existing path with `grantId`, agent mode on for the
      grant's lifetime when it was off (derived in the extension; no
      `set-agent-mode` signal) (`05 §2`).
- [ ] `src-tauri/crates/cli` (`rowel-cli`): `status`, `mcp` (stdio MCP, the
      four tools the desktop host can serve — `rowel_request_secret` is
      listed from I3 — no values in results, agent key in a `0600` file for
      now) (`04`).
- [ ] Session end: `grant-ended` on `end-grant` and on expiry; cookie deletion
      per registrable domain with the existing `cookies` permission; dialog
      notice (`05`).
- [ ] Passkey ask shows *requested by …* and writes an audit row (`05 §3`).
- [ ] Bundle the CLI in the desktop installers; *Install command line tool*
      in Settings › General (`06`).
- [ ] Tests: broker tests for every action and for "no reply carries a secret
      field"; an MCP harness test; Playwright for grant-driven fill, second
      client refused, cookie deletion in Chromium and Firefox.
- [ ] **Acceptance:** Claude Code asks for github.com; dialog; approve; the
      extension fills the open tab; the MCP result is `active` with no secret;
      after expiry the GitHub cookies are gone; a revoked agent's next call is
      refused.

---

## I3 — Secrets for commands and APIs: the sidecar  `[ ]`  *(in review: PR #657, branch `feat/agent-sidecar`, on `main`)*

**Deliverable.** A coding agent uses an API key or an `.env` value for one
command or one API call without the value ever appearing in its transcript,
its environment or the MCP results. Spec `03` (floors, binding), `04` (thin
clients), `05 §4–5` (header, env, redaction).

- [ ] `sidecar` client kind and the CLI's second key; `resolve` action refused
      for any other kind and bound to the grant id and the same lockbox (`03`, `04`).
      *Outcome:* `Host::resolve` with a default refusal; `AppHost` resolves
      under the session and agents locks (`03` "as built in I3"); the sidecar
      key is `keys::Role::Sidecar`.
- [ ] Placeholder resolution `rowel://<lockbox>/<item>/<field>`; `list-visible`
      returns refs and fields (`04 §3`). *Outcome:* `rowel_broker::reference`;
      names resolve by id or slug (lockbox: registry name; item: title).
- [ ] `rowel run --grant … -- cmd`: env mode, full-command-line targets, requests
      a grant when none is given, output through the redactor (`05 §5`).
      *Outcome:* as specified, plus `--env-file`, inherited `rowel://`
      variables, and the one `command::line` quoting rule (`05`).
- [ ] `rowel redact`: streaming exact-match matcher with encodings and carry-over
      window, as a library and a filter (`05`). *Outcome:* values under 6
      bytes are never matched; the filter resolves its values under a grant.
- [ ] `rowel hook`: pre-tool rewrite to the `rowel run` form, post-tool redact;
      documented configs for Claude Code, Codex and Cursor (`04`). *Outcome:*
      the post hook is an exposure check (resolve the command's own
      placeholders, scan the output, end the grant on a hit), not a rewrite of
      the output, which the platforms' Bash hooks cannot do (`05`, Q16).
- [ ] `rowel proxy`: path-prefix addressing, `X-Rowel-Grant`, header and body
      placement rules, response redaction, per-grant token bucket (`05 §4`).
      *Outcome:* loopback only; body placement gated on content type (JSON,
      form) since the grant has no placement field yet; `/http/` behind
      `--allow-http`.
- [ ] Policy floors for `env` and `header`; Settings lists header and env rules
      with their targets (`03`). *Outcome:* the baseline admits `header` and
      `env` at `ask`; `RuleRow`/`GrantRow` carry `mode` and `target`.
- [ ] CLI keys in the OS secure store on macOS and Windows (non-gated item in
      `secure_store.rs`), `0600` files elsewhere (Q9). *Outcome:* in the CLI
      crate's own `keys` backend (the app's `secure_store.rs` links Tauri),
      with a one-time move of an existing file.
- [ ] Tests: broker tests for `resolve` and binding; integration tests for
      `run`, `proxy` and `redact`; a rule for `node deploy.js*` must not match
      `node -e …`.
- [ ] **Acceptance:** Claude Code runs a Stripe CLI command through `rowel run`
      and calls the Stripe API through `rowel proxy`; the key appears nowhere
      the agent can read; `env` printing the value comes back redacted.

---

## I4 — Audit you can read, and Tier 1 released  `[ ]`  *(in review: PR #658, branch `feat/agent-audit`, stacked on #657)*

**Deliverable.** Vault Health shows what agents did, across the user's devices,
with a verifiable chain; the threat model and docs say what shipped. This
closes the free product. Spec `09`, `11 §2 PR 5 and PR 8`.

- [ ] `Snapshot.audit`, export and `INSERT OR IGNORE` merge, `vault_digest`
      including audit rows, retention with `pruned_through` (`09`). *Outcome:*
      as specified, except that every device reclaims every writer's aged
      prefix at pack time and only the chain's own writer writes the
      `pruned_through` note — a union merge hands a device its own pruned rows
      back otherwise, and iOS has no chain of its own (`09`, Q21).
- [ ] Vault Health › *Agent activity*: recent events, standing rules older than
      30 days, agents unseen for 90 days, *Verify log* (`09`). *Outcome:*
      `AgentActivity/` under the Vault Health section, desktop only, shown
      when the Agents switch is on or the log has something to show; commands
      `audit_agent_activity` and `audit_verify` in `commands/activity.rs`.
- [ ] `docs/threat-model.md`: the extension section reworded and a new Agents
      section per `01`; `docs/agents.md` user guide; README claims worded to
      what shipped.
- [ ] Release: extension version bump and store submissions; app release with
      notes; the spec's revision history updated. *Outcome:* extension 0.2.0
      in the PR; the store submissions and the app's Release workflow are the
      owner's manual steps after the merge (`docs/releasing.md`).
- [ ] **Acceptance:** two devices (the `two_devices.rs` harness) converge on
      the union of their audit logs and both chains verify; a device with only
      new audit rows pushes; an altered row breaks verification.

---

## I5 — `rowel serve`: the broker without the GUI  `[ ]`

**Deliverable.** The same crates run as a headless broker on a developer's VM,
a CI runner or a Linux box without a desktop: a device-held lockbox is unlocked
from a TTY or a key file, agents and sidecars connect over the local socket or
a WebSocket, standing rules serve them, and an `ask` is answered from a terminal
or times out. This is the binary the server (I7) and the cloud (I9) run. Spec
`06` (reuse), `07` (precursor to `OrgHost`).

- [ ] Move TOTP code generation (the `totp()` builder behind
      `commands::generator::generate_otp`, and the `totp-rs` dependency) into
      `rowel_core::otp`, route the app's command through it, then move
      `logins_in` and the TOTP lookup into `rowel_broker::lockbox` beside
      `save_login_in`. Behaviour unchanged; this is the seam I0 left open.
- [ ] `rowel-serve` library crate with `HeadlessHost: Host` over
      `SqliteStore` + `PayloadCipher`, built on the shared functions from I0;
      the broker test-suite runs against it.
- [ ] Unlock: password prompt on a TTY, or key material from a `0600` key file
      or a file descriptor; lock on `SIGTERM`; idle auto-lock.
- [ ] Transports: the local socket on the same path scheme as the app, plus a
      `ws://`/`wss://` listener (TLS from files, or plain behind a reverse
      proxy) carrying the same frames (`04`).
- [ ] Approvals without a GUI: rules from a config file; an `ask` waits for
      `rowel approve <id>` on the same machine or denies after the timeout;
      every decision an audit row (`03`).
- [ ] `rowel admin rules` to list, add and remove rules in the config; schema
      documented.
- [ ] Optional: `rowel serve --install-host` writes the native-messaging
      manifest so a browser on that box can use the extension against it.
- [ ] Integration test: `rowel serve` + `rowel mcp` + `rowel proxy` in a
      container, standing rules, one `ask` answered by `rowel approve`.
- [ ] **Acceptance:** on a Linux VM without a GUI a coding agent gets a header
      grant under a standing rule and an env grant after a TTY approval; the
      app's extension connects to the same broker when the host manifest is
      installed.

---

## I6 — Org lockboxes and enrollment in the app  `[ ]`

**Deliverable.** The desktop and iOS apps can hold a lockbox whose key came from
a server, sync it through that server, and answer that server's approval
requests in the same dialog. No visible change for users without a server. Spec
`01` (keying), `02` (enrollment), `07` (member API, `Provider::RowelServer`).

*Decide first:* Q7 (one curve or two for the enrollment key).

- [ ] `KdfParams::External { kid, gen }`: serde, `validate`, `derive` refuses,
      `next_generation`; unlock resolves the master from `Keyring` or the server
      and refuses clearly otherwise (`01`).
- [ ] Device enrollment key generated once and kept under the app key; shown
      with a fingerprint in Settings (`01`, `02`).
- [ ] `Provider::RowelServer`: `VaultStore` over the pack endpoints with ETag
      compare-and-swap; member session in `Tokens` (`07`).
- [ ] Settings › Lockboxes › *Connect to a Rowel Server*: sign in, enroll
      (pending until approved), list org lockboxes, add one as a workspace with
      the `External` sidecar and `WRAPPED_KEY_FILE` (`07`).
- [ ] Approve another device of mine from Settings (`02`).
- [ ] Approvals stream client (SSE or long poll) feeding `GrantDialog`; push
      token registration stubbed for I10 (`03 §4`).
- [ ] Tests against a fake server (in-process `axum`): add, sync, re-key,
      approve; unlock paths with and without a wrapped master.
- [ ] **Acceptance:** against the fake server the app adds an org lockbox,
      edits an item on two devices and converges, answers an approval, and
      survives a master rotation; a vault holding an `External` lockbox still
      unlocks offline with the password when the wrapped master is local.

---

## I7 — Rowel Server, single node  `[ ]`

**Deliverable.** A company runs one binary. Members sign in with their identity
provider and enroll devices. Agents in GitHub Actions prove who they are with
their OIDC token, request grants through the sidecar, and sensitive ones wait
for a tap on the owner's Mac. Audit exports as JSONL. Spec `02` (workload
identity, enrollment), `03 §4` (routing), `07`.

*Decide first:* Q1 (server license), Q2 (self-host first, assumed), Q3 (names).

- [ ] `rowel serve --server`: tenant metadata in SQLite, config file, data
      directory, `RootKey` from a key file (KMS implementations later) (`07`).
- [ ] Org lockbox lifecycle: create (mint, wrap, `External` descriptor),
      freeze, members and roles, wrapped masters per approved device (`07`).
- [ ] Member sign-in through generic OIDC; device enrollment pending → approve
      by an existing device or an admin; audit rows (`02`).
- [ ] Pack endpoints as merge authority: `decode_pack` → `merge` → `pack`,
      revision bumps on every server-side write; the Q19 interleaving test
      matrix on the `two_devices.rs` harness (`07`).
- [ ] `OrgHost: Host` over org lockbox files with the key LRU and idle timeout;
      `key_unwrapped` audit rows (`07`).
- [ ] Workload identity: challenge endpoint, `Oidc` verifier with GitHub
      Actions trust policies, audience binding, `jti` replay cache, token
      lifetime cap (`02`).
- [ ] `wss://…/v1/agent`; sidecar associations with `sidecar_for` binding (`03`, `05 §4`).
- [ ] Approvals: `pending_approvals`, routing to owner / group / principal, SSE
      stream, answer endpoint, per-rule timeouts (`03 §4`).
- [ ] `rowel admin` for agents, trust policies, rules, members, devices,
      lockboxes; `rowel admin audit export` and the HTTP export (`07`, `09`).
- [ ] Install docs: systemd unit, reverse proxy example, backup and restore
      (metadata + lockbox files + root key held apart).
- [ ] **Acceptance:** a GitHub Actions job with `rowel proxy` as a step-level
      sidecar authenticates by OIDC, gets an `auto` header grant for a read
      endpoint and waits for the owner's tap for a `money` endpoint; the audit
      export shows both; agent writes and device pushes interleave correctly.

---

## I8 — Tier 2 at scale  `[ ]`

**Deliverable.** Admins manage everything from the Rowel app, approvals route to
the right people without fatigue, keys rotate when people leave, and exports
feed existing tooling. Spec `03`, `07`, `09`, Q20.

- [ ] Admin surfaces in Settings for admin-role members: agents, trust
      policies, rules, lockbox members, devices (`07`).
- [ ] Per-lockbox rule delegation to teams; approver groups; batched approvals
      ("the next N uses by this agent for an hour") (`03`, Q20).
- [ ] Rate limits per agent at the server; anomaly flags in audit
      (request storms, URL-after-URL) (`03`).
- [ ] Master rotation with re-wrap; offboarding rotates lockboxes a leaver
      could read (`01`, `08 SSO notes`).
- [ ] Postgres as an alternative metadata store; sharding by lockbox id
      documented (`07`, Q18).
- [ ] Signed webhook and OTLP exports (`09`).
- [ ] **Acceptance:** a design-partner-sized tenant (hundreds of members, tens
      of agents) runs a week with approvals under a stated latency and no
      standing grant older than its policy allows.

---

## I9 — Rowel Cloud MVP on Cloudflare  `[ ]`

**Deliverable.** Hosted Tier 2: a company signs up, pays, connects, and gets
exactly the I7 behaviour without running anything. The broker is the same
binary in a Cloudflare Container; everything around it is Workers-native.
Spec `08` (Deployment on Cloudflare).

*Decide first:* Q4 (billing unit), Q25 (container transport spike result).

- [ ] Spike: `rowel serve --server` image in a Cloudflare Container bound to a
      tenant Durable Object; Worker routes HTTPS; confirm WebSocket
      pass-through to the container or adopt HTTPS-POST-per-frame for agents
      (`08`, Q25).
- [ ] Durable state: lockbox files in versioned R2, loaded on container
      activation, written back with compare-and-swap after each mutation; the
      tenant DO is the single writer and wakes the container (`08`).
- [ ] Metadata in D1 (or the DO's SQLite) behind the same store interface the
      single-node server uses.
- [ ] Root key: Worker secret or Secrets Store → per-tenant HKDF through
      `RootKey`; customer-managed KMS as the enterprise option (`08`).
- [ ] Self-serve signup, our own OIDC member login, tenant creation, Stripe
      billing with usage counted in D1 or Analytics Engine (`08`).
- [ ] Edge: WAF, rate limiting, custom domain, jurisdiction and region per
      tenant (`08`).
- [ ] Ops: health checks, structured logs, R2 versioning as backup, runbook.
- [ ] **Acceptance:** a new tenant completes the I7 GitHub Actions scenario
      against the cloud; state survives container sleep and a forced restart;
      a tenant pinned to the EU jurisdiction never runs or stores elsewhere.

---

## I10 — Approvals anywhere, enterprise fit  `[ ]`

**Deliverable.** Approvals reach phones as notifications, free users can approve
a laptop agent from their phone through the relay, enterprises bring their
identity provider, and audit streams to their SIEM. Spec `08`.

*Decide first:* Q24 (opt-in account in the free tier).

- [ ] Push approvals: APNs and FCM senders behind `ApprovalNotifier`; device
      tokens at enrollment; notifications carry ids only (`08`).
- [ ] Free-tier approval relay: Worker + KV with TTL, QR pairing, sealed
      payloads, opt-in account, rate limits (`08`, Q24).
- [ ] Enterprise SSO through Cloudflare Access (JWT assertion validated with
      the application audience) and SCIM deprovisioning that triggers
      rotation; self-serve keeps our OIDC login (`08`).
- [ ] SIEM export through Queues to signed webhooks and OTLP; retention per
      tenant (`08`, `09`).
- [ ] Residency enforcement in code paths that talk to storage, push and
      export; status page and error budget.
- [ ] **Acceptance:** an approval raised by a CI agent reaches a phone in
      seconds and is answered with Face ID; a free user approves a laptop ask
      from their phone; an Okta-removed member loses access and the lockboxes
      they could read are rotated.

---

## I11 — Reach  `[ ]`

**Deliverable.** The full architecture: agents in cloud browsers, OAuth-backed
APIs, more identity platforms, passkeys for org lockboxes where the standards
allow, policy as code. Spec `02`, `04`, `05`, Q5.

*Decide first:* Q5 (passkeys in org lockboxes), Q15 (CONNECT proxy).

- [ ] Remote extension transport (`wss://…/v1/extension`) with enrollment
      codes; recipes for Browserbase, Kernel and Anchor (`04`, Q22).
- [ ] Token mode: the `oauth` item kind, refresh, RFC 8693 exchange with an
      `act` claim where supported (`05`).
- [ ] AWS STS and SPIFFE verifiers (`02`, Q8).
- [ ] Passkeys in org lockboxes per the Q5 decision, step-up floor (`05 §3`).
- [ ] Policy as code on the same `evaluate` interface (`03`).
- [ ] `browsingData`-based storage clearing as an optional permission (Q12).
- [ ] Interop with whichever agent-credential standard gains traction
      (FIDO agentic auth, Bitwarden's SDK, IETF drafts) (`10`).
- [ ] **Acceptance:** a Browserbase session signs into a site with a Rowel
      fill approved on a phone; an agent calls a Google API with a token minted
      by Rowel from a stored refresh token; a Kubernetes job authenticates with
      SPIFFE.

---

## Release checklist (every iteration)

- [ ] All items above ticked and the acceptance demo recorded (a short screen
      capture or a test log linked from the PR).
- [ ] Threat model updated for anything security-relevant that shipped.
- [ ] Spec sections touched by the iteration updated; revision history line
      added in `README.md`.
- [ ] Decisions recorded in the project context; open questions resolved in
      `10-open-questions.md` marked ✅ with a pointer.
- [ ] README and website claims worded to what shipped, gaps stated.
