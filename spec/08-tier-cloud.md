# 08 — Tier 3: Rowel Cloud

## 1. The same server, run by us

Rowel Cloud is Tier 2 operated as a service. A company that does not want to
run a server signs up, connects its identity provider, and gets the same
product: org lockboxes, agent identity, policy, approvals, audit. The binary
is the same; the difference is who holds the root keys, who is on call, and
the features that only make sense with a service in the middle.

## 2. What only a service can add

- **Push approvals.** A request reaches the approver's phone as a notification
  with the item, the agent and the reason, and is answered with Face ID. Tier 2
  can only poll; a hosted service has the Apple and Google push credentials.
- **The approval relay for free users.** A person on Tier 1 whose desktop agent
  needs a yes while they are away can route the ask through the cloud to their
  phone. The relay carries a sealed request it cannot read. It is opt-in and
  is the one thing in the free tier that needs an account, for the relay alone
  (Q24); it is also the on-ramp to the paid product.
- **SSO and SCIM.** Members come and go with the company's directory.
- **Managed keys.** Root keys in an HSM-backed KMS per region; no operator ever
  holds one.
- **SIEM export, retention, residency.** Audit streams to the company's tools;
  data stays in the region they chose.
- **Billing.** Seats for members, a metered component for agent runs or grants.

## 3. What stays the same

The agent protocol, the policy model, the audit schema, the app's experience,
and the honesty of the trust model: an org lockbox in the cloud is readable by
the service while serving a grant, and device-held lockboxes never touch it
unless the user chooses the cloud as a sync provider, in which case it sees an
opaque pack like Google Drive does.

---

### Technical details

**Tenancy.** One deployment, many tenants, `tenant_id` on every row and in
every HKDF label (`07`). Lockbox files per tenant on encrypted volumes; the
file is SQLCipher under a key the service only has while serving, so a volume
snapshot is ciphertext. Per-tenant root key derivation from a regional KMS key
(`RootKey` trait, `07`), with the option of a customer-managed KMS key for
enterprise tenants (the trait makes this a configuration, not a feature).

**Push.** APNs and FCM senders behind an `ApprovalNotifier` trait with the
Tier 2 implementation being "none, the app polls". The notification body
carries only the request id and a short title; the app fetches details over the
authenticated stream, so a notification on a lock screen does not leak the
item. iOS: the existing Rowel iOS app registers a device token at enrollment.

**Approval relay for Tier 1 (❌).** The desktop app seals the `AskPayload` to
the user's own phone's enrollment key (the X25519 key every install generates
per `01`), posts it to `POST /v1/relay/asks`, and polls
`GET /v1/relay/asks/{id}`. The phone receives a push, fetches the sealed blob,
unseals it, shows the same dialog, seals the answer back. Pairing: the phone
shows a QR code carrying its public key and relay device id; the desktop scans
it (or the user types a short code) once. The sync pack cannot carry device
keys today because the merge ignores `meta` (`09`), so QR is the v1 pairing
and a synced device directory is a later refinement. The service stores
ciphertext and an expiry; it learns that a user has an ask pending and nothing
else. It needs a free account for push registration and rate limiting, opt-in,
used for nothing else (Q24).

**SSO.** OIDC for member sign-in (any provider); SAML only if a customer forces
it, through a bridge. SCIM for provisioning and deprovisioning: a removed
member's devices lose their wrapped masters on the next sync and the server
refuses their API calls at once; lockbox masters are rotated when a member with
plaintext access leaves (policy-configurable, default on for admins).

**SIEM export.** Audit rows as JSON Lines over a signed webhook, and as OTLP
log records for collectors that speak it. Hash chain per lockbox is preserved
in the export so a consumer can verify continuity (`09`).

**Billing.** Stripe. Seats = active members per month; usage = grants
activated per month above a plan allowance. Agents and lockboxes are not
billed, so nobody is tempted to put everything in one lockbox or one agent to
save money.

**Residency.** Region is a tenant attribute set at creation; it picks the KMS
key, the storage, and the push and SIEM egress. No cross-region replication of
lockbox files; metadata replication is per region as well.

**Availability.** Agents fail closed on an outage, as in Tier 2. The approval
relay failing leaves Tier 1 exactly as it is without the cloud. Status page
and an error budget are operational matters outside this spec.

**Deployment on Cloudflare (❌).** The broker cannot run in a plain Worker: it
needs SQLCipher, native crypto and keys held in process memory. It runs as
`rowel-server serve` in a **Cloudflare Container**, the same binary a
self-hosting company runs, and everything around it is Workers-native. The
principle: Cloudflare for the edge, coordination, storage and plumbing; the
container for the one thing that must be a trusted Rust process.

| Need | Cloudflare piece | Fit and fallback |
| --- | --- | --- |
| Front door: TLS, WAF, rate limits, custom domains, routing | Worker | Good fit. |
| Per-tenant coordination: single writer, approval queue, wake the broker, hold member apps' approval streams | one Durable Object per tenant | Good fit; DO SQLite holds small tenant state. Large tenants shard by lockbox id across several DOs. |
| The broker | Container bound to the tenant DO (`durable_object` policy), `lite` or `basic` instance, `sleepAfter` ≈ 10 min | Disk is **ephemeral** and the instance sleeps when idle, so the container holds no state of record; cold start 1–3 s is paid once per wake. |
| Lockbox files of record | R2, versioned | Loaded by the container on activation, written back with compare-and-swap after each mutation; the DO serialises writers. Versioning is the backup. Lockboxes are small, so whole-file writes are fine for v1; sealed-rows storage (Q18) is the later optimisation. |
| Metadata: members, devices, agents, policies, pending approvals, audit index | D1 (or the tenant DO's SQLite) | Behind the same store interface the single-node server uses with SQLite. |
| Root key | Worker secret binding or Secrets Store → per-tenant HKDF via the `RootKey` trait; customer-managed external KMS for enterprise | Secrets Store was still in open beta when this was written (Q25); the trait keeps the choice reversible. |
| Member SSO and SCIM for enterprise tenants | Cloudflare Access in front of the member and admin API: validate the `Cf-Access-Jwt-Assertion` against the team's keys and the application audience; SCIM deprovisioning events trigger rotation | Self-serve tenants use our own OIDC login instead. |
| Push to phones | APNs and FCM over `fetch` from the Worker | |
| SIEM, webhooks | Queues | |
| Billing and metering | Stripe; usage counted in D1 or Analytics Engine | |
| Free-tier approval relay | Worker + KV with TTL (sealed blobs), push | No container involved. |
| Agent and extension transport | WebSocket terminated at the Worker/DO and passed to the container, **or** HTTPS POST per frame | The sealed channel is one request, one reply, so per-frame HTTPS is a complete fallback if WebSocket pass-through to containers proves awkward (Q25). |
| Residency | `jurisdiction: eu` / `us` on the container and region pinning for R2 and D1 | Tenant attribute set at creation. |
| Static console (later) | Pages | |

Activation sequence:

```
  request ─▶ Worker (TLS, WAF, auth) ─▶ tenant Durable Object
                                            │ wake container if asleep
                                            ▼
                                      rowel-server serve (Container)
                                            │ load lockbox files from R2
                                            │ serve grants / merges / approvals
                                            │ write back to R2 with CAS
                                            ▼
                                      sleep after idle; R2 + D1 hold everything
```

What is deliberately *not* on Cloudflare: HSM-grade key custody (no such
product; the `RootKey` trait points at a customer's KMS when they need it),
and the broker's logic itself, which stays portable so that self-host and
cloud are one code path.
