# 05 — Delivery modes: using a credential without handing it over

## 1. Five ways to use a secret, none of which gives it to the agent

The whole design rests on this table. Each mode puts the value somewhere the
agent cannot read, at the moment it is needed, and nowhere else.

| Mode | What happens | Where the value goes | Fits |
| --- | --- | --- | --- |
| **fill** | The browser extension types the login (and TOTP) into the page and may submit it | the page's form fields, briefly | web apps with no API; local or cloud browsers |
| **passkey** | The vault signs a WebAuthn assertion; the approver's tap is user presence | nowhere; a signature leaves | phishing-resistant sign-in; the strongest mode |
| **header** | The agent calls an API through a Rowel proxy with a placeholder; the proxy swaps in the real header | the outgoing HTTP request, inside the proxy | API keys, bearer tokens, MCP server auth |
| **token** | Rowel holds a refresh token and issues short-lived scoped access tokens, or performs token exchange naming the agent | the proxy, as above | services with OAuth; phase 4 |
| **env** | One child process is started with the value in its environment; the agent's own shell never has it | that child process only | legacy CLIs; the weakest mode, floored to `ask` |

Two things wrap every mode:

- **Redaction.** The broker knows which values it released. Anything the agent
  reads afterwards (tool output, page text) passes through a filter that
  replaces an exact occurrence of a released value with its placeholder.
- **Session end.** A grant has a lifetime. When it ends, the login it enabled
  ends too: the extension deletes that site's cookies in the agent's browser
  profile. The vault governs the login's lifetime, not just the fill.

## 2. Fill, in detail

The extension already fills. What changes is *who asked* and *what happens
after*. When an agent asked (through a grant), or when the extension is in
agent mode because an agentic browser is driving, a fill:

1. waits for the grant to be active (approved, or auto by policy);
2. fills only on a tab whose origin the grant names, only the item the grant
   admits (if several, the one the approver picked);
3. submits if the grant says so, otherwise hands back to the agent;
4. checks the page did not end up showing the secret in text (a login form
   that echoes the password on error, a debug page), and clears the fields if
   submission failed;
5. hides its own in-page UI and popup for the duration, so the agent cannot
   drive the extension itself.

Agent mode is both explicit and implicit: explicit when an agent client
requests `fill`; implicit when the user switches it on in the extension for a
browser profile an agent uses. Reliable *detection* of an agentic browser
without its cooperation is an open question; the toggle is the simple answer.

## 3. Passkey, in detail

The vault is already a software authenticator: it holds the private key and
signs assertions, and every ceremony puts up a dialog naming the relying party
and the asking page. For agents, nothing cryptographic changes. The dialog adds
one line, *requested by Claude Code*, and the grant records it. The approver's
tap is the user-presence gesture the specification requires, performed by a
present human on the device that holds the key. For that reason a passkey
grant is never `auto` (`03`).

This is the mode to lead with. A filled password can land on a lookalike site;
an assertion is bound to the origin and is useless anywhere else. Nothing is
typed, nothing can be scraped, nothing needs clearing.

Server-held passkeys for unattended agents, and how standards bodies land on
agent-driven ceremonies, are open questions (`10`, Q5). For a person's own
agent on their own machine, the person is present and verifying, which is the
spirit of the specification.

## 4. Header and token, in detail

The agent is told to call `api.stripe.com` through a local address and to put
`rowel://work/stripe/apiKey` where the key would go. The proxy checks the grant
covers that host, path and method, swaps the placeholder for the value, forwards
over TLS, and scrubs the value from the response. The agent's process, its
transcript and its environment never contain the key. In Tier 2 the proxy runs
as a sidecar the company controls, outside the agent's sandbox, which is exactly
the arrangement the hosted-sandbox audit found to be the only safe one.

Token mode is header mode where the thing swapped in is minted, not stored: an
access token refreshed from a stored refresh token, scoped down, short-lived.

## 5. Env, in detail

Some tools only take a key from the environment. `rowel run` resolves the
placeholders a grant covers into the environment of *one* child process and
pipes its output through redaction. The parent shell, and therefore the agent,
never sees the value. It is still the weakest mode, because that child process
can print it, so policy floors it to `ask` unless a rule names the full command
line, and the redactor catches a value printed verbatim.

---

### Technical details

**Fill (🟡, extension + host).**

- Host side (✅ I2): `request-grant` with `modes: ["fill"]` and `targets:
  [Origin(url)]`. The candidates are the lockbox's logins whose `url_host`
  column matches the origin's host, narrowed by the request's selector; no
  payload is opened to decide a grant, and a site with no login is `denied`
  (`no_candidates`). On activation `AppHost` queues `Signal::FillGrant {
  grant_id, origin, item: VisibleItem, auto_submit: false, expires_at_ms }`
  for **every** extension client associated to the same lockbox and pushes
  `rowel-signal` (`04`). Only an extension with a tab at that origin acts.
  The grant is not single-use (a fill is followed by the TOTP step and
  retries, `03`); instead the first extension the host serves under it
  *claims* it (`Grant.served_to`), and a different extension asking under
  the same `grantId` afterwards is refused `GrantDenied`. No per-profile
  addressing is needed.
- Extension side (`extension/rowel/background/`): a new `grants.js` module
  handles `rowel-signal` → `get-signals`, keeps `activeGrants` in
  `browser.storage.session` (already used by `credentials.js`), and on
  `FillGrant` finds the tab with a matching origin (`browser.tabs.query`, the
  `tabs` permission exists), then asks the host `get-logins { url, grantId }`
  through `keepass.retrieveCredentials`. The host serves only the admitted
  item; the content script's existing `kpxcFill` path fills it
  (`content/fill.js`). `autoSubmit` reuses upstream's auto-submit; TOTP reuses
  `get-totp { grantId }`.
- **Exposure check**: after fill (and after submit, on the next page load for
  the same tab), the content script scans `document.body.innerText` and every
  `input:not([type=password])`'s `value` for the exact password and TOTP it
  filled; a hit clears those fields, reports `exposed: true` to the host (which
  logs it and ends the grant), and shows the extension's notification.
  Values live only in the content script's closure for the check and are
  dropped after. Limits: same-origin frames only, no closed shadow roots, no
  pixels (Q13).
- **Hide UI in agent mode**: `page.js` skips injecting the in-page icons,
  autocomplete and banners when `agentMode` is set; the popup shows a single
  "Agent mode on · cancel" screen. The settings page stays reachable through
  `chrome://extensions`.
- **Agent mode flag**: per browser profile, in the extension's settings, set
  from the popup — or, as built in I2, by the extension itself when a
  `fill-grant` arrives while it is off (`rowelAgentModeByGrant` remembers
  that a grant did it), and back off at the `grant-ended` of the last agent
  grant, or on `database-locked`. Agent mode the person turned on is never
  touched, and there is no `set-agent-mode` signal: the host cannot see a
  profile's setting and should not guess at it.
- **As built in I2** (`grants.js::onSignal`): `rowel-signal` → `test-associate`
  → sealed `get-signals`, also once after every reconnect. A `fill-grant`
  is remembered under its origin with the host's `expiresAtMs`, the origin
  is marked agent-driven (`rowelAgentGrants`), and the active tab at that
  origin (exact origin match, port included) is told `redetect_fields`,
  upstream's own message that reloads the content script's settings and
  ends in a retrieval — so a tab loaded before agent mode went on fills as
  agent mode rather than through the person's icons; `page.clearLogins`
  first, so the retrieval reaches the host. Without a tab the grant waits
  for the next load at that origin. `item` and `autoSubmit` are not read:
  the host serves the admitted item on `get-logins`, and submit stays
  upstream's `autoSubmit` setting.
- **As built in I1** (`extension/rowel/background/grants.js`,
  `content/agent-mode.js`, `popups/agent-mode.js`): the flag is the
  `agentMode` entry of the extension's settings object; the grant map is
  `browser.storage.session["rowelGrants"]`, origin → `{ grantId, expiresAt }`
  with the host's 15-minute window, cleared on `database-locked`; the
  after-submit handoff is `browser.storage.session["rowelFilled"]`, tab id →
  `{ password, totp, url }`, read-and-delete within 60 s. `get-logins` sends
  exactly one of `grantId` (a live grant for the page's origin) or
  `agentMode: "true"`; `get-totp` sends `grantId` when one is live. Refusals:
  `GrantDenied` and `GrantExpired` forget the grant (`GrantExpired` retries
  once with `agentMode`), `GrantPending` retries once after `retryAfterMs`
  capped at 10 s, everything else gives up; notices are OS notifications with
  fixed text, never page or entry data. Agent mode retrieves on page load
  whatever `autoRetrieveCredentials` says, fills the one admitted entry (TOTP
  first when the entry has one) once per form, and submits only through
  upstream's `autoSubmit`. The exposure scan runs after the fill, once more a
  second later, and on the next load of the same tab; the TOTP needle must not
  be adjacent to another digit in the page text, so an order number does not
  end a grant.
- **Grants are per document origin.** Every request is keyed by the origin of
  the document that made it, not the tab's top page: `get-logins` by its
  request URL, `get-totp` by the sender's frame URL (the background forwards
  the sender to handlers), `report-exposure` by the origin the content script
  names. A cross-origin login frame is therefore its own origin: it asks its
  own grant, and the dialog names the frame's origin, which is where the
  credential goes. A fill in the content script has a lifecycle of its own,
  start → scan → rescan → end, held in one object; starting the next fill
  first finalises the previous one by running its last scan with its own
  secrets and its own ignored field, so two fills never share state or a
  timer and no fill's exposure check is skipped by its successor.

**Passkey (✅ I2).** `browser::passkeys::{register, assert}` and the
`UserConsent` dialog (`ask_passkey`, `events::browser_passkey`) are unchanged
in mechanism. The `Host` methods take the connection's `Client`; the event
carries `requestedBy: { kind, name }`, shown as one line in the dialog; and
every completed ceremony writes a `used` row with `mode: passkey`, the origin
as target and the client as actor, a refused one a `denied` row (`09`). No
grant object is involved in Tier 1: the ceremony's own tap is the approval.
The floor in `03` keeps every passkey ceremony behind the tap, agent or not.

**Header / proxy (❌, `rowel proxy`).**

```
  agent process                       rowel proxy (sidecar)                 upstream
  ─────────────                       ─────────────────────                 ────────
  GET http://127.0.0.1:7400/https/api.stripe.com/v1/balance
  Authorization: Bearer rowel://work/stripe/apiKey
  X-Rowel-Grant: <grantId>            ─▶ parse target host+path+method
                                         grant covers? (Target::Http match)
                                         resolve(ref) via broker (sidecar kind)
                                         replace placeholder in headers / body
                                         forward over TLS to api.stripe.com  ─▶
                                      ◀─ response
                                         redact released values in body/headers
  ◀─ response
```

- Addressing: path prefix `/https/<host>/<path>` (v1). Most SDKs allow a base
  URL override, which is all the agent needs. A `CONNECT`-style forward proxy
  with a local CA is deferred (Q15).
- The proxy's client key is kind `sidecar`. `resolve` is refused for any other
  kind by the host, so an agent that found the proxy's port still cannot get
  values without going through the proxy's policy-checked path, and the grant
  id it must present is one the broker issued to *it* (`03`).
- Replacement happens in headers and, when a request body is `application/json`
  or form-encoded and the grant allows body placement, in the body. Never in
  URLs (they land in logs upstream).
- Limits per grant: `max_uses` and a token-bucket rate limit (default 60/min)
  so a looping injected agent cannot hammer an upstream with a real key.
- Tier 2: the sidecar runs where the company puts it (a pod sidecar, a host
  daemon) and associates with the server over WebSocket with its own workload
  identity, kind `sidecar`, bound to its agents by `sidecar_for`. The agent's
  sandbox reaches it over the pod network; the agent's egress to the real
  upstream is blocked by the company's network policy, which is what makes the
  proxy the only path.

**Token (❌, phase 4).** A new item kind `oauth` (or `apikey` with a `refresh`
field; decide at implementation) holding client id, refresh token, scopes,
token endpoint. `resolve` for a `token`-mode grant triggers the broker to
refresh (or exchange, RFC 8693, with `act: { sub: agent_id }` when the upstream
supports it) and returns the short-lived access token to the sidecar, which
places it as in header mode. The refresh token never leaves the broker.

**Env (❌, `rowel run`).** `rowel run --grant <id> -- <cmd...>`: resolves every
`rowel://` placeholder found in the child's intended environment (from
`--env NAME=rowel://…` flags or a `.rowel-env` file the user points at) through
`resolve`, spawns the child with those values set and the parent's environment
otherwise, and streams stdout/stderr through the redactor. The grant's
`Target::Command` is matched against the *full* command line. `rowel hook`'s
pre-tool rewrite produces exactly this invocation; when no grant exists yet,
`rowel run` requests one first (it holds the sidecar key, which may request
as well as resolve).

**Redaction (❌, `rowel redact`, also a library in the sidecar and the hook).**
Input: a byte stream and the set of values released under the grants this
process resolved. Output: the stream with each exact occurrence replaced by its
placeholder. Also matches the obvious encodings of each value (base64, URL
percent-encoding, JSON-escaped). Streaming matcher with a carry-over window of
the longest value's encoded length, so a value split across chunks is still
caught. It is a safety net, not a guarantee: a value transformed otherwise
passes (Q16). Values are held in a `Zeroizing` set for the process lifetime.

**Session end (❌, extension).** On `grant-ended { origins }` or on the grant's
`expires_at`, the extension deletes the site's cookies with the `cookies` API
the manifest already requests (`cookies.getAll({ domain })` for each origin's
registrable domain, then `cookies.remove` per cookie, both Chromium and
Firefox). This needs **no new permission** and is precise to the domain.
Clearing localStorage, IndexedDB and service workers as well would need the
`browsingData` permission (Chrome: `origins` applies to cookies, storage and
cache; Firefox: `hostnames`); that is a later, optional addition, because a
new permission prompt on update is a cost and cookies are the session. Safari
has neither API from an extension; document session end as best-effort there.
Scope: only origins the grant named. If the user also uses that site in the
same profile, the dialog warned at grant time ("this will sign you out of
github.com in this browser when the agent is done"); profile isolation is the
better answer (Q12).

*As built in I2.* The host queues `grant-ended { grantId, origins }` for every
extension of the lockbox from one helper, on every way a grant ends: expiry,
`end-grant` from the agent or from Settings, a switch going off, an exposure.
The extension deletes the cookies of each origin's registrable domain, found
through upstream's own `page.getBaseDomainFromUrl` (which probes the public
suffix list with a throwaway cookie, as it does for iframes), in every cookie
store; leaves an origin a newer grant holds whole, session included, since
it is that grant's to end; shows a fixed-text notification; and turns agent
mode back off only if a grant turned it on and none is left. A lock ends
every grant on the host with no row and no `grant-ended` (nothing is served
from a locked vault), so on `database-locked` the extension signs out of
every origin it held a grant for, the person's own agent-mode grants
included. Signals, the lock and an exposure run through one queue in
`grants.js`, each to its end before the next: a lockbox switch sends
`database-locked` and `database-unlocked` back to back and a fill-grant may
follow at once, and the lock's cleanup must not erase it. Safari:
`browser.cookies` is absent, so the deletion is skipped.
