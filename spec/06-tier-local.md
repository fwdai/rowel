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
  `grant-ended` signals, re-arm to the next deadline. *As built in PR A:*
  the timer waits on an `Instant`, which does not advance while an Apple
  machine sleeps, and grants end by the wall clock, so each wait is capped at
  30 seconds (`agents::EXPIRY_RECHECK`) and the deadline re-read against
  `now_ms` when it comes due: a grant ends, and its site is signed out, at
  most half a minute after a wake. Listing the grants in Settings expires
  through the same recording path (`list_grants_in`) and is followed up like
  a firing (`grants_changed`: lists, re-arm, wake the extensions), with the
  locks let go. `rowel serve` and the Rowel Server already sweep every
  second.
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
one id for the process. `rowel serve` writes under its own `serve_writer_id`
(PR C), never the `install_id` a copied data directory brings from the
device, so the box and the device never extend one chain; both ids come from
`rowel_core::layout::kept_id`.

**As built in I2.** An agent's `request-grant` runs the same three phases as
the extension's `agentMode` fill, with the client record (kind, name) in
hand: the candidates for a fill are the logins whose `url_host` matches the
target origin, the dialog event carries `clientKind`, the agent's `reason`
(shown as text, never interpreted) and `endsSession` (the sign-out notice),
and activation queues the `fill-grant` signal. Signals live in
`AgentState.signals`, a per-client-key queue cleared with every seal;
`take_signals` drains it under the session and agents locks with no access
check; every push is followed, outside the locks, by `server::notify_signals`
(the clear `rowel-signal`). `grant_status`, `wait_grant` and `end_grant`
answer only for the grant's own client. `visible_items` reads the client's
own live rules and active grants, never the baseline. `client_seen` writes
`last_seen_at_ms` once per connection, at `test-associate`.

**As built in I3.** `request_grant` takes a *secret* request (`header`,
`env`) through the same three phases as a fill: the candidates are the
lockbox's items of a kind with a field to serve, named by id or title slug;
the probes are the request's targets concretised (`03`); activation writes
no `used` row. The dialog event carries `targets: [{kind: "origin", origin}
| {kind: "http", host, path, methods} | {kind: "command", line}]` in place
of `origin`, and `endsSession` only for a fill; the `GrantDialog` titles a
header ask by the request pattern and an env ask by the command line, shown
verbatim in a block, lists every item the request named rather than offering
a pick (a secret request is covered whole or not at all, `03`), and "for N
days" writes `defaults::standing_for` with the request's targets and mode,
naming every item. `resolve` (`03`, `04`) is `AppHost`'s one
reply that carries a value to a non-extension, made in the same critical
section as `grant_covers`. Settings' `RuleRow` and `GrantRow` gain `mode`
and `target` (an origin as written; `GET, POST api.stripe.com/v1/*`; the
command line), which the lists show beside the mode in words; `origin` is
kept for fill rows. A secret grant's end signals no extension: there is no
site to sign out of.

**As built in I5.** The engine left the app. Everything above that knows no
window, session or timer lives in `rowel_broker::agents` — `AgentState` with
its access epoch, the audit words (`draft`, `about`, `record`, the target
texts), `decide`, `settle`, `deny_unasked`, `covered`, `client_covered`,
`resolved`, `visible_in`, the Settings rows — and takes a `Locked { store,
held, writer, now }` the host hands it under its own session → agents locks;
`decide` also takes the host's `extra` rules, evaluated after the baseline and
before the lockbox's own (the app passes none). The lockbox-side reads
(`logins_in`, `totp_in`, `clients_in`/`write_clients`, `rules_in`/`live_rules`,
`item_by_name`, `field_value`) sit in `rowel_broker::lockbox` beside
`save_login_in`; the socket serve loop and the signal registry in
`rowel_broker::server`, whose `register` takes a transport-neutral sink so a
WebSocket connection joins the same lock signals; TOTP code generation in
`rowel_core::otp::code_now`, with `totp-rs` a core dependency.
`src-tauri/src/browser/mod.rs` keeps the consent slot, `AppHost` as a thin
host (the locks, the dialog, the epoch check, the events, the expiry timer)
and the Safari socket paths; `src-tauri/src/agents.rs` keeps `initialise`,
`change_access`, `rearm`, `clear` and the Settings wrappers. Nothing about
the release invariant or the lock order changed; the headless host
(`rowel-serve`, `07`) runs the same functions under locks of its own.

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

**CLI binary (🟡, I2).** `src-tauri/crates/cli` (`rowel-cli`), a workspace
member built with the rest. Its binary is named `rowel-cli`, not `rowel`: the
app's own executable is `rowel` (`Contents/MacOS/rowel`, `/usr/bin/rowel`),
so a sidecar of the same name would collide in Cargo's target directory, in
the `.app` on a case-insensitive disk and in the Linux packages. It ships
beside the app as a Tauri sidecar (`bundle.externalBin`), release-only: the
release workflow builds it for the target (both Apple architectures,
`lipo`'d) into `src-tauri/binaries/rowel-cli-<triple>` and merges
`src-tauri/tauri.release.conf.json` with `--config`, so a dev build never
needs it. Settings › General › *Command line tool* (`cli_status`,
`cli_install`) finds the sidecar beside the running executable and, on macOS
and Linux, symlinks it as `rowel` into `/usr/local/bin` when writable, else
`~/.local/bin`; on Windows it shows the path to add to PATH. Client
subcommands: `status` and `mcp` (I2), `hook`, `run`, `proxy`, `redact` (I3).
Two client identities (`04`): the agent key used by `mcp` (`keys::Role::Agent`,
`rowel/agent.json`), the sidecar key used by `run`, `redact`, `hook post` and
`proxy` (`Role::Sidecar`, `rowel/sidecar.json`), each a record of the public
key and the name the app gave the client, each let in through the same dialog
once; `status` reports both and where they live. Since I3 the record lives in
the OS secure store where there is one (`keys::SecureStore`: a login-keychain
generic password on macOS, a Credential Manager entry on Windows, service
`app.rowel.cli`, account `agent` / `sidecar`, no biometric gate — what it adds
over a file is the store's own access control, under which another program
reading the item prompts the user) and in the `0600` file elsewhere; a file
from an earlier release is moved into the store the first time it is read,
and a store that refuses is stood in for by the file for that run, said once
on stderr, so a broken keychain never strands an association (Q9). It is the
CLI crate's own backend rather than the app's `secure_store.rs`, which links
Tauri. The same binary gained `serve` (the headless broker, I5; in `--server`
mode later the Rowel Server), `approve` and `admin rules|agents|key-file|lock`
(`07`, `docs/serve.md`), and since I5 every client command reaches a broker
elsewhere when `ROWEL_BROKER=ws://…|wss://…` is set, with `ROWEL_BROKER_CA`
for a CA of one's own (`04`). The desktop app is not split into a UI plus this daemon: it embeds
the same crates and is the broker itself (Q26). *As built in PR B (#667):* on
Windows the bundled folder holds the app's own `Rowel.exe`, so *Install*
copies the tool to `%LOCALAPPDATA%\app.rowel.desktop\bin\rowel.exe` and puts
that folder on the user's PATH (HKCU, `WM_SETTINGCHANGE`); an AppImage keeps a
copy in the app's local data directory and links `~/.local/bin/rowel` to it;
both copies refresh at startup; *Remove* (`cli_uninstall`) undoes either; the
row shows the bundled path everywhere. A foreign read of the CLI's keys
prompts only on macOS (the keychain), not in Windows Credential Manager. The
release also carries the CLI on its own (macOS notarized zip, Windows zip,
Linux x86_64 and arm64 tarballs at glibc ≥ 2.35) with `SHA256SUMS`. *As built
in PR C (#668):* the native-messaging manifests and the stdio↔socket relay
live in `rowel_broker::{manifest, relay}`, shared by the app and
`rowel native-host`; `rowel serve --install-host` writes an owner-only
launcher in its data directory and a manifest per installed browser (macOS,
Linux; refused on Windows, where only the app writes the registry), and
`--uninstall-host` removes them.

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
