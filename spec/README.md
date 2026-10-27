# Rowel for Agents — specification

This directory is the design of Rowel's AI-agent features: how an agent gets to
use a credential without ever holding it, on a developer's own machine for free,
and at company scale through a paid server. It is written for two readers at
once. Every section opens in plain language for a person deciding what to build,
and closes with a **Technical details** subsection for an agent (or engineer)
implementing it against this codebase.

Read the overview first. Everything else hangs off it.

| File | What it settles |
| --- | --- |
| [BRIEF.md](BRIEF.md) | **Start here if you are an agent starting cold.** What this is, where things are, how a step is worked, the gates and pitfalls, the prompt that starts a step |
| [SUMMARY.md](SUMMARY.md) | The plain-language version: what we are building, the steps, and the decisions still open. Start here if you do not need internals |
| [PLAN.md](PLAN.md) | **The implementation checklist.** Eleven iterations, each a shippable superset of the last; tick items and iterations as they land |
| [00-overview.md](00-overview.md) | The problem, the principles, the three tiers and how they stack, the reuse map, the glossary |
| [01-trust-model.md](01-trust-model.md) | What each tier can read, device-held vs org lockboxes and the agent-access switch, the key hierarchy, why not env vars |
| [02-identity.md](02-identity.md) | Who is asking: humans, agents, runs, devices; local association, workload identity, delegation |
| [03-grants-and-policy.md](03-grants-and-policy.md) | The grant object, approval modes, policy rules, approval routing |
| [04-protocol.md](04-protocol.md) | The agent protocol: an extension of the existing sealed channel; actions, framing, transports, thin clients |
| [05-delivery-modes.md](05-delivery-modes.md) | How a credential is *used* without being handed over: fill, passkey, header, token, env; redaction; session end |
| [06-tier-local.md](06-tier-local.md) | Tier 1, the desktop app as broker: what changes in `src-tauri`, the extension and Settings |
| [07-tier-server.md](07-tier-server.md) | Tier 2, the self-hosted server: crates, org lockbox keying, the host trait over an org lockbox, storage, sync, approvals |
| [08-tier-cloud.md](08-tier-cloud.md) | Tier 3, Rowel Cloud: tenancy, KMS, push approvals, SSO, SIEM export, the approval relay for free users |
| [09-audit.md](09-audit.md) | The attributed, hash-chained access log and where it surfaces |
| [10-open-questions.md](10-open-questions.md) | Every part that is not settled, with options and a recommendation |
| [11-roadmap.md](11-roadmap.md) | Phases, and the PR slicing for the first phase with scope, files and acceptance |

## Conventions

- **Vocabulary** follows `docs/development.md`: *vault* (everything one master
  password opens), *lockbox* (a compartment; `workspace` in code), *item* (an
  `Entry`). New terms are defined in the overview's glossary and used
  consistently: *broker*, *agent*, *principal*, *run*, *grant*, *delivery mode*,
  *approver*.
- **Status marks** on headings and bullets: ✅ exists in the codebase today,
  🟡 partly exists or is a mechanical extension, ❌ new, ❓ open question
  (listed in `10-open-questions.md`).
- **Code references** name real files and symbols as of the commit this spec was
  written against (`5cb64c36`, October 2026). Proposed names for new crates,
  types and actions are proposals; keep them unless there is a reason not to,
  and update this spec when the code settles on something else.
- **Diagrams** are ASCII, in fenced blocks, so they render anywhere and diff.
- **Simplest thing first.** Where two designs would do, the spec picks the one
  with less new machinery and notes the other under open questions. Scope grows
  by phase (`11-roadmap.md`), not by ambition in phase 0.

## Revision history

- **2026-10-26, review of PR #665.** Device enrollment proves possession of
  the device key through a challenge bound to the server's URL, and sessions
  start unbound (`02`); replay records last as long as a token is accepted
  (`02`); freezing is a transition under the vault lock, file paths come from
  stored ids, and the app refreshes one caller at a time (`07`).
- **2026-10-26, I7 built (the Rowel Server, single node; PR #665).** I6
  ticked as merged (PR #662). The server mode is its own crate,
  `rowel-server`, built in this repository by the owner's decision (`00 §4`,
  Q1). Recorded as built: the proof inside `hello` and
  `Host::associate_on_channel`, the key-bound audience format, replay keyed
  by issuer, and a trust policy that admits one client kind, which is how a
  sidecar is bound instead of `sidecar_for` (`02`, `05`); an agent
  definition bound to one lockbox, the vault cache with an idle seal, no
  fills or passkeys from a server-held lockbox (`07`, Q5); pack endpoints as
  merge authority with the file existing from creation, revision bumps on
  every server-side write and 409 for another key generation (`07`, Q19 ✅);
  routing to approver members with one server-wide timeout (`03`); the
  audit mirror, export, verify and the separate tenant event table (`09`,
  Q21); the pack format lifted into `rowel_core::sync`.
- **2026-10-26, I6 built (org lockboxes in the app).** I5 ticked as merged
  (PR #659). Settled the enrollment key to one X25519 curve (Q7, half) and
  the sign-in model: the Rowel Server is the OAuth2 authorization server the
  app talks to, the IdP stays behind it, enrollment is a bearer call (`02`).
  The member session is held once per install in `servers.sealed` beside the
  enrollment key rather than in each lockbox's token file (`01`, `07`). The
  member API is recorded as built — `auth/authorize`, `auth/token`, `me`,
  the listing carrying `etag` and `wrappedMaster`, PUT preconditions, the SSE
  framing and keepalive — and the device half of the pack endpoints (`07`).
  Desktop ships the connect flow; the phone's is a later iteration.
- **2026-10-24, first draft**, written against commit `5cb64c36`.
- **2026-10-24, review against the code and the platforms.** Verified the
  seams the spec leans on (`browser/{actions,server,mod}.rs`, `appkey.rs`,
  `kdf.rs`, `sync/engine.rs`, the extension's `client.js`/`keepass.js`, both
  manifests, the Safari shim) and the external facts (MV3 service-worker
  lifetime with native messaging, `browsingData` origin semantics, crypto_box
  sealed boxes). Corrections made: Tier 1 has no key escrow, so "agent
  lockbox" became two independent properties, *org lockbox* (server-held,
  escrowed) and *agent access* (a per-lockbox switch); grant signals ride the
  existing clear wake-up push plus a sealed fetch instead of polling; passkey
  grants can never be `auto`; session end uses the `cookies` permission the
  extension already has, not a new `browsingData` permission; rules and
  standing grants are device-local in Tier 1 while audit rows sync as a union
  with per-writer hash chains (the merge ignores `meta`); `env` targets match
  the full command line; the grant id is the capability between agent and
  sidecar, and the CLI holds separate agent and sidecar keys; new-device
  enrollment needs approval; `jti` replay cache and audience fallback for
  workload tokens; the server's merge is the engine's receiving half, not the
  whole loop; rate limits on asks; the free-tier relay needs an opt-in account
  (Q24).
- **2026-10-24, implementation plan.** Added `PLAN.md`, the iteration
  checklist. Settled the packaging: the server is the `rowel` CLI's `serve`
  subcommand in `--server` mode, not a separate binary, and the desktop app
  is not split into a UI plus a daemon (Q26). Added the Cloudflare deployment
  mapping to `08` (Containers for the broker, Durable Objects per tenant, R2
  for lockbox files, D1 for metadata, Access for enterprise SSO) with the
  points to confirm in a spike (Q25).
- **2026-10-24, licensing.** Settled Q1: library crates relicensed to
  MIT OR Apache-2.0 (license files and manifest fields added to `rowel-core`
  and `rowel-autofill`; the broker crate follows when it lands), apps and
  extension stay GPL-3.0, server mode and cloud proprietary in a private
  repository, DCO sign-off for contributions. Repository map added to `00 §4`.
- **2026-10-24, I1 review rounds.** Recorded what three review rounds on
  PR #653 changed in the design: the release invariant, then the switch state
  moved under the agents lock with its epoch and the session lifecycle
  resetting it (`03`, `06`), the consent slot as the only registry of an ask
  and the full lock order (`06`), the per-install audit writer id (`06`,
  `09`), grants keyed by the requesting document's origin and a fill
  lifecycle that finalises its predecessor (`05`).
- **2026-10-25, I2 as built.** Client kinds and the agent vocabulary, the
  `rowel-signal` wake-up and the two signals, the client half of the channel
  and the CLI (`02`, `04`): the `Client` record's timestamps follow the
  codebase's `_ms` convention; `test-associate` also answers
  `brokerCapabilities`; a fill grant an agent asked for is claimed by the
  first extension served (`Grant.served_to`) rather than spent, and there is
  no `set-agent-mode` signal — the extension derives agent mode from the
  grants it holds (`04`, `05`); `list-visible` in Tier 1 reads the client's
  own rules and grants, never the baseline (`04`); the CLI binary is
  `rowel-cli`, a release-only sidecar reached as `rowel` through the symlink
  Settings installs, its agent key a `0600` file until I3 (`06`); passkey
  ceremonies carry `requestedBy` and write rows (`05`, `09`).
- **2026-10-25, I3 as built.** The sidecar: `resolve` and the placeholder
  (`rowel_broker::reference`) with strict parsing and name resolution by id
  or slug (`03`, `04`); the baseline rule admits `header` and `env`, a
  request's targets become per-method probes with the money floor lenient
  for an any-method request, and a secret grant counts its uses at `resolve`
  only (`03`, `09`); the dialog carries typed targets and "for N days" writes
  a rule for the request's own targets and mode (`06`); the CLI's second key
  and `rowel run`, `redact`, `hook` and `proxy` as built, with the one
  command-line quoting rule, the redactor's limits and the hook shapes of
  Claude Code, Codex and Cursor (`04`, `05`, Q16, Q17); CLI keys in the OS
  secure store on macOS and Windows (Q9).
- **2026-10-25, I4 as built.** The audit log syncs and is readable (`09`):
  `Snapshot.audit`, the union merge of rows by their whole contents (forks
  and altered copies kept beside, after review), the digest extended with
  each row's content fingerprint; retention changed
  from "a writer prunes its own rows" to "every device reclaims every writer's
  aged prefix, and only the chain's own writer notes the cut", because a union
  merge hands a device its own pruned rows back and iOS writes no rows of its
  own (Q21 ✅); the prune runs where the tombstone purge does, on a push, inside
  the key-checked reclaim; Vault Health › *Agent activity* and *Verify the
  log* with the two commands `audit_agent_activity` and `audit_verify`; the
  redactor-over-`detail` check left to the server tier. `docs/threat-model.md`
  gained its "Agents" section and `docs/agents.md` the user guide (`01`, `11
  §2 PR 8`); extension version 0.2.0.
- **2026-10-26, I5 as built.** The grant engine, the lockbox-side reads and
  the socket serve loop lifted out of the app into `rowel_broker::{agents,
  lockbox, server}`, TOTP into `rowel_core::otp`, so the app and the headless
  broker are thin hosts over one engine (`06`); `rowel serve` as the
  `rowel-serve` crate: unlock from a terminal, a key file or a descriptor,
  `serve.json` rules after the baseline, an approvals queue answered by
  `rowel approve` over a control socket, the socket (probed) and a WebSocket
  with bare frames, idle lock and SIGTERM (`03`, `04`, `07`); `ROWEL_BROKER`
  on the client side and clients skipping clear signals (`04`); Q9 and Q26
  annotated; I0–I4 ticked as merged in `PLAN.md`.
- **2026-10-27, Tier 1 audit and PR A.** An audit of I0–I7 on `main`: I7
  ticked as merged; items ticked without being delivered reopened (*this
  run*, the denied-fill fixture, the e2e dialog spec, `--install-host`, the
  I5 extension acceptance); the follow-up PRs A–D listed in `PLAN.md`. PR A
  (#666) recorded: the redactor at a constant lag (`05`), one expiry path and
  a wall-clock recheck (`03`, `06`), the audit failing closed with
  `AuditUnavailable` (`09`), the proxy forwarding only what a grant covers
  (`05`). The server mode moved to the private `fwdai/rowel-server` (`00 §4`,
  Q1).
- **2026-10-27, PRs B and C.** PR B (#667): the CLI on the PATH on Windows
  and from an AppImage, standalone CLI archives and extension zips in releases
  (`06`). PR C (#668): the run grant and its 8 h cap (`03`); exposure reports
  only from a grant's holder, `GrantExpired` on an expired grant (`04`); the
  shared manifest and relay, `rowel native-host`, `rowel serve
  --install-host`, the broker's own writer id (`06`).
- **2026-10-27, PR A merged; PR D.** The #666 review round: a grant use, a
  passkey and a standing rule commit with their audit row or not at all
  (`use_grant`, `SqliteStore::commit_with_audit`), and the proxy forgets a
  bucket once it has refilled (`05`, `09`). PR D (#669) removes the server
  mode from the public repository; the spec names `rowel-server serve` and
  `rowel-server admin` throughout (`00 §4`, `07`, `08`, Q1, Q19, Q21).

## Relationship to the other docs

- `docs/threat-model.md` is the security contract for what ships. This spec
  proposes changes to it (notably to "The browser extension": *a fill asks
  nothing* becomes mode-dependent). When a phase lands, the threat model is
  updated to say what the code does; this spec stays the *why* and the *plan*.
- `docs/browser-extension.md` and `docs/safari-extension.md` describe the
  extension fork this spec extends.
- `docs/sync-providers-design.md` describes the sync engine the server tier
  reuses as its device-sync transport.
