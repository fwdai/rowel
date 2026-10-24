# 11 — Roadmap and the first phase's PR slicing

The living, tickable checklist is [`PLAN.md`](PLAN.md); its iterations I0–I4
cover phases 0 and 1 and reference the PR acceptance criteria below, I5–I8
cover phase 2 with `rowel serve`, I9–I10 phase 3 on Cloudflare, I11 phase 4.
This file keeps the phase overview and the detailed phase 0 slicing.

## 1. Phases

Each phase ships something a person can use on its own. Nothing in a later
phase is required for an earlier one to be worth having.

```
  Phase 0   local, free      agents over the local socket · grants · consent dialog
                             grant-driven fill + exposure check · passkey attribution
                             MCP server + hooks · audit in the lockbox · session end
                             ─────────────────────────────────────────────────────
  Phase 1   local tooling    rowel run · rowel proxy · rowel redact · CLI keys in
                             the OS secure store · Vault Health agent panel polish
                             ─────────────────────────────────────────────────────
  Phase 2   server           rowel serve --server · org lockboxes (External KDF, wrapped
                             masters) · OIDC workload identity (GitHub first) ·
                             OrgHost · pack endpoints + merge authority · member and
                             device enrollment · approvals by polling · admin in
                             Settings
                             ─────────────────────────────────────────────────────
  Phase 3   cloud            multi-tenant deploy · KMS roots · push approvals ·
                             approval relay for free users · SSO/SCIM · SIEM export ·
                             billing
                             ─────────────────────────────────────────────────────
  Phase 4   reach            remote extension in cloud browsers · token mode /
                             OAuth broker · passkeys in org lockboxes (per Q5) ·
                             AWS STS and SPIFFE verifiers · policy as code ·
                             browsingData-based storage clearing
```

## 2. Phase 0, sliced into PRs

Conventions as in `docs/v1-next.md`: each PR is self-contained, keeps the repo
gates green (`bun run typecheck` · `bun run lint` 0 warnings · `bun run test` ·
`cargo test` · `cargo clippy -D warnings`; extension: `bun run extension:lint
&& extension:test && extension:build`), and lands with its tests. Order is a
dependency order; PRs 5, 6 and 7 can run in parallel once 4 is in.

### PR 1 — Extract the broker crate

- **Scope.** Move `protocol.rs`, `frame.rs`, `actions.rs`, `passkeys.rs`, the
  socket-name helpers and the protocol tests from `src-tauri/src/browser/` into
  `src-tauri/crates/broker` as `rowel-broker`. `passkeys.rs` imports
  `rowel_core::passkey` directly instead of the `crate::passkey` re-export.
  Re-export from `browser/mod.rs` so call sites do not change. No behaviour
  change.
- **Files.** `src-tauri/crates/broker/{Cargo.toml, src/lib.rs, src/protocol.rs,
  src/frame.rs, src/actions.rs, src/passkeys.rs, src/socket.rs, src/tests.rs}`,
  `src-tauri/Cargo.toml` (workspace member), `src-tauri/src/browser/mod.rs`,
  `server.rs`, `proxy.rs` (imports), `docs/development.md` (crate list).
- **Acceptance.** `cargo test -p rowel-broker` runs the moved tests unchanged;
  the app builds on macOS, Windows, Linux and iOS; extension connects and fills
  as before.

### PR 2 — Client kinds and agent association

- **Scope.** `Client { kind, platform, created_at, last_seen_at }` with
  `Extension` default on read; `hello` in `associate`/`test-associate`;
  `agents.enabled` setting; the associate dialog shows kind and platform;
  Settings lists agents in the clients list with their kind; the existing
  `browser_forget_client` revokes either kind.
- **Files.** broker `actions.rs`, `protocol.rs` (new codes);
  `src-tauri/src/browser/mod.rs` (`clients_in`/`write_clients` migration,
  `ask` payload), `settings.rs`, `events.rs`, `commands/browser.rs`;
  frontend `BrowserConsent/AssociateDialog.tsx` and
  `Settings/Sections/Browser/Clients.tsx`.
- **Acceptance.** A test client of kind `agent` associates through `Mock`
  and is refused while `agents.enabled` is false; an old `clients` row reads
  back as `Extension`; the Settings list shows and revokes an agent; the
  revoked agent's next request is refused (test through `Connection`).

### PR 3 — Grants, policy and the consent dialog

- **Scope.** `rowel_broker::{grant, policy, audit}` types, `evaluate`, `Floors`
  and `Limits`; `request-grant`, `wait-grant`, `end-grant`, `grant-status`,
  `list-visible` actions; `Host` additions implemented in `Mock` and `AppHost`;
  `Answer::Grant` in the consent slot with the blocking ask; `GrantDialog` with
  once / this run / N days / deny and step-up via biometrics or master
  password; the per-lockbox `agents` flag in the registry; rules and standing
  grants as sealed `meta` JSON; `audit` table, migration and writer (rows
  written, not yet synced or surfaced); grant expiry via `timer::Timer`.
- **Files.** broker crate (new modules + tests); `src-tauri/src/browser/mod.rs`
  (`AppHost`), `state.rs` (in-memory grants), `workspace.rs` (flag),
  `commands/browser.rs` (`agent_grant_respond`, `agent_rules`, `agent_grants`,
  revokes), `commands/workspace.rs` (flag command); core store migration for
  `audit`; frontend `BrowserConsent/GrantDialog/`, Settings lists, lockbox
  settings toggle.
- **Acceptance.** Policy evaluator unit tests cover specificity, both floors,
  rate limits and deny-by-default; protocol tests cover active / pending-busy /
  denied / expired paths and that no reply ever contains a secret field; a
  grant for a lockbox with agents off is denied before policy; e2e spec drives
  the dialog through each choice; a standing rule survives lock and unlock and
  is device-local (absent after a restore on another device is acceptable and
  asserted).

### PR 4 — Grant-driven fill in the extension

- **Scope.** `grants.js`, `rowel-signal` → `get-signals`, `grantId` on
  `get-logins`/`get-totp`, agent-mode flag and UI hiding, exposure check and
  clear-on-failure, `set-agent-mode` signal. Host side: `Signal::FillGrant` to
  every extension client, `get-signals`, `server::signal(ROWEL_SIGNAL)`, serving
  only admitted items under a grant and marking fill grants spent on first
  service. Safari `isSignal` learns the action.
- **Files.** `extension/rowel/background/{grants.js, keepass.js, page.js,
  client.js}`, `extension/rowel/content/{fill.js, rowel.js}`,
  `extension/rowel/popups/*` (agent-mode screen), `extension/tests/` (Playwright
  fixture page + spec); broker `actions.rs`; `src-tauri/src/browser/{mod.rs,
  server.rs}`; `src-tauri/gen/safari/Sources/HostConnection.swift`;
  `docs/browser-extension.md` (conflict table rows).
- **Acceptance.** Playwright: with agent mode on, a fill without a grant is
  refused; with an active grant the fixture form is filled and submitted; a
  second extension client asking for the same grant is refused; a fixture
  that echoes the password triggers clear + `exposed` report; in-page UI is
  absent in agent mode. Upstream sync script (`extension:sync --check`) still
  passes.

### PR 5 — Passkey attribution, audit sync and Vault Health

- **Scope.** `requested_by` on the passkey ask; audit rows for every ceremony
  and grant event; `Snapshot.audit`, `export`/`merge` union, `vault_digest`
  including audit rows, pruning with `pruned_through`; Vault Health *Agent
  activity* panel with per-writer chain verify.
- **Files.** `src-tauri/src/browser/mod.rs` (`ask_passkey` payload),
  `events.rs`, `sync/engine.rs`, core `store/{sqlite.rs, hash.rs}`, frontend
  `BrowserConsent/PasskeyDialog.tsx`, `Settings/Sections/Audit/AgentActivity/`,
  commands `audit_agent_activity`, `audit_verify`.
- **Acceptance.** A ceremony from an agent client shows the line and writes a
  row; two fake devices syncing (the `two_devices.rs` harness) end with the
  union of both audit logs and both chains verify; verify reports a break when
  a row is altered in a fixture; a device with only new audit rows pushes.

### PR 6 — The `rowel` CLI: `mcp` and `hook`

- **Scope.** New crate `src-tauri/crates/cli` with socket discovery from the
  broker crate, two client keys (`0600` files now; secure store in phase 1),
  `rowel mcp` (stdio MCP, five tools, no values in results, agent key),
  `rowel hook` (pre-tool rewrite to the `rowel run` form, post-tool redact stub
  that passes through until phase 1), `rowel status`. Bundling in the desktop
  installers and the Settings *Install command line tool* action.
- **Files.** `src-tauri/crates/cli/**`, `src-tauri/Cargo.toml`, bundling config
  (`tauri.conf.json`, CI workflows), Settings General section, `docs/development.md`.
- **Acceptance.** An MCP client (a test harness speaking stdio MCP) requests a
  login; the app's dialog appears; approval returns `active` with no secret;
  a hook rewrites `STRIPE_KEY=rowel://… cmd` into the `rowel run` form; the
  agent key is refused at `resolve`.

### PR 7 — Session end

- **Scope.** `grant-ended` signal on `end-grant` and on expiry, cookie deletion
  per registrable domain with the existing `cookies` permission, the dialog's
  "will sign you out" notice.
- **Files.** broker grant lifecycle, `src-tauri/src/browser/mod.rs` (timer
  wiring), extension `grants.js`, dialog copy.
- **Acceptance.** Playwright: a cookie set by the fixture site is gone after
  `end-grant` and after expiry, in Firefox and Chromium; no manifest change;
  Safari documented as best-effort.

### PR 8 — Threat model and docs

- **Scope.** `docs/threat-model.md` "The browser extension" rewording and a
  new "Agents" section per `01`; `docs/agents.md` user guide; README line
  worded to what shipped.
- **Acceptance.** Docs review; no code.

## 3. Phase 1 and 2 at a glance (sliced when phase 0 lands)

Phase 1: `rowel run` and `redact` with the streaming matcher (one PR),
`rowel proxy` with path-prefix addressing and per-grant rate limits (one PR),
CLI keys in the OS secure store (one PR), `env`/`header` rules with full
command-line targets (folded into the first).

Phase 2: `KdfParams::External` and wrapped-master unlock in the app (one PR,
app only, testable with a fake broker); device enrollment key and the
approve-new-device flow (one PR); `rowel serve` headless broker and then its
`--server` mode with `OrgHost` over a lockbox file and the WebSocket agent
transport (two PRs, `PLAN.md` I5 and I7); OIDC verifier with
GitHub Actions trust policies, `jti` replay cache (one PR); pack endpoints +
merge authority + `Provider::RowelServer` in the app (one PR, the largest,
with the interleaving test matrix from Q19); member enrollment and approvals
by polling (one PR); admin surfaces in Settings (one PR); threat model update
(one PR).

## 4. Measures of done for the whole programme

- A developer can run Claude Code against a GitHub login and a Stripe key with
  neither value ever appearing in the transcript, the environment, or the
  MCP tool results, and can read what happened in Vault Health.
- A company can run the same agent in GitHub Actions against an org lockbox,
  with the job identified by its OIDC token, the sensitive endpoint waiting for
  a tap on a phone, and the audit row exported to their SIEM.
- Both use the same placeholders and the same `request-grant`.
