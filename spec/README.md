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

## Relationship to the other docs

- `docs/threat-model.md` is the security contract for what ships. This spec
  proposes changes to it (notably to "The browser extension": *a fill asks
  nothing* becomes mode-dependent). When a phase lands, the threat model is
  updated to say what the code does; this spec stays the *why* and the *plan*.
- `docs/browser-extension.md` and `docs/safari-extension.md` describe the
  extension fork this spec extends.
- `docs/sync-providers-design.md` describes the sync engine the server tier
  reuses as its device-sync transport.
