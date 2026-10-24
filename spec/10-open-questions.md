# 10 — Open questions

Everything in this spec that is not settled, in one place. Each entry says what
is uncertain, the options, what the spec assumes meanwhile, and who decides.
When one is resolved, record the decision in the project context and update
the section it belongs to; leave the entry here with a ✅ and a pointer.

## Product and business

**Q1. Server license and open-core boundary.** ✅ *Settled 2026-10-24.*
The library crates under `src-tauri/crates` are MIT OR Apache-2.0; the apps
and the extension stay GPL-3.0; the server mode and Rowel Cloud are
proprietary in a private repository that depends on the public crates;
contributions are inbound = outbound with a DCO sign-off. See `00 §4`,
the README License section and `CONTRIBUTING.md`. What remains open is only
whether the single-node server is ever offered as a supported product (Q2).

**Q2. Self-host first or cloud first.** *Owner decides.*
Spec assumes self-host binary first (`11`), because the cloud is that binary
plus operations, and because early design partners will want to run it inside.

**Q3. User-facing names.** *Owner decides.*
For the server-held, escrowed lockbox: "org lockbox", "team lockbox", "shared
lockbox", "company lockbox"; the word must carry the warning that the server
can open it. For the per-lockbox switch: "Agents can use this lockbox" or
"Agent access". Spec uses *org lockbox* and *agent access*.

**Q4. Billing unit.** *Owner decides.* Spec assumes seats plus metered grants
(`08`), chosen so nobody is tempted to collapse lockboxes or agents.

**Q24. An optional account in the free tier.** *Owner decides.*
The approval relay (`08`) needs the phone registered for push and some
per-user rate limiting, which means a free account used for nothing else.
Rowel's positioning is "no account". Options: (a) offer the relay as the one
opt-in account feature and say so plainly; (b) no relay in the free tier;
(c) a pairing-only relay keyed by the phone's public key with no account,
accepting weaker abuse controls. Spec assumes (a).

## Security and standards

**Q5. Passkeys from server-held lockboxes, and the FIDO stance.** *Decide at phase 4.*
Tier 1 passkey use by agents keeps the human's tap on the device holding the
key, which is within the spirit of WebAuthn's user-presence requirement. A
server signing with a key it holds, gated by an approver's tap on a different
device, is a stretch of that requirement. FIDO's Agentic Authentication
working group (announced April 2026) has not settled delegation. Options:
(a) refuse passkeys in server-held lockboxes until standards land; (b) allow
with `step_up` floor and clear labelling; (c) allow only for relying parties
the tenant owns. Assumed: (a) in phase 2, revisit at phase 4. Settled for
Tier 1 regardless: a passkey grant is never `auto`; the approver's tap on the
device holding the key is the user-presence gesture (`03`).

**Q6. Relying parties fingerprinting agent sessions.** *Watch.*
Research exists on fingerprinting browsing agents; banks say they detect
agentic activity. A fill or assertion from Rowel is indistinguishable from the
user's, which is the point, but sites may ban automation by terms. Rowel
should not pretend otherwise: the grant dialog can carry a one-line notice for
item tags the user marks `financial`.

**Q7. Delegation proof format.** *Decide at phase 2.*
Custom signed statement inside the sealed channel (spec default) vs OAuth
token exchange (RFC 8693) with an `act` claim. The custom format ships first;
interoperability with IdPs comes when a customer needs the server to accept
tokens their IdP minted. Also: whether the enrollment key is one X25519 key
used for sealing and a derived Ed25519 for signing, or two keys.

**Q8. Workload identity breadth.** *Decide per customer.*
Generic OIDC covers GitHub Actions, GitLab, Kubernetes with OIDC discovery,
GCP, Azure. AWS roles do not issue OIDC tokens to arbitrary audiences; the
usual trick is a presigned `sts:GetCallerIdentity` request the server replays.
SPIFFE needs mTLS at the WebSocket. Order: OIDC → AWS STS → SPIFFE.

**Q9. Same-user local processes can lift client keys.** *Accepted residual.*
As for the extension today. Mitigations: the CLI stores its two keys (agent
and sidecar) in the OS secure store (`secure_store.rs` has the abstraction;
it is biometric-gated today, a non-gated item is a small addition); grants,
floors, full-command-line targets and rate limits bound what a lifted key
buys; audit shows it. Not solvable without OS-level process identity.

**Q10. Agent access on device-held lockboxes while unlocked.** *Design is settled; wording is not.*
In Tier 1 there is no escrow: a standing `auto` rule serves an agent without a
dialog for as long as the vault is unlocked, and nothing at all once it locks.
The switch's copy must say this plainly. Consider a shorter auto-lock default
for a vault that holds standing `auto` rules, and a Vault Health nudge when
such rules exist on the Personal lockbox rather than a dedicated one.

## Browser and fill

**Q11. Detecting an agentic browser without its cooperation.** *Experiment in phase 0.*
`chrome.management` can tell whether an extension such as Claude in Chrome is
installed and enabled, not whether it is driving right now. Heuristics
(synthetic input events, debugger attached via `chrome.debugger` detection)
are unreliable. Spec default: an explicit per-profile agent-mode toggle plus
explicit integration through the protocol. Revisit if Chrome exposes a signal.

**Q12. Session end scope and profile isolation.** *Experiment in phase 0.*
Deleting a site's cookies signs the human out too if they share the profile.
Better: a dedicated browser profile or Firefox container per agent. Can the
extension create or steer one? Not in Chromium from an extension; the agent
platform must launch the profile. Spec: delete cookies per registrable domain
with the `cookies` permission the manifest already has (no new prompt), warn
in the dialog, document "use a separate profile for agents". Clearing
localStorage and IndexedDB too needs the `browsingData` permission and is a
phase 4 addition if cookies prove insufficient for real sites.

**Q13. Exposure check reach.** *Accepted limit.*
Text and input values only; not pixels, not shadow roots the page closed, not
iframes from other origins. State it.

**Q14. Safari.** *Defer.*
No `browsingData`, no native messaging (the App Group socket is the path),
and agentic browsers are not Safari. Agent mode in Safari is read-only parity
at best in phase 0.

## Proxy and tooling

**Q15. HTTPS egress: path-prefix rewrite vs CONNECT with a local CA.** *Decide after phase 1 feedback.*
Path-prefix (`/https/<host>/…`) needs no certificates and works with every SDK
that allows a base URL. A real forward proxy needs a local CA installed in the
agent's trust store, which is intrusive on a laptop and normal in a pod. Ship
path-prefix; add CONNECT for Tier 2 sidecars if customers ask.

**Q16. Redaction limits.** *Accepted limit, document it.*
Exact match plus common encodings. A value split across stream chunks or
transformed (hashed, partially shown) passes. Consider a streaming matcher
with a window equal to the longest value plus encoding overhead.

**Q17. MCP tool surface vs hooks.** *Experiment.*
Some agents will only have MCP; some only hooks; some both. The MCP server
alone cannot stop the agent from running `env`; the hook can rewrite commands.
Ship both; measure which developers use.

## Server

**Q18. Storage: SQLCipher file per lockbox vs rows in Postgres.** *Decide at phase 2.*
Files keep format identity with the app (the pack *is* the file) and make the
merge engine reusable as-is. Rows in Postgres scale and index better. Spec:
files per lockbox, metadata in Postgres/SQLite, shard by lockbox when needed.
Revisit at the first tenant with thousands of lockboxes.

**Q19. Merge authority and write interleaving.** *Design chosen; validate with tests.*
The server merges device pushes with `engine::sync` and bumps the revision on
every agent write, so device CAS retries. Needs a test matrix: agent
`save_login` during a device push; two devices and one agent; a device offline
for longer than tombstone TTL.

**Q20. Approval latency and fatigue at scale.** *Learn from design partners.*
`ask` for every CI fill does not scale; `auto` for everything defeats the
point. Levers: standing rules with expiry, batched approvals ("approve the
next 20 uses of this item by this agent for an hour"), and sensible defaults
per item kind. Spec ships the levers, not the defaults.

**Q21. Audit chain with many writers and pruning.** *Design chosen; verify.*
Per-writer chains and a `pruned_through` row keep verification possible; an
auditor's tooling has to understand both. Provide `rowel admin audit verify`
and a documented algorithm.

**Q22. Remote extension in cloud browsers.** *Phase 4.*
The enrollment-code handshake, how the agent platform injects the extension
and the code (Browserbase, Kernel, Anchor each differ), and whether the
extension's WebSocket transport should be a separate build. Also whether to
offer a headless "fill service" (Rowel drives a browser it owns) instead of
depending on the platform's extension support.

## Cloud and packaging

**Q25. Cloudflare building blocks to confirm in the I9 spike.** *Engineering.*
Whether a WebSocket can be passed from the Worker or Durable Object into the
container (the docs show HTTP forwarding to a container port; if WebSocket
pass-through is awkward, agents use HTTPS POST per frame, which the one
request, one reply channel allows with no protocol change); whether Secrets
Store has left beta or a Worker secret binding is the root-key home for now;
container cold start and sleep behaviour against the approval latency target.
None of these changes the design, only the first implementation.

**Q26. A daemon under the desktop app.** *Decided: no, for now.*
Reuse is at the crate level: the app embeds the broker; the CLI embeds the
same crates and runs it headless with `serve`. Splitting the app into a UI
and a daemon would move keys into a second process, add an IPC trust boundary
between two same-user processes, and complicate biometric unlock and iOS for
no security gain. Revisit only if a concrete need appears, such as serving
agents while the GUI is not running on a desktop OS.

## Process

**Q23. Threat-model updates per phase.** *Settled process.*
Each phase's last PR updates `docs/threat-model.md` to say what the code does,
and the README's claims stay worded to what shipped (an existing project
decision).
