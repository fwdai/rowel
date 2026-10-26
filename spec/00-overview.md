# 00 — Overview

## 1. The problem in one paragraph

A service does what its code says. An agent does what its inputs say, and its
inputs are web pages, pull-request titles, emails and documents that an attacker
can write. So the machine an agent runs on is a semi-hostile execution
environment, and every assumption behind "put the secret in an environment
variable on the box" fails there: the agent can read it, it is alive for every
task, it is unscoped, nothing attributes its use to a run or a person, nothing
can make one action wait for a human, and it cannot log into a website at all.
In 2026 this stopped being theoretical: a malicious PR title made three vendors'
coding agents post repository secrets back into GitHub, and an audit of a hosted
agent sandbox found environment variables, mounted files and the system prompt
all readable by the agent, with only credentials injected by a proxy *outside*
the sandbox safe.

Rowel's answer is a **broker**: the agent never receives a credential in any
mode. It asks; a person or a policy decides; Rowel *uses* the credential at the
edge, by filling a form, signing a passkey assertion, swapping a header at HTTP
egress, or injecting into one child process, and writes down who used what, for
whom, where, and why.

## 2. Principles

1. **The model never holds the secret.** Values exist in the broker and at the
   point of use. Agents work with placeholders and outcomes.
2. **Grants, not access.** The unit is a grant: this agent, this item, this
   target, this mode, this lifetime, this approval. Standing access is a grant
   with a long lifetime, visible and revocable like any other.
3. **The human stays the authenticator.** Phishing-resistant by construction
   where possible: a passkey assertion from the vault beats a filled password,
   and the approver's tap is the user-presence gesture.
4. **Attribution end to end.** Human → agent → run → action, in a log the user
   can read and a company can export.
5. **Local first, then up.** The free desktop app is the complete product for
   one person. The server adds *unattended* and *shared*, not *better*.
6. **One protocol, one core, three places to run it.** Nothing is written
   twice for the server that the app already does.
7. **Honest trust tiers.** Where the broker must be able to read a secret to
   act while nobody is present, say so in the UI, and keep it in a compartment
   the user chose.

## 3. The three tiers

```
                         ┌──────────────────────────────────────────────┐
  Tier 3                 │  Rowel Cloud                                 │  paid
  hosted                 │  = Tier 2 operated by us, multi-tenant       │
                         │  + KMS/HSM root keys  + push approvals       │
                         │  + SSO  + billing  + SIEM export             │
                         │  + approval relay for free users             │
                         └───────────────────▲──────────────────────────┘
                                             │  same binary, same API
                         ┌───────────────────┴──────────────────────────┐
  Tier 2                 │  Rowel Server (self-hosted)                  │  paid
  team / company         │  org lockboxes the broker can open           │
                         │  workload identity (OIDC/SPIFFE) for agents  │
                         │  policy, approval routing, audit export      │
                         │  devices sync org lockboxes through it       │
                         └───────────────────▲──────────────────────────┘
                                             │  same crates: core + broker
                         ┌───────────────────┴──────────────────────────┐
  Tier 1                 │  Rowel desktop app (+ extension, + CLI)      │  free
  one person,            │  the broker is the app over the local socket │
  one machine            │  approval is the app's dialog + biometrics   │
                         │  device-held lockboxes stay end-to-end sealed│
                         └──────────────────────────────────────────────┘
```

**Tier 1, local.** A developer runs Claude Code, Codex, Cursor, a local
browser agent or an agentic browser on their own machine. Rowel is already
running as the native host for the browser extension. Agents connect to the
same local socket the extension uses, associate like an extension does, and
ask for grants. The app's dialog asks the person. The extension fills, the
vault signs passkeys, the CLI injects into one child process. Nothing leaves
the machine. This tier is free, open source, and complete on its own.

**Tier 2, server.** A company has agents in CI, in Kubernetes, in a cloud
browser farm, and in employees' hands. Nobody is at a keyboard when most of
them run. The server holds **org lockboxes** whose keys it can open, verifies
agents by the identity their platform already gives them (a GitHub Actions
OIDC token, a Kubernetes service-account token, a SPIFFE SVID), evaluates
policy, routes approvals to the right person's Rowel app, and writes the
audit log. Employees' Rowel apps sync org lockboxes through the server using
the sync engine the app already has, so an org lockbox is one more lockbox
in the same vault behind the same master password.

**Tier 3, cloud.** Tier 2 as a service: we run it, multi-tenant, with the
root keys in a KMS, push notifications for approvals, SSO, billing and SIEM
export. It also offers the one thing the free tier cannot do without a
server: approving a desktop agent's request from a phone while away, through
a relay that carries sealed requests it cannot read.

## 4. How the tiers stack (reuse map)

The rule: **a tier adds a place to run the broker and a way to prove identity;
it does not add a second broker.** Two binaries carry it: the desktop app,
which embeds the broker and is it on a laptop, and the `rowel` CLI, whose
`serve` subcommand is the headless broker on a developer's VM, the self-hosted
server in `--server` mode, and the process inside Rowel Cloud's containers. The
app is not split into a UI plus a daemon: that would move the keys into a
second process for no security gain (Q26). The step-by-step build order is in
`PLAN.md`.

**Repositories and licenses.** Two repositories, two principles. Proprietary
code depends on open code, never the reverse; and the GPL obliges on
distribution, not on use. The public repository holds the apps and the
extension under GPL-3.0 and the library crates (`rowel-core`, `rowel-broker`,
`rowel-autofill`, the headless `rowel serve` for one person's lockbox, the CLI
client commands) under MIT OR Apache-2.0, plus the server API as
documentation. A private repository holds the server mode (tenants, members,
workload identity, policy store, routing, escrow, audit export), the
Cloudflare glue, billing, the admin web console and the relay; it pins a
tagged version of the public crates and builds its own server binary. The app
contains no proprietary code; its team-facing surface is small and generic
(connect, enroll, approve, manage my devices), and administration lives in the
console. An open client exposes the server contract by nature; the business
lives in the server and its operation, as Bitwarden and Tailscale show.
*As built in I7:* by the owner's decision the server mode is built for now in
the public repository, as its own crate (`src-tauri/crates/server`,
`rowel-server`, `publish = false`), so the boundary is a directory and moving
it to the private repository is a crate move. *Moved 2026-10-27:* the server
mode now lives in the private repository `fwdai/rowel-server`, one package
with its own binary (`rowel-server serve`, `rowel-server admin …`, replacing
`rowel serve --server` and `rowel admin server …`), depending on the public
`rowel-core`, `rowel-broker` and `rowel-serve` by a pinned git rev; the
public repository drops the crate, the admin commands and `docs/server.md`
(PLAN, PR D) and keeps `docs/server-api.md`, the contract its clients speak.

```
                 ┌────────────────────────────────────────────────────────────┐
                 │ rowel-core (crates/core)                            ✅     │
                 │ crypto (Argon2id → HKDF split, AES-GCM AEAD), SQLCipher     │
                 │ store, Entry model, passkey authenticator, OTP, host match │
                 └───────────────▲─────────────────────────▲──────────────────┘
                                 │                         │
   ┌─────────────────────────────┴───────────┐   ┌─────────┴──────────────────┐
   │ rowel-broker (crates/broker)        ✅  │   │ sync engine (src-tauri/   │
   │ sealed channel + framing  ✅            │   │ src/sync) ✅               │
   │ Host trait + Connection   ✅            │   │ Remote / LocalVault traits │
   │ passkey ceremonies        ✅            │   │ pack format, LWW merge     │
   │ grants + policy           ✅            │   └─────────┬──────────────────┘
   │ client kinds (ext/agent/  ✅            │             │
   │   sidecar) + audit        ✅            │             │
   │ placeholders + resolve    ✅ (I3)       │             │
   └───▲──────────────▲──────────────▲───────┘             │
       │              │              │                     │
 ┌─────┴─────┐  ┌─────┴──────┐  ┌────┴─────────────────────┴──────────────┐
 │ Rowel app │  │ rowel CLI ✅│  │ rowel serve ✅ (I5, headless)            │
 │ AppHost ✅ │  │ mcp, hook, │  │ HeadlessHost ✅ · OrgHost ✅ (org lockbox) │
 │ dialog ✅  │  │ run, proxy,│  │ workload identity verifiers ✅           │
 │ Settings ✅│  │ redact (I3)│  │ config rules ✅ · approval router ✅     │
 └───┬───────┘  └────────────┘  │ pack endpoints = Remote for devices ✅   │
                                 │   (device side ✅ I6, server ✅ I7)       │
     │                          └──────────────────────────────────────────┘
 ┌───┴─────────────────────────┐
 │ browser extension (fork) ✅  │   gains: agent mode, grant-driven fill,
 │ native messaging ✅          │   exposure check, cookie clearing, and a
 │ fill, TOTP, passkeys ✅      │   WebSocket transport to a server
 └─────────────────────────────┘
```

What each tier reuses from the one below:

| Capability | Tier 1 | Tier 2 | Tier 3 |
| --- | --- | --- | --- |
| Crypto, store, entry model, passkeys | core | core, unchanged | same |
| Sealed channel, framing, `Host` trait | broker crate | broker crate, `OrgHost` implements `Host` | same |
| Grants, policy evaluation, audit rows | broker crate | broker crate; policy *storage* is the server's | same |
| Consent UI | app dialog | app dialog, request arrives from the server | same, pushed |
| Identity proof | OS user + association key | + OIDC/SPIFFE verifiers | + SSO for humans |
| Device sync of lockboxes | Drive/Dropbox/iCloud providers | + the server as a provider | same |
| Extension fill path | native messaging | + WebSocket to server for remote browsers | same |

## 5. Why a company pays

Not for storage. They pay because:

- **The agent box is hostile.** Secrets come off it entirely; a leaked transcript
  or a prompt injection yields placeholders.
- **Web logins, API keys, passkeys and OAuth tokens under one policy.** Secrets
  managers stop at API keys; env vars cannot log into Salesforce. The incumbent
  password manager needs two products and a partner badge for the web half.
- **Attribution per run** for audit and compliance: which agent, acting for whom,
  did what, approved by whom.
- **Blast radius.** Revoke one agent's grant instead of rotating a key everywhere.
  Keys are never cached on boxes; rotation needs no redeploy.
- **Human-in-the-loop as policy, not prompt engineering.** A refund endpoint waits
  for a tap on the finance lead's phone; a read endpoint does not.
- **Laptop to prod with no code change.** Developers already run Tier 1 for
  free; the same placeholders resolve against the server in CI.
- **Self-hostable, open core.** Regulated companies run it inside and can read it.

## 6. Glossary

- **Broker** — the component that answers agent requests and holds the keys to
  do so: the desktop app in Tier 1, the server in Tiers 2 and 3.
- **Agent** — a program acting for a principal: a coding agent, a browser agent,
  a CI job, a scheduled worker. Identified by a *client key* locally or a
  *workload identity* remotely.
- **Principal** — the human (or team) an agent acts for. In Tier 1 it is the OS
  user. In Tier 2 it is a member, and may be absent for unattended agents.
- **Run** — one execution of an agent. Carries a `run_id` the agent mints.
- **Grant** — the unit of access (see `03`): agent, principal, items, targets,
  modes, approval, lifetime, reason.
- **Delivery mode** — how a credential is used without being handed over:
  `fill`, `passkey`, `header`, `token`, `env` (see `05`).
- **Approver** — the human who answers an `ask`; in Tier 1 the user, in Tier 2
  whoever policy routes it to.
- **Client kind** — what sits on the other end of the sealed channel:
  `extension`, `agent`, or `sidecar` (the CLI/proxy that may receive values
  for `header` and `env` modes).
- **Org lockbox** — a server-held lockbox (Tiers 2 and 3) whose random master
  is escrowed to the broker so it can act while nobody is present. A
  *device-held lockbox* is what Rowel has today: password-derived, opaque to
  every server.
- **Agent access** — a per-lockbox switch, off by default, that allows rules
  and grants to be written for items in it. Independent of where the lockbox
  is held (see `01`).
