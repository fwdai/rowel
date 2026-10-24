# 06 — Tier 1: the desktop app as the broker

## 1. What a developer gets, for free

Rowel is already running on the machine, unlocked with the master password,
registered as the native host for the browser extension, listening on a local
socket. Tier 1 makes that socket the place agents go.

- A coding agent (Claude Code, Codex, Cursor) uses the MCP server or the hook
  to ask for a GitHub login, a Stripe key for one command, or a `.env` entry.
  Rowel asks the person, once or for a week, and fills, injects or swaps.
- An agentic browser (Claude in Chrome, Comet, Atlas, a Browser Use script on
  the same machine) gets logins through the extension in agent mode, with the
  in-page UI hidden and every fill a grant.
- Passkeys work for agents through the same dialog they work for people, with
  one extra line saying who asked.
- Everything is written to a log the user can read in Vault Health.
- Nothing leaves the machine, and nothing changes for a user who never turns
  any of this on. The browser extension setting grows an "Agents" section;
  the default is off, and so is agent access on every lockbox.

This is the complete product for one person, and the adoption path into
companies: the placeholders a developer puts in their agent's instructions are
the same ones that resolve against the server at work.

## 2. The pieces

```
  ┌───────────────────────────── the machine ─────────────────────────────────┐
  │                                                                            │
  │   ┌──────────────┐    ┌──────────────┐    ┌──────────────┐                 │
  │   │ Claude Code  │    │ Claude in    │    │ Browser Use  │                 │
  │   │ (MCP / hook) │    │ Chrome       │    │ script       │                 │
  │   └──────┬───────┘    └──────┬───────┘    └──────┬───────┘                 │
  │          │ stdio             │ drives            │ drives                  │
  │   ┌──────▼───────┐    ┌──────▼──────────────────▼───────┐                  │
  │   │ rowel mcp    │    │ Chrome profile                  │                  │
  │   │ (agent key)  │    │  ┌──────────────────────────┐   │                  │
  │   │ rowel run /  │    │  │ Rowel extension          │   │                  │
  │   │ rowel proxy  │    │  │ agent mode · grant fill  │   │                  │
  │   │ (sidecar key)│    │  └────────────┬─────────────┘   │                  │
  │   └──────┬───────┘    └───────────────┼─────────────────┘                  │
  │          │ socket                     │ native messaging → relay → socket  │
  │   ┌──────▼────────────────────────────▼──────────────────────────────┐     │
  │   │ Rowel app                                                        │     │
  │   │  AppHost (Host impl) · grants · policy · consent dialog · audit  │     │
  │   │  lockboxes: Personal (agents off)  Work (agents off)             │     │
  │   │             Agents (agents on, rules + standing grants)          │     │
  │   └──────────────────────────────────────────────────────────────────┘     │
  └────────────────────────────────────────────────────────────────────────────┘
```

## 3. What the user sees

- **Settings › Browser extension** becomes **Settings › Browser & agents**:
  the existing extension list, plus connected agents (name, platform, last
  used, revoke), standing rules, and active grants.
- **The consent dialog** gains a third shape beside *associate* and *passkey*:
  the grant ask, with the agent's name and reason, the item (or a picker when
  several match), the target, and the choices *once · this run · N days ·
  deny*. Step-up adds the biometric prompt, or the master password on Linux.
- **Vault Health** gains *Agent activity*: recent uses, standing rules older
  than 30 days, agents not seen in 90 days.
- **Lockbox settings** gain *Agents can use items in this lockbox*, off by
  default, with the plain-language note that while Rowel is unlocked, agents
  with standing rules use items here without asking.

---

### Technical details

**Crate extraction (🟡, PR 1).** Move `src-tauri/src/browser/{protocol.rs,
frame.rs, actions.rs, passkeys.rs}` and the protocol half of `tests.rs` into
`src-tauri/crates/broker` (`rowel-broker`). `actions.rs` imports only
`super::{passkeys, protocol}`; `passkeys.rs` imports `super::protocol` and
`crate::passkey::{store::PasskeyVault, Authenticator, UserConsent}`, which is
the `rowel_core::passkey` re-export in `src-tauri/src/lib.rs` and becomes a
direct `rowel_core::passkey` import. Neither touches Tauri. What stays in
`src-tauri/src/browser/`: `mod.rs` (`AppHost`, `Pending`, consent glue,
`clients` storage), `server.rs` (socket accept, signal queues), `proxy.rs`
(native-messaging relay), `manifest.rs`. The socket-name and root-dir helpers
the CLI also needs (`socket_name`, `root_dir`) move to the broker crate. No
behaviour change; the existing tests pass in the new crate with the `Mock`
host.

**AppHost additions (❌).** In `src-tauri/src/browser/mod.rs`:

- `request_grant`: check the workspace's `agents` flag, build `GrantRequest`,
  load rules from the open lockbox (`meta` key `agent_rules`, sealed JSON like
  `browser_clients`), `policy::evaluate` with `Limits`, then either activate,
  deny, or ask through `CONSENT.ask` with `Answer::Grant`, blocking the
  connection's thread as `associate` does. The ask emits
  `events::agent_grant(app, id, AskPayload)` and `window::raise`. Active grants
  live in an in-memory `Grants` map on `AppState`; standing ones are rules.
- `resolve`: only for `Client.kind == Sidecar`; unseal the item with
  `session.payload_cipher()` as `totp` does today; pick the field by
  `Reference.field`; bump `uses`; audit.
- `take_signals`: drain the per-client signal queue; `request_grant` for
  `fill` pushes `server::signal(ROWEL_SIGNAL)` after queueing.
- Expiry: a `timer::Timer` (one pending callback, re-armable) armed to the
  earliest `expires_at`; when it fires, expire what is due, queue
  `grant-ended` signals, re-arm to the next deadline.
- The session epoch rule applies throughout: a lock or lockbox switch between
  ask and answer discards the grant (`commands::unlocked_epoch`,
  `session.store_at(epoch)` as `associate` does). A lock also clears the
  `Grants` map: nothing is served from a locked vault.

**As built in I1.** The grant state, expiry timer, rules in `meta`, audit
helpers and the Settings DTOs live in `src-tauri/src/agents.rs`; `AppHost`'s
`request_grant` is three phases (`decide_grant` under the session lock, the
ask with the lock released through `CONSENT.try_ask`, which tells a busy slot
apart from no answer, and `settle_grant` under the lock again after the epoch
check). Commands: `browser_set_agents_enabled`,
`browser_set_agents_standing_days` (1–90), `agent_grant_respond(id, answer,
password?)`, `agent_rules`, `agent_grants`, `agent_revoke_rule`,
`agent_end_grant`, `workspace_set_agents`. Events: `agent:grant` (the ask:
client name, origin, mode, `stepUp`, `verify: "biometric" | "password"`,
candidates, default days) and `agents:changed`. The lockbox flag is
`Workspace.agents` in the core registry. Step-up is enforced in Rust:
`agent_grant_respond` verifies the person before a yes counts, with the OS
biometric prompt where there is one and otherwise the master password derived
from the open lockbox's KDF sidecar and compared in constant time; a failed
verification is the refusal. `session::sealed` clears grants and rates on
every lock.

Two more invariants hold the ask and the log together. **The consent slot is
the only registry of a pending ask**: the slot records the ask's kind
(associate, passkey, grant with or without step-up), the grant ask reserves
the slot and emits its dialog in one step under the slot's lock after
verifying the access epoch, and a switch-off refuses whatever grant ask is up
through the slot and closes its dialog (`agent:grant:closed`); there is no
second list to fall out of step. Every lock, auto-lock and lockbox switch
(`session::sealed` → `agents::clear`) refuses the ask the same way, bumps the
access epoch and resets the in-memory access state, so nothing decided under
the previous session can settle in the next, and no dialog or waiting request
survives it. Lock order: workspace_lock → session → agents; slot → agents;
never agents → slot; never session → workspace_lock. **One install id per process**: the audit writer id is
minted once into an owner-only file and held in `AppState` behind a
`OnceLock`, so concurrent first uses agree and a write failure still leaves
one id for the process.

**Server loop (✅, unchanged).** `server.rs::accept` spawns a thread per
connection and `serve` stays one-request-one-reply; a blocking ask inside
`request_grant` parks only that connection's thread, exactly as `associate`
does today. The per-connection signal queue (`server::register`) carries the
new clear `rowel-signal` beside `database-locked`/`unlocked`.

**Consent UI (❌, frontend).** `src/components/Main/BrowserConsent/` holds
`AssociateDialog.tsx` and `PasskeyDialog.tsx` today, driven by the
`browser_associate` and `browser_passkey` events (`src-tauri/src/events.rs`,
`src/store/events.ts`, `src/api/browser.ts`). Add `GrantDialog/` beside them,
driven by a new `agent_grant` event; answers go through a new command
`agent_grant_respond(id, answer)` beside `browser_respond` in
`src-tauri/src/commands/browser.rs`. Step-up calls the existing biometric
authenticate command before responding, or verifies the master password on
platforms without one. Split per the repo's React convention: folder with
`index.tsx`, `ItemPicker.tsx`, `Choices.tsx`; tests beside
`src/test/browser.test.tsx` and `browserPasskey.test.tsx`.

**Settings (❌, frontend).** `src/components/Main/Sidebar/Settings/Sections/Browser/`
gains `Agents/` with lists bound to new commands `agent_rules`, `agent_grants`,
`agent_revoke_rule`, `agent_end_grant`; agents themselves are rows of the same
`clients` list (`browser_status`, `browser_forget_client` already exist and
work for any kind). Events `agents:changed` refresh them, as
`events::browser_clients` does for extensions.

**Settings model (🟡, `settings.rs`).** `browser: { enabled, compat }` gains
`agents: { enabled: bool, default_standing_days: u32 }`. Agents are refused at
`associate` while `agents.enabled` is false, with a `Code` that the CLI turns
into "turn on Agents in Rowel › Settings". The per-lockbox `agents` flag lives
in the workspace registry (`01`).

**CLI binary (❌).** `src-tauri/crates/cli` (`rowel` binary), built in CI for
desktop targets and shipped beside the app (in the `.app` bundle's `MacOS/`,
the Windows installer, the Linux packages), with an *Install command line tool*
button in Settings that symlinks it into the PATH. Client subcommands: `mcp`,
`hook`, `run`, `proxy`, `redact`, `status`. Two client identities (`04`): the
agent key used by `mcp`, the sidecar key used by `run` and `proxy`; both in the
OS secure store where available (a non-gated item beside the biometric one in
`secure_store.rs`), else `0600` files. The same binary later gains `serve`
(the headless broker, and in `--server` mode the Rowel Server), `approve` and
`admin` (`07`, `PLAN.md` I5 and I7). The desktop app is not split into a UI
plus this daemon: it embeds the same crates and is the broker itself (Q26).

**Extension (🟡).** `extension/rowel/background/grants.js` (new), changes in
`client.js` (`onNativeMessage` routes `rowel-signal`), `page.js` (hide UI in
agent mode), `keepass.js` (pass `grantId` through `get-logins`/`get-totp`, add
`get-signals`), `content/` (exposure check after fill), the popup (agent-mode
screen). No manifest permission changes in phase 0: `tabs`, `cookies`,
`storage` and `nativeMessaging` are already granted. Every change is a
Rowel-only file or a listed conflict in `docs/browser-extension.md`'s table;
keep upstream identifiers. Safari: `HostConnection.swift::isSignal` learns the
new action; agent mode itself is deferred there (Q14).

**Audit (🟡).** See `09`; in Tier 1 rows go into the open lockbox's `audit`
table, travel with the pack as a union, and surface in Vault Health.

**Tests.** Broker-crate tests through `Mock` for every new action, the policy
evaluator and the rate limits; a Playwright test in `extension/tests` for
grant-driven fill, the exposure check and cookie deletion against a fixture
page; an e2e spec for the dialog (`tests/e2e`, `data-testid` on every control).
