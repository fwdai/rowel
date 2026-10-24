# 03 — Grants and policy

## 1. The grant is the unit

A grant says: *this agent* may use *these items* at *these targets* in *these
modes* until *this time*, and whether a person must say yes first. Nothing
else confers access. A fill consumes a grant. A header swap at the proxy
checks a grant. A standing permission is a grant with a long lifetime and
`auto` approval, listed in Settings like any other, and revocable there.

This replaces three things people reach for today and regret: the whole vault
behind one token, an environment full of keys, and a yes/no dialog with no
memory. It also gives the server tier and the local tier the same vocabulary,
so a policy a developer tests on their laptop reads the same in CI.

```
  ┌──────────────────────────────────────────────────────────────┐
  │ grant                                                        │
  │  who:     agent (+ acting_for principal, optional)           │
  │  what:    items (ids, or a selector: lockbox / tag / kind)    │
  │  where:   targets (origins for fill/passkey, host+path+method│
  │           patterns for header/token, full command lines for  │
  │           env)                                               │
  │  how:     modes  (fill | passkey | header | token | env)      │
  │  when:    not_before, expires_at, max_uses                   │
  │  gate:    approval (auto | ask | step_up), reason_required   │
  │  trace:   run_id, reason, approver, created_by (user|policy) │
  └──────────────────────────────────────────────────────────────┘
```

## 2. Approval modes

- **auto** — covered by a standing grant or a policy rule; no human in the loop.
  The default for low-risk, read-only API targets in CI.
- **ask** — one tap on the approver's device for this request, with the item,
  the target and the agent's reason on screen. The default for web logins.
- **step_up** — a tap *plus* biometrics (Touch ID, Face ID, Windows Hello) on
  the approver's device, or the master password where the device has no
  biometrics. Floored on for cards, identities, and any target a policy marks
  as money or identity.

Two floors that no rule can lower:

- **Passkeys are never `auto`.** The approver's tap is the user-presence
  gesture WebAuthn requires. Removing it would turn the vault into the
  "virtual authenticator in production" anti-pattern.
- **`env` mode is at least `ask`** unless a rule names the full command line,
  because the child process can print what it was given.

A request the policy does not cover is **denied**, not asked. Asking is itself
something policy grants, so an agent cannot spam approvers for items it was
never meant to touch, and a client that keeps being denied is slowed down.

## 3. Policy

Policy is a list of rules evaluated deny-by-default. A rule matches on agent,
items and targets, and yields the approval mode and limits. In Tier 1 the user
writes rules by answering dialogs ("allow Claude Code to use GitHub logins for
7 days without asking"); in Tier 2 admins write them, and teams can be delegated
their own lockboxes' rules.

Keep it simple at first: flat rules, most specific wins, floors cannot be
lowered by a less specific rule. Policy-as-code (Cedar or similar) is a later
layer on the same evaluation interface, not a prerequisite.

## 4. Approval routing (Tier 2)

In a company, the person who should say yes is not whoever is online. A request
is routed to the item's owner, or to an approver group the rule names, or to
the principal the agent acts for. It waits a bounded time, then expires as
denied. Approvers see one request at a time, as the app's dialog does today,
and the rest queue on the server. Push notification in Tier 3; the app polls
a short-lived channel in Tier 2.

---

### Technical details

**Types (❌, `rowel_broker::grant`).**

```rust
pub struct Grant {
    pub id: String,                 // 16 random bytes hex (`crypto::random_hex_id`)
    pub lockbox_id: String,
    pub client_key: String,         // the agent client it was issued to (Tier 1) ...
    pub agent_id: Option<String>,   // ... or which agent definition (Tier 2)
    pub acting_for: Option<String>,
    pub items: ItemSelector,        // Ids(Vec<String>) | Lockbox | Tag(String) | Kind(EntryKind)
    pub targets: Vec<Target>,
    pub modes: Vec<Mode>,           // Fill | Passkey | Header | Token | Env
    pub approval: Approval,         // Auto | Ask | StepUp
    pub reason_required: bool,
    pub not_before_ms: i64,
    pub expires_at_ms: i64,
    pub max_uses: Option<u32>,
    pub uses: u32,
    pub run_id: Option<String>,
    pub reason: Option<String>,
    pub approver: Option<String>,   // "user" in Tier 1; member id in Tier 2
    pub created_by: CreatedBy,      // User | Policy(rule_id) | Admin(member_id)
    pub state: State,               // Pending | Active | Spent | Expired | Revoked | Denied
}

pub enum Target {
    /// fill / passkey: an origin, matched with `rowel_core::host::site_matches`
    /// semantics (scheme, host or parent, port).
    Origin(String),
    /// header / token: host (exact or `*.`), path glob, methods.
    Http { host: String, path: String, methods: Vec<String> },
    /// env: a glob over the *full* command line (`node deploy.js*`), never
    /// `argv[0]` alone — `node -e "print(env)"` must not match a rule for
    /// `node deploy.js`.
    Command(String),
}
```

**The grant id is a capability.** It is unguessable (128 bits) and it is the
only thing an agent hands to a sidecar (`X-Rowel-Grant` header to the proxy,
`--grant` to `rowel run`). `resolve` requires a client of kind `sidecar`, the
grant in state `Active`, the reference and target inside it, and in Tier 1 the
sidecar associated to the same lockbox; in Tier 2 additionally a policy
binding `sidecar_for: [agent_id]` (or the same workload identity namespace),
so a sidecar cannot resolve grants of agents it does not serve.

**Lifecycle.**

```
            request-grant
                 │
        lockbox agent-access on?  ──no──▶ Denied
                 │ yes
        policy.evaluate(agent, items, targets, modes) + floors
                 │
      ┌──────────┼──────────────┬───────────────┐
      ▼          ▼              ▼               ▼
   no rule     Auto           Ask            StepUp
      │          │              │               │
   Denied     Active     Pending ──ask──▶   Pending ──ask + biometric/password──▶
                              │  yes/no/timeout        │
                              ▼                        ▼
                        Active | Denied          Active | Denied
                 │
   use (fill / resolve / sign) → uses += 1; Spent when max_uses reached
                 │
   expires_at → Expired; end-grant or Settings → Revoked
```

**Policy evaluation (❌, `rowel_broker::policy`).**

```rust
pub struct Rule {
    pub id: String,
    pub agents: AgentSelector,      // Any | Ids(..) | Platform(..)
    pub items: ItemSelector,
    pub targets: Vec<Target>,
    pub modes: Vec<Mode>,
    pub approval: Approval,
    pub ttl_ms: i64,
    pub max_uses: Option<u32>,
    pub reason_required: bool,
    pub approvers: Option<ApproverSelector>,  // Tier 2: Owner | Group(id) | Principal
    pub sidecar_for: Option<Vec<String>>,     // Tier 2: which sidecars may resolve
    pub created_at_ms: i64,
    pub created_by: CreatedBy,
    /// A standing rule's own end ("for 7 days"); `None` for a rule that stays
    /// until removed. The host drops dead rules (`Rule::is_live(now_ms)`)
    /// before evaluating, because `evaluate` has no clock.
    pub expires_at_ms: Option<i64>,
}

pub fn evaluate(rules: &[Rule], req: &GrantRequest, candidates: &[CandidateItem],
                probes: &[Probe], floors: &Floors) -> Decision
```

`candidates` are the items the request's selector resolved to (`id`, `kind`,
`tags`, nothing secret) and `probes` the concrete places asked for (the page
URL for a fill, host + path + method for a header, the full command line for
`env`); the host resolves both, the evaluator stays pure.

- Specificity order: item id > tag > kind > lockbox; within equal specificity,
  the stricter approval wins. `Floors` are non-negotiable minimums: `card` and
  `identity` kinds and `Http` probes matching a target in `Floors.money` (a
  list policy fills; empty by default) evaluate to at least `StepUp`;
  `Passkey` mode is at least `Ask`; `Env` mode is at least `Ask` unless a
  rule's `Target::Command` matches the full command line. A rule with an empty
  `targets` list matches any target.
- **Tier 1 baseline.** Deny-by-default stays the evaluator's rule; the
  desktop app supplies the baseline a person expects by prepending
  `policy::defaults::personal()` whenever the lockbox's agent-access switch
  is on: one rule, any agent, any item in the lockbox, any target, modes
  `fill` and `passkey`, approval `ask`, and a grant that lives 15 minutes with
  no use cap. A fill grant is deliberately not single-use: the fill is
  followed by the TOTP step under the same grant, username-first sign-ins ask
  for the page's logins twice, and a failed submit is retried; the window
  bounds all of it. So the first ever fill asks instead of failing, and
  everything the user answers "for N days" to becomes a more specific `auto`
  rule on top of it (`policy::defaults::standing`, which carries the rule's
  own `expires_at_ms`). While a fill grant is active, a second
  `get-logins` from the same client for the same origin is served under it
  rather than asked again: the extension sends the `grantId` it was given,
  and the host also looks for a live grant of that client covering the
  request before it evaluates policy.
- Pure function over data, with tests in the broker crate; no I/O. The same
  `evaluate` runs in every tier.
- **Rate limits** (`rowel_broker::policy::Limits`): per client, at most 10
  `request-grant` calls per minute that reach a person, and after three denials
  in a row a cool-down that doubles from 30 s. Exceeding either answers
  `GrantDenied` without a dialog and writes an audit row. Rules may raise the
  limits for a specific agent.

**Where rules and grants live.** In Tier 1 they are *device-local*, like the
extension associations today (`browser::clients`, a sealed JSON value under a
`meta` key): a rule names a client key that exists only on this device, so
syncing it would carry nothing another device can use. Store them as sealed
JSON under `meta` keys `agent_rules` and `agent_grants` (active and standing
ones; in-memory `Grants` on `AppState` for the rest). The sync merge ignores
`meta`, so nothing changes in the engine. In Tier 2 rules and grants live in
the server's database per tenant.

**Standing grants in Tier 1 UI.** The approve dialog offers "just this once",
"for this run", and "for N days" (7 by default). The last writes a `Rule` with
`agents: Ids([client_key])`, `items: Ids([item])`, `targets: [the origin]`,
`approval: Auto`. Settings › Browser & agents lists rules and active grants with
revoke; Vault Health shows standing rules older than 30 days.

**Approval in Tier 1: block, like `associate`.** `AppHost::request_grant` runs
policy under the session lock, releases it, and asks through
`Pending<Answer>` (`src-tauri/src/browser/mod.rs`) with a new variant
`Answer::Grant(Option<GrantAnswer>)`, exactly as `associate` asks for a name:
the connection's thread blocks for up to `CONSENT_TIMEOUT` (60 s) and the
reply carries the final state. `GrantAnswer { item_id, standing: Option<ttl> }`
lets the user *swap* the item (the dialog lists every item the selector
admits, as the passkey dialog lists accounts). The epoch check after the answer
(`session.store_at(epoch)`) discards a grant whose lockbox changed hands while
the user decided. When the consent slot is already taken by another ask, the
reply is `pending` with `retryAfterMs`, not a refusal, so the agent retries
after the other dialog closes rather than failing. `wait-grant` therefore
matters mostly in Tier 2.

**Rows and uses, as built.** `requested` is written for an ask that reached
the screen (first row of the settle step) and for an `auto` decision; a
`pending` reply writes nothing, so a retry after a busy slot does not double
the log. A reply of `active` is a use: the host counts it (`use_once`) and
writes `used` at once, because dispatch serves the admitted item immediately.
`grant_covers` (the `grantId` paths of `get-logins` and `get-totp`) is a
predicate with a side effect, on purpose: a yes there is a use too, counted
and written with detail `{"via":"grantId"}`; a reuse before policy writes
`{"reuse":true}`. Turning the Agents switch off, or a lockbox's agent access
off, revokes every active grant (`revoked`, reason `switch` or `lockbox`);
every lock clears grants and rate state outright, with no rows, because
nothing is served from a locked vault.

**Step-up in Tier 1.** `biometrics::authenticate()` where `secure_store::SUPPORTED`
(macOS, iOS, Windows); elsewhere (Linux) the dialog asks for the master
password and verifies it against the open lockbox's KDF sidecar.

**Candidates in replies.** `request-grant` returns the admitted items' `id`,
`title` and `username` so the agent can tell the user which account it will
use. Usernames are already list-visible metadata in Rowel; a lockbox-level
`hide_usernames` flag is a later addition if a customer needs it.

**Server routing (❌, Tier 2).** `pending_approvals` table: grant id, approver
candidates, created, expires, state. The approver's app subscribes
(`GET /v1/members/me/approvals/stream`, server-sent events or long poll) and
answers (`POST /v1/approvals/{id}`) with the same `GrantAnswer`; the server
resolves the grant and the waiting `wait-grant` call on the agent's channel
returns. First answer wins; the others are told it was handled. Per-rule
timeout: default 120 s interactive, up to 24 h for queued approvals of
unattended agents, after which the run is told `Denied` and may retry within
the rate limits.
