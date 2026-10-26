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

## I0 — Extract the broker crate  `[x]`  *(merged: PR #652)*

**Deliverable.** Nothing visible changes. The protocol, framing, `Host` trait,
passkey glue and socket helpers live in `rowel-broker` with their tests, ready
to be linked by the app, the CLI and the server. Spec `06` (Crate extraction),
`11 §2 PR 1`.

- [x] Create `src-tauri/crates/broker`; move `protocol.rs`, `frame.rs`,
      `actions.rs`, `passkeys.rs`, the socket-name and root-dir helpers, and
      the protocol tests; `passkeys.rs` imports `rowel_core::passkey` directly.
- [x] Re-export from `src-tauri/src/browser/mod.rs`; no call-site changes.
- [x] Factor the lockbox-side functions `AppHost` uses into the broker crate
      as functions over `SqliteStore` + `PayloadCipher`, so a headless host (I5)
      and `OrgHost` (I7) call the same code. Behaviour unchanged. *Outcome:*
      `save_login_in` moved (`rowel_broker::lockbox`); `logins_in` and the TOTP
      lookup stayed in the app because code generation lives in the app's
      `commands::generator` (core's `otp` only parses seeds). They move with
      the first item of I5.
- [x] Workspace member in `src-tauri/Cargo.toml`; crate list in `docs/development.md`.
- [x] **Acceptance:** `cargo test -p rowel-broker` passes the moved tests; app
      builds on macOS, Windows, Linux, iOS; the extension connects and fills.

---

## I1 — Agent mode for agentic browsers  `[x]`  *(merged: PR #653)*

**Deliverable.** A person using Claude in Chrome, Comet or any agentic browser
turns on agent mode in the Rowel extension; from then on every fill is a grant
the person approves in Rowel's dialog, the extension's own UI is hidden, a page
that echoes a secret is cleared, and a standing rule removes the dialog for a
site for N days. No new clients, no CLI yet. Spec `01` (agent-access flag),
`03`, `05 §2`, `06`.

*Decide first:* none (names in Q3 can stay provisional).

- [x] Per-lockbox agent-access flag: `Workspace.agents` in the registry, a
      command, and the toggle with its plain-language copy in lockbox settings
      (`01`, `06 §3`).
- [x] `rowel_broker::{grant, policy}`: `Grant`, `Rule`, `Target`, `evaluate`,
      `Floors`, `Limits`, with unit tests for specificity, both floors, rate
      limits and deny-by-default (`03`).
- [x] `rowel_broker::audit::Writer` and the `audit` table migration in core;
      rows written for every grant event (`09`, not yet synced or shown).
- [ ] `Answer::Grant` in the consent slot; `agent_grant` event;
      `BrowserConsent/GrantDialog/` with once · this run · N days · deny, item
      picker, step-up via biometrics or master password (`03`, `06`).
      *Audit 2026-10-27:* built except *this run*, which the dialog never
      offered; it lands in PR C.
- [x] Rules and standing grants as sealed `meta` JSON; Settings › Browser &
      agents lists rules and active grants with revoke (`03`, `06`).
- [x] `settings.browser.agents.enabled` (default off) gating all of the above.
- [x] Extension: agent-mode toggle in the popup, `agentMode`/`grantId` on
      `get-logins` and `get-totp`, in-page UI and autocomplete suppressed,
      exposure check after fill and after submit with clear-on-failure (`05 §2`).
- [x] Host: in agent mode a fill runs the grant flow (policy → ask → serve the
      one admitted item); outside agent mode behaviour is unchanged.
- [x] Grant expiry via `timer::Timer` armed to the earliest deadline.
- [ ] Tests: broker protocol tests through `Mock`; Playwright fixture page for
      approved fill, denied fill, exposure; e2e spec for the dialog.
      *Audit 2026-10-27:* the denied-fill fixture and the `tests/e2e` dialog
      spec were never written (the dialog has a Vitest test only); PR C.
- [x] Threat model: one paragraph noting that agent mode makes fills ask
      (`01`, full rewrite comes in I4).
- [x] **Acceptance:** with agent mode on, reaching a login page in an agentic
      browser raises the dialog; approve fills, deny fills nothing; a 7-day
      rule fills without a dialog next time and is listed and revocable; the
      exposure fixture is cleared and logged.

---

## I2 — Agent clients and the MCP server  `[x]`  *(merged: PR #655)*

**Deliverable.** Claude Code, Codex or Cursor configured with `rowel mcp` can
ask to log into a site; the person approves; the extension fills the open tab;
the grant ends and the site's cookies go with it. Tool results never contain a
secret. Spec `02` (local association), `04`, `05` (signals, session end), `06`
(CLI).

- [x] `Client { kind, platform, created_at, last_seen_at }` with `Extension`
      default on read; `hello` on `associate`/`test-associate`; the associate
      dialog shows kind and platform; Settings shows kinds;
      `browser_forget_client` revokes either (`02`, `11 §2 PR 2`).
- [x] Agent actions `request-grant` (blocking ask, `pending` + `retryAfterMs`
      when the slot is busy), `wait-grant`, `end-grant`, `grant-status`,
      `list-visible`; new `Code`s; implemented in `Mock` and `AppHost` (`04`).
- [x] Signals: clear `rowel-signal` push through `server::signal`, sealed
      `get-signals`, `fill-grant` queued for every extension client and
      claimed by the first one served (`Grant.served_to`); `get-signals` once
      after reconnect; Safari `isSignal` learns the action (`04`).
- [x] Extension `grants.js`: handles the wake-up, finds the tab by origin,
      fills through the existing path with `grantId`, agent mode on for the
      grant's lifetime when it was off (derived in the extension; no
      `set-agent-mode` signal) (`05 §2`).
- [x] `src-tauri/crates/cli` (`rowel-cli`): `status`, `mcp` (stdio MCP, the
      four tools the desktop host can serve — `rowel_request_secret` is
      listed from I3 — no values in results, agent key in a `0600` file for
      now) (`04`).
- [x] Session end: `grant-ended` on `end-grant` and on expiry; cookie deletion
      per registrable domain with the existing `cookies` permission; dialog
      notice (`05`).
- [x] Passkey ask shows *requested by …* and writes an audit row (`05 §3`).
- [x] Bundle the CLI in the desktop installers; *Install command line tool*
      in Settings › General (`06`).
- [x] Tests: broker tests for every action and for "no reply carries a secret
      field"; an MCP harness test; Playwright for grant-driven fill, second
      client refused, cookie deletion in Chromium and Firefox.
- [x] **Acceptance:** Claude Code asks for github.com; dialog; approve; the
      extension fills the open tab; the MCP result is `active` with no secret;
      after expiry the GitHub cookies are gone; a revoked agent's next call is
      refused.

---

## I3 — Secrets for commands and APIs: the sidecar  `[x]`  *(merged: PR #657)*

**Deliverable.** A coding agent uses an API key or an `.env` value for one
command or one API call without the value ever appearing in its transcript,
its environment or the MCP results. Spec `03` (floors, binding), `04` (thin
clients), `05 §4–5` (header, env, redaction).

- [x] `sidecar` client kind and the CLI's second key; `resolve` action refused
      for any other kind and bound to the grant id and the same lockbox (`03`, `04`).
      *Outcome:* `Host::resolve` with a default refusal; `AppHost` resolves
      under the session and agents locks (`03` "as built in I3"); the sidecar
      key is `keys::Role::Sidecar`.
- [x] Placeholder resolution `rowel://<lockbox>/<item>/<field>`; `list-visible`
      returns refs and fields (`04 §3`). *Outcome:* `rowel_broker::reference`;
      names resolve by id or slug (lockbox: registry name; item: title).
- [x] `rowel run --grant … -- cmd`: env mode, full-command-line targets, requests
      a grant when none is given, output through the redactor (`05 §5`).
      *Outcome:* as specified, plus `--env-file`, inherited `rowel://`
      variables, and the one `command::line` quoting rule (`05`).
- [x] `rowel redact`: streaming exact-match matcher with encodings and carry-over
      window, as a library and a filter (`05`). *Outcome:* values under 6
      bytes are never matched; the filter resolves its values under a grant.
- [x] `rowel hook`: pre-tool rewrite to the `rowel run` form, post-tool redact;
      documented configs for Claude Code, Codex and Cursor (`04`). *Outcome:*
      the post hook is an exposure check (resolve the command's own
      placeholders, scan the output, end the grant on a hit), not a rewrite of
      the output, which the platforms' Bash hooks cannot do (`05`, Q16).
- [x] `rowel proxy`: path-prefix addressing, `X-Rowel-Grant`, header and body
      placement rules, response redaction, per-grant token bucket (`05 §4`).
      *Outcome:* loopback only; body placement gated on content type (JSON,
      form) since the grant has no placement field yet; `/http/` behind
      `--allow-http`.
- [x] Policy floors for `env` and `header`; Settings lists header and env rules
      with their targets (`03`). *Outcome:* the baseline admits `header` and
      `env` at `ask`; `RuleRow`/`GrantRow` carry `mode` and `target`.
- [x] CLI keys in the OS secure store on macOS and Windows (non-gated item in
      `secure_store.rs`), `0600` files elsewhere (Q9). *Outcome:* in the CLI
      crate's own `keys` backend (the app's `secure_store.rs` links Tauri),
      with a one-time move of an existing file.
- [x] Tests: broker tests for `resolve` and binding; integration tests for
      `run`, `proxy` and `redact`; a rule for `node deploy.js*` must not match
      `node -e …`.
- [x] **Acceptance:** Claude Code runs a Stripe CLI command through `rowel run`
      and calls the Stripe API through `rowel proxy`; the key appears nowhere
      the agent can read; `env` printing the value comes back redacted.

---

## I4 — Audit you can read, and Tier 1 released  `[x]`  *(merged: PR #658; store submissions and the app release are the owner's steps)*

**Deliverable.** Vault Health shows what agents did, across the user's devices,
with a verifiable chain; the threat model and docs say what shipped. This
closes the free product. Spec `09`, `11 §2 PR 5 and PR 8`.

- [x] `Snapshot.audit`, export and `INSERT OR IGNORE` merge, `vault_digest`
      including audit rows, retention with `pruned_through` (`09`). *Outcome:*
      as specified, except that every device reclaims every writer's aged
      prefix at pack time and only the chain's own writer writes the
      `pruned_through` note — a union merge hands a device its own pruned rows
      back otherwise, and iOS has no chain of its own (`09`, Q21).
- [x] Vault Health › *Agent activity*: recent events, standing rules older than
      30 days, agents unseen for 90 days, *Verify log* (`09`). *Outcome:*
      `AgentActivity/` under the Vault Health section, desktop only, shown
      when the Agents switch is on or the log has something to show; commands
      `audit_agent_activity` and `audit_verify` in `commands/activity.rs`.
- [x] `docs/threat-model.md`: the extension section reworded and a new Agents
      section per `01`; `docs/agents.md` user guide; README claims worded to
      what shipped.
- [x] Release: extension version bump and store submissions; app release with
      notes; the spec's revision history updated. *Outcome:* extension 0.2.0
      in the PR; the store submissions and the app's Release workflow are the
      owner's manual steps after the merge (`docs/releasing.md`).
- [x] **Acceptance:** two devices (the `two_devices.rs` harness) converge on
      the union of their audit logs and both chains verify; a device with only
      new audit rows pushes; an altered row breaks verification.

---

## I5 — `rowel serve`: the broker without the GUI  `[x]`  *(merged: PR #659)*

**Deliverable.** The same crates run as a headless broker on a developer's VM,
a CI runner or a Linux box without a desktop: a device-held lockbox is unlocked
from a TTY or a key file, agents and sidecars connect over the local socket or
a WebSocket, standing rules serve them, and an `ask` is answered from a terminal
or times out. This is the binary the server (I7) and the cloud (I9) run. Spec
`06` (reuse), `07` (precursor to `OrgHost`).

- [x] Move TOTP code generation (the `totp()` builder behind
      `commands::generator::generate_otp`, and the `totp-rs` dependency) into
      `rowel_core::otp`, route the app's command through it, then move
      `logins_in` and the TOTP lookup into `rowel_broker::lockbox` beside
      `save_login_in`. Behaviour unchanged; this is the seam I0 left open.
      *Outcome:* done, and further: the whole Tauri-free grant engine moved
      to `rowel_broker::agents` and the socket serve loop to
      `rowel_broker::server`, so the headless host runs the app's code
      rather than a copy (`06`).
- [x] `rowel-serve` library crate with `HeadlessHost: Host` over
      `SqliteStore` + `PayloadCipher`, built on the shared functions from I0;
      the broker test-suite runs against it. *Outcome:* `rowel-serve` with
      `HeadlessHost`; its tests drive `Connection<HeadlessHost>` over a real
      temp vault through the broker's test extension and peer (associate,
      auto rule, ask → approve → resolve, deny, timeout, switch off, step-up
      password, seal, key file, WebSocket, control socket).
- [x] Unlock: password prompt on a TTY, or key material from a `0600` key file
      or a file descriptor; lock on `SIGTERM`; idle auto-lock. *Outcome:* as
      specified; the key file is written by `rowel admin key-file`; an idle
      lock reopens from a key file, re-prompts in password mode and is
      refused with `--key-fd`.
- [x] Transports: the local socket on the same path scheme as the app, plus a
      `ws://`/`wss://` listener (TLS from files, or plain behind a reverse
      proxy) carrying the same frames (`04`). *Outcome:* as specified, plus
      a lock file the app and the broker both take, so one broker serves a
      data directory, ceilings on the WebSocket before association
      (connections, handshake time, message size, queued requests), and
      `ROWEL_BROKER` on the client side.
- [x] Approvals without a GUI: rules from a config file; an `ask` waits for
      `rowel approve <id>` on the same machine or denies after the timeout;
      every decision an audit row (`03`). *Outcome:* `serve.json` rules; an
      approvals queue with a control socket; associations and passkey
      ceremonies are asks too; step-up by master password; 120 s default.
- [x] `rowel admin rules` to list, add and remove rules in the config; schema
      documented. *Outcome:* plus `admin agents on|off`, `admin key-file`,
      `admin lock`; schema in `docs/serve.md`.
- [ ] Optional: `rowel serve --install-host` writes the native-messaging
      manifest so a browser on that box can use the extension against it.
      *Not built in I5; the owner chose to build it in PR C (2026-10-27).*
- [x] Integration test: `rowel serve` + `rowel mcp` + `rowel proxy` in a
      container, standing rules, one `ask` answered by `rowel approve`.
      *Outcome:* `crates/cli/tests/serve.rs` over the built binary and a temp
      data directory; runs where the CLI's keys are files (Linux), ignored on
      macOS and Windows, whose secure store a test must not touch.
- [ ] **Acceptance:** on a Linux VM without a GUI a coding agent gets a header
      grant under a standing rule and an env grant after a TTY approval; the
      app's extension connects to the same broker when the host manifest is
      installed. *Audit 2026-10-27:* the agent half holds; the extension half
      waits on `--install-host` (PR C) and a standalone CLI artifact (PR B).

---

## I6 — Org lockboxes and enrollment in the app  `[x]`  *(merged: PR #662)*

**Deliverable.** The desktop and iOS apps can hold a lockbox whose key came from
a server, sync it through that server, and answer that server's approval
requests in the same dialog. No visible change for users without a server. Spec
`01` (keying), `02` (enrollment), `07` (member API, `Provider::RowelServer`).

*Decide first:* Q7 (one curve or two for the enrollment key). *Settled: one
X25519 key; a signing key is minted beside it when delegation lands.*

- [x] `KdfParams::External { kid, gen }`: serde, `validate`, `derive` refuses,
      `next_generation`; unlock resolves the master from `Keyring` or the server
      and refuses clearly otherwise (`01`). *Outcome:* as specified; the
      server is asked only on a gesture (`org_refresh_lockbox_key`, which also
      adopts a rotation), never during an unlock. A password typed for an org
      lockbox, a master-password change from inside one, and promoting one to
      primary are refused; the password-change saga skips org lockboxes.
- [x] Device enrollment key generated once and kept under the app key; shown
      with a fingerprint in Settings (`01`, `02`). *Outcome:*
      `enrollment.key.sealed` in the root; the member session once per install
      beside it (`servers.sealed`); both re-sealed by a password change and
      purged when stale after a primary deletion.
- [x] `Provider::RowelServer`: `VaultStore` over the pack endpoints with ETag
      compare-and-swap; member session in `Tokens` (`07`). *Outcome:* the
      token file names the server only; the session is resolved from the
      install's sealed file; never in the account pickers.
- [x] Settings › Lockboxes › *Connect to a Rowel Server*: sign in, enroll
      (pending until approved), list org lockboxes, add one as a workspace with
      the `External` sidecar and `WRAPPED_KEY_FILE` (`07`). *Outcome:* the
      app signs in against the server as its OAuth2 authorization server
      (PKCE, loopback); desktop only in I6, the phone's deep-link twin later.
- [x] Approve another device of mine from Settings (`02`). *Outcome:* with
      the pending device's fingerprint quoted in the confirmation.
- [x] Approvals stream client (SSE or long poll) feeding `GrantDialog`; push
      token registration stubbed for I10 (`03 §4`). *Outcome:* SSE, one
      thread per approved connection while the vault is open, through the
      consent slot with the tenant named; no push stub yet.
- [x] Tests against a fake server (in-process `axum`): add, sync, re-key,
      approve; unlock paths with and without a wrapped master. *Outcome:*
      `org::fake_server` + `org::tests` (sign-in, enrollment, two devices
      converging with a stale PUT, rotation, approvals loop, stale-file
      reseal); the frontend's `org.test.tsx`.
- [x] **Acceptance:** against the fake server the app adds an org lockbox,
      edits an item on two devices and converges, answers an approval, and
      survives a master rotation; a vault holding an `External` lockbox still
      unlocks offline with the password when the wrapped master is local.

---

## I7 — Rowel Server, single node  `[ ]`  *(merged: PR #665; acceptance against real GitHub Actions is the owner's step; moved to the private repository `fwdai/rowel-server` on 2026-10-27, see "Tier 1 audit" below)*

**Deliverable.** A company runs one binary. Members sign in with their identity
provider and enroll devices. Agents in GitHub Actions prove who they are with
their OIDC token, request grants through the sidecar, and sensitive ones wait
for a tap on the owner's Mac. Audit exports as JSONL. Spec `02` (workload
identity, enrollment), `03 §4` (routing), `07`.

*Decide first:* Q1 (server license), Q2 (self-host first, assumed), Q3 (names).
*Settled for I7: the owner chose to build the server mode in this public
repository as its own crate, `crates/server` (`rowel-server`), so a later move
to a private repository is a crate move (`00 §4`, Q1); self-host first (Q2);
names stay provisional (Q3).*

- [x] `rowel serve --server`: tenant metadata in SQLite, config file, data
      directory, `RootKey` from a key file (KMS implementations later) (`07`).
      *Outcome:* `rowel-server`, linked by the CLI; `server.json`,
      `server.db`, `FileRootKey` (HKDF per tenant, AES-GCM with the lockbox
      id as associated data); one server per data directory (the broker's
      `Lock`); seal on SIGTERM; `GET /v1/health`; TLS from PEM files or
      plain behind a proxy. `rowel admin server init` bootstraps a data
      directory.
- [x] Org lockbox lifecycle: create (mint, wrap, `External` descriptor),
      freeze, members and roles, wrapped masters per approved device (`07`).
      *Outcome:* as specified; lockbox roles `reader`, `approver`, `admin`;
      a master is sealed only to an approved device and withdrawn with it.
      Rotation is I8.
- [x] Member sign-in through generic OIDC; device enrollment pending → approve
      by an existing device or an admin; audit rows (`02`). *Outcome:* the
      server is the app's OAuth2 server and an OIDC relying party to the IdP
      (verified email required; auto-provisioning only under
      `allowedDomains`); enrollment, approval and admin actions are rows of
      a tenant `server_events` table, not lockbox audit rows (`09`).
- [x] Pack endpoints as merge authority: `decode_pack` → `merge` → `pack`,
      revision bumps on every server-side write; the Q19 interleaving test
      matrix on the `two_devices.rs` harness (`07`). *Outcome:* the pack
      format and snapshot reader lifted into `rowel_core::sync`; the matrix
      runs in the server crate with core-built devices over HTTP rather than
      on the app's `two_devices.rs`, which links Tauri (`10`, Q19 ✅).
- [x] `OrgHost: Host` over org lockbox files with the key LRU and idle timeout;
      `key_unwrapped` audit rows (`07`). *Outcome:* a vault cache with an
      idle seal (10 min) rather than a sized LRU; an agent definition is
      bound to one lockbox; no fills, saves or passkeys from a server-held
      lockbox (Q5).
- [x] Workload identity: challenge endpoint, `Oidc` verifier with GitHub
      Actions trust policies, audience binding, `jti` replay cache, token
      lifetime cap (`02`). *Outcome:* `proof` rides inside `hello`; the
      host gets the session key through `Host::associate_on_channel`; a
      trust policy admits one client kind.
- [x] `wss://…/v1/agent`; sidecar associations with `sidecar_for` binding (`03`, `05 §4`).
      *Outcome:* the WebSocket loop of `rowel serve` lifted and shared; a
      sidecar is bound by a trust policy of kind `sidecar`, not by
      `sidecar_for`, which stays unenforced (`05`).
- [x] Approvals: `pending_approvals`, routing to owner / group / principal, SSE
      stream, answer endpoint, per-rule timeouts (`03 §4`). *Outcome:*
      routing to the lockbox's `approver`/`admin` members (and `acting_for`
      only if that member holds the lockbox); first answer wins; one
      server-wide ask timeout (`askTimeoutMs`), per-rule timeouts and
      long-parked asks are I8.
- [x] `rowel admin` for agents, trust policies, rules, members, devices,
      lockboxes; `rowel admin audit export` and the HTTP export (`07`, `09`).
      *Outcome:* `rowel admin server …` on the data directory, the HTTP
      admin API, and `audit verify` per lockbox and writer (Q21).
- [x] Install docs: systemd unit, reverse proxy example, backup and restore
      (metadata + lockbox files + root key held apart). *Outcome:*
      `docs/server.md`; the contract is `docs/server-api.md`.
- [ ] **Acceptance:** a GitHub Actions job with `rowel proxy` as a step-level
      sidecar authenticates by OIDC, gets an `auto` header grant for a read
      endpoint and waits for the owner's tap for a `money` endpoint; the audit
      export shows both; agent writes and device pushes interleave correctly.
      *Outcome so far:* the in-process equivalent is tested (fake IdP minting
      GitHub-shaped tokens, `auto` and `ask` grants, approver's answer,
      export, Q19 matrix); the run against real GitHub Actions needs a
      deployed server and is the owner's step.

---

## Tier 1 audit (2026-10-27): follow-ups before testers

An audit of I0–I7 on `main` against this plan and the spec found the items
above that were ticked but not delivered, two correctness bugs and the
distribution gaps. They land as four PRs; Tier 1 is ready for outside testers
when A, B and C are merged.

- [ ] **PR A, security and correctness** *(in review: #666)*: the
      redactor at a constant lag (no prefix oracle); one expiry path that always
      writes `ended` and signs out, with a 30 s wall-clock recheck; the audit
      fails closed (`AuditUnavailable` = 47); the proxy forwards only what a
      grant covers (`05`, `06`, `09`).
- [ ] **PR B, distribution:** the Windows *Install command line tool* path,
      the AppImage link, standalone CLI tarballs (Linux x86_64 and arm64),
      extension zips attached to releases, `docs/releasing.md`.
      *(in review: #667)* *Outcome:* on Windows the tool is copied to
      `%LOCALAPPDATA%\app.rowel.desktop\bin\rowel.exe` and that folder goes on
      the user's PATH (the bundled folder holds the app's own `Rowel.exe`); an
      AppImage keeps a copy in the app's local data directory; both refresh at
      startup; *Remove* undoes either. Standalone archives for macOS
      (notarized), Windows and Linux x86_64/arm64 (glibc ≥ 2.35) with
      `SHA256SUMS`; Chromium and Firefox zips attached; the extension version
      lives only in its source manifest; CLI, broker and core tests run on
      macOS and Windows CI; `--help` exits 0.
- [ ] **PR C, plan gaps:** *this run* in the grant dialog, the denied-fill
      fixture, the `tests/e2e` dialog spec, the untranslated agent strings,
      `rowel serve --install-host`. *(in review: #668, stacked on #666)*
      *Outcome:* a run grant (`Grant.for_run`) serves only requests carrying
      its own `run_id` from the same client, capped at 8 h, offered only when
      the request has a run id; every locale complete and tested; an expired
      grant answers `GrantExpired`; exposure reports only from the grant's
      holder or the served extension, once; `rowel serve` writes under its own
      `serve_writer_id`; manifests and relay in `rowel_broker::{manifest,
      relay}`, `rowel native-host`, `rowel serve --install-host` /
      `--uninstall-host` on macOS and Linux.
- [ ] **PR D, open-core boundary:** remove `crates/server`, `rowel admin
      server`, the `--server` wiring and `docs/server.md` from the public
      repository; they live in the private `fwdai/rowel-server` (one package,
      binary `rowel-server serve` / `rowel-server admin`, public crates pinned by
      git rev) (`00 §4`, Q1).

Tier 2, before any server is deployed (in the private repository): members
bound by the IdP's `sub` rather than matched by email, and `email_verified`
required; `ws://` refused off loopback for agents; workload issuers and JWKS
over https only.

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
