# 02 — Identity: who is asking

## 1. Three questions every request answers

Before the broker decides anything it needs to know three things, and it needs
proof for each:

1. **Which agent** is this? A named thing an admin or the user recognises.
2. **Which run** of it? So two jobs of the same agent are told apart in the log.
3. **For whom?** The principal: a person, when the agent acts on someone's
   behalf; or nobody, when it is an unattended job.

Rowel already answers the first for browser extensions: an extension holds a key,
the user names it in a dialog, and the name and key are remembered inside the
lockbox it was let into. Agents extend this, not replace it.

## 2. Tiers of evidence

```
  weakest ───────────────────────────────────────────────────────▶ strongest

  Tier 1: local association          Tier 2: workload identity      delegated
  ─────────────────────────          ─────────────────────────      ─────────
  OS user on an owner-only socket    a token the platform minted:   a human's
  + a client key the user named      GitHub Actions OIDC, k8s       device signed
    in the app's dialog                service-account JWT, SPIFFE  "this agent
                                       SVID, generic OIDC           acts for me"
  "the thing on my machine I         "the job in repo X on branch   on top of either
   said yes to"                        Y, workflow Z"
```

- **Local association (✅ for extensions, 🟡 for agents).** The agent connects
  to the socket, exchanges session keys, and asks to associate. The dialog says
  it is an *agent*, which platform it claims to be, and asks for a name. The
  name and key are stored with the lockbox. Good enough for one person's
  machine, which is what Tier 1 is.
- **Workload identity (❌, Tier 2).** The agent presents a token its platform
  minted. The server verifies the signature against the issuer and checks the
  claims against a *trust policy* an admin wrote ("repo `acme/deploy`, branch
  `main`, workflow `release.yml`"). A match maps to an **agent definition** the
  admin created. No shared secret is ever provisioned to the box.
- **Delegation (❌).** When a human starts an agent that will act for them (an
  employee's local coding agent, a personal assistant on a phone), the human's
  enrolled device signs a short statement naming the agent and a scope. The
  run then carries both identities, and the log says "agent A, for person P".

## 3. Agent definitions

In Tier 2 an agent is a record an admin owns: a name, an owner team, one or more
trust policies that identify it, and the policies that say what it may ask for.
In Tier 1 the same record exists implicitly: the row the association created,
plus whatever standing grants the user gave it.

Agents are never anonymous and never "the vault". Revoking an agent is deleting
or disabling that record; every open session checks it on the next request, as
the extension's association is checked today.

---

### Technical details

**Client record (✅ I2).** `rowel_broker::actions::Client` extends the old
`{ name, key }`:

```rust
#[serde(rename_all = "camelCase")]
pub struct Client {
    pub name: String,
    /// Identification public key (base64), as today. The whole credential in
    /// Tier 1; in Tier 2 the session key the workload proof was bound to.
    pub key: String,
    #[serde(default)]
    pub kind: ClientKind,              // Extension (default) | Agent | Sidecar
    pub platform: Option<String>,      // "claude-code", "codex", "cursor", "browser-use", "cli"
    #[serde(default)]
    pub created_at_ms: i64,            // 0 for a row from before kinds
    pub last_seen_at_ms: Option<i64>,  // set once per connection, at test-associate
    /// Tier 2 only: the agent definition this client resolved to.
    pub agent_id: Option<String>,
}
```

Stored where `browser::clients` rows are today (sealed inside the lockbox,
`clients_in` / `write_clients` in `src-tauri/src/browser/mod.rs`); serde
defaults make an old `{"name","key"}` row read back as an extension created
at time zero, which is what it was. The `_ms` suffix follows the codebase's
convention for millisecond timestamps (`Grant.expires_at_ms`).

**Association for agents (✅ I2).** The sealed `associate` and `test-associate`
messages accept an optional `hello { kind, platform, version, capabilities }`
(`proof` comes with Tier 2). The `Host` method is
`associate(&self, key: &str, who: &Hello) -> Result<Client, Code>`; the
connection keeps the *stored* record it matched and looks it up again on every
request, so the kind that gates an action is the lockbox's, never the
connection's claim. `AppHost` refuses an agent with `AgentsDisabled` before
any dialog while the app's Agents switch is off; otherwise the same
`browser:associate` event carries `kind` and `platform` and the one
`AssociateDialog` reads them (an agent is named after its platform by default
and told what it will and will not be able to do). The stored row gets
`created_at_ms` and an `associated` audit row; a forget writes
`revoked_client`. One ask at a time stays.

**Run identity.** Every request after association carries `run_id` (agent
minted, opaque, ≤ 64 chars) and may carry `reason` (≤ 300 chars). The broker
records them; it never trusts them for authorization.

**Workload identity verification (❌, Tier 2).** `rowel_server::identity`:

```
  agent ──(1) GET /v1/agent/challenge ───────────────────▶ server
        ◀── nonce, audience ─────────────────────────────
        ──(2) WS /v1/agent  change-public-keys ─────────▶   (sealed channel up)
        ──(3) associate { proof: { kind: "oidc",
                                    token: <JWT aud=audience,
                                           bound: sha256(session pubkey) > } } ▶
                                                    verify sig via issuer JWKS
                                                    check exp, aud, nonce
                                                    match claims → trust policy
                                                    → agent definition
        ◀── { client, agent_id, policies_hash } ───────
```

- Verifiers are a trait: `trait IdentityVerifier { fn verify(&self, proof) ->
  Result<Verified> }` with implementations `Oidc { issuer, jwks_cache, required_claims }`
  (covers GitHub Actions, GitLab, Kubernetes with OIDC discovery, most cloud
  workload identity) and later `Spiffe` (mTLS SVID). AWS STS-style presigned
  `GetCallerIdentity` is a separate verifier and is deferred (`10`).
- **Binding.** The OIDC token's `aud` must equal the challenge audience and the
  token must be requested *after* the session key exists; the agent puts
  `sha256(session_pubkey)` into a claim the platform lets it set (GitHub
  Actions and Kubernetes projected tokens both let the requester choose the
  audience, so the audience itself is
  `rowel:<tenant>:<sha256(session_pubkey)>`). A token replayed on another
  channel then fails the audience check. Where a platform cannot customise the
  audience, the fallback is a fixed audience plus the challenge nonce echoed in
  the sealed `associate` message, a token lifetime the trust policy caps at ten
  minutes, and a `jti` replay cache on the server for that window. The replay
  cache runs for every verifier regardless.
- **Trust policy** is data: `{ issuer, claims: { repository: "acme/deploy", ref:
  "refs/heads/main", workflow: "release.yml" } }`. Exact match per claim, with
  glob allowed on values an admin marks as such.

**Delegation proof (❌).** A compact signed statement by a device signing key
minted beside the X25519 enrollment key when delegation lands (Q7 settled the
enrollment key itself to one curve in I6; the proof's format stays open):

```
  { v: 1, device: <device_id>, agent: <agent_id or client key>,
    scope: { lockboxes: [...], ttl: <secs> }, iat, exp }  + signature
```

The server verifies it against the member's enrolled devices and attaches
`acting_for = member_id` to the session. In Tier 1 no proof is needed: the OS
user is the principal. The format deliberately stays inside the sealed channel
for v1; converging on OAuth token exchange with an `act` claim (RFC 8693) is a
later interoperability step, not a v1 requirement.

**Device enrollment (🟡 app side built in I6; the server is I7).** A member
signs in and the app submits its enrollment public key (`01`). The new device
is *pending* until an already-enrolled device of the same member, or an
admin, approves it; only then does the server return wrapped lockbox masters
to it. A stolen single sign-on session therefore cannot mint a device that
reads org lockboxes. The first device of a member is approved by an admin or
by an invitation code the admin issued. Every enrollment and approval is an
audit row.

*As built in I6* (`src-tauri/src/org`): the app never talks to the company's
identity provider itself. The Rowel Server is the OAuth2 authorization server
the app signs in against — authorization code with PKCE, the public client id
`rowel-app`, a loopback redirect, the same flow the cloud providers use — and
federates to the IdP on its own side (I7), so one flow in the app covers every
company and the server keeps the IdP tokens that SCIM and offboarding need
(I10). Enrollment is then a bearer call, `POST /v1/members/enroll
{devicePubkey, deviceName}` → `{deviceId, state}`; approval is `POST
/v1/members/devices/{id}/approve` from an approved device of the same member,
after the member compared the pending device's fingerprint (`01`) in
Settings; `GET /v1/members/me/devices` lists them with their public keys so
every device computes the same fingerprint. Desktop ships the connect flow;
the phone's deep-link twin of the sign-in is a later iteration.

**Revocation.** `Host::clients()` is consulted on every request today
(`Connection::associated` is re-checked against the open lockbox's list). The
server does the same against its agent and client tables; disabling an agent
definition ends every session on its next request. No tokens to expire, no
cache to flush.
