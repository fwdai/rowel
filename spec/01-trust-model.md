# 01 — Trust model: two properties of a lockbox

## 1. The one honest line

Rowel's promise today is "no backend holds your secrets": a lockbox is sealed
under a key derived from the master password and nothing else can open it. An
unattended agent breaks that promise by definition: for the broker to fill a
login at 3 a.m. in a CI job, *something* that is not the user must be able to
open the item.

Rather than blur this, the design keeps two properties of a lockbox separate
and visible, and draws both lines where the product already has one: the
lockbox.

**Where the key lives (hosting).**

- A **device-held lockbox** is what Rowel is today. Its key derives from the
  master password; it syncs as an opaque pack through a cloud account the user
  owns; no server, including ours, can read it.
- An **org lockbox** (Tiers 2 and 3) is server-held. Its key is random and
  *escrowed to the broker*, because the broker has to act for agents while
  nobody is present. The UI says so in plain words at creation. Members' apps
  hold a copy of the key too, so an org lockbox looks like any other lockbox
  in the vault: same master password, same compartment list.

**Whether agents may use it (access).** Every lockbox, of either kind, has an
agent-access setting: *off* (default), or *on*, which lets policy rules and
grants be written for items in it. A lockbox with agent access off never
answers an agent, whatever the rules say. The recommended setup is a dedicated
lockbox for the items agents need, which is the same gesture people already use
to split Personal from Work.

The two properties are independent. A device-held lockbox with agent access on
serves agents only while the app is unlocked and a person is there to approve
or has written a standing rule. An org lockbox with agent access off is a shared
team lockbox for humans, nothing more.

## 2. What each component can read

```
                          device-held lockbox       org lockbox (escrowed)
                          ───────────────────       ──────────────────────
  the user's app          plaintext (unlocked)      plaintext (unlocked; key
                                                    came from the server)
  browser extension       items for the page,       same, plus grant-driven fills
                          after URL match
  local agent (Tier 1)    nothing; only outcomes    nothing; only outcomes
  local sidecar (Tier 1)  values for header/env     same
                          modes, per grant
  Rowel Server (Tier 2)   nothing (opaque pack,     plaintext while serving a grant;
                          if synced through it)     key wrapped under the server root
  Rowel Cloud (Tier 3)    nothing                   same as Tier 2, root key in KMS
  remote agent (Tier 2/3) —                         nothing; only outcomes
  remote extension        —                         the item for the fill, inside
  (cloud browser)                                   the sealed channel, per grant
```

The invariant that holds in every cell: **a client of kind `agent` never
receives a secret value.** Only `extension` (to fill) and `sidecar` (to inject
at the edge) do, and only for the items a grant names.

## 3. Why not env vars on the box

Short version, so this document can be cited on its own:

1. The agent can read them with one shell call, and will if a web page tells
   it to.
2. They are standing: alive for the box's lifetime and every task.
3. They are unscoped: the key works for every endpoint.
4. They are unattributed: the upstream sees the key, not the run or the person.
5. They cannot wait for a human on a sensitive action.
6. They cannot log into a website.
7. They leak through transcripts, logs, screenshots, child processes.
8. Rotation is a redeploy and burns the key everywhere at once.

For a deterministic service, KMS-to-env is fine. This design is specific to
agent workloads and should not claim otherwise.

## 4. Residual risks, stated

- **Same-user local processes.** As in `docs/threat-model.md` today, a process
  running as the same OS user can lift an extension's or a sidecar's association
  key and present it. Grants and approval modes bound what that buys; they do
  not remove it. The CLI's keys should live in the OS secure store where one
  exists (open question Q9).
- **The logged-in session.** After a fill, the agent's browser holds cookies.
  `05` ends them with the grant. An automation agent that controls the browser
  through the devtools protocol can read cookies *during* the grant; session
  end bounds the window, not the capability. A site that issues long-lived
  tokens elsewhere is out of reach.
- **The child process in `env` mode.** It holds the value and can print it.
  Policy floors the mode to `ask`, standing rules name the full command line,
  and the output redactor is the second net. The mode exists for tools that
  take nothing else; it is labelled the weakest for a reason.
- **The broker's memory.** In Tiers 2 and 3 an org lockbox's key and the items
  it serves are in the server's memory while serving. Zeroize on drop,
  per-tenant keys, idle timeouts and short grant lifetimes bound exposure; a
  compromised server host is the compromised-OS case of the threat model, now
  for a company's org lockboxes.
- **Vision.** A screenshot taken after a fill can show an autofilled username
  or a password field's dots. Redaction (`05`) covers text; it cannot cover
  pixels. Agents that need vision should fill with the field masked and submit
  before the next capture.

---

### Technical details

**Key hierarchy today (✅).** `rowel_core::crypto::vault::VaultKey` is either
`Argon2 { master: 32 bytes }` or `Legacy { secret }`. The master is HKDF-split
into the SQLCipher key (`INFO_SQLCIPHER`), the per-item payload AEAD key
(`INFO_PAYLOAD`), the attachment key, the delete-marker key and the
retired-keys key (`crypto/vault.rs`). The master comes from
`kdf::derive(password, KdfParams)` with the descriptor in a sidecar beside the
database (`storage::KDF_SIDECAR_FILE`) and in every sync pack's header.

**One vault, many lockboxes (✅).** `appkey.rs`: the primary workspace's key is
the *app key*; every other workspace keeps its own key sealed under it in a
local sidecar (`storage::WRAPPED_KEY_FILE`), and `Keyring` holds every open
workspace's key material until the next lock. Opening the primary opens them
all. The sidecar is device-local by design.

**Org lockbox keying (❌).** Nothing below the master changes. What changes is
where the 32 bytes come from and who else holds a wrapped copy:

```
  device-held lockbox                   org lockbox
  ───────────────────                   ───────────
  master password                       random 32 bytes (CSPRNG), minted by
     │ Argon2id (sidecar params)        the server at lockbox creation
     ▼                                      │
  master (32 B)  ──────────────────────▶  master (32 B)
     │ HKDF (unchanged)                     │ HKDF (unchanged)
     ├─ SQLCipher key                       ├─ SQLCipher key
     ├─ payload AEAD key                    ├─ payload AEAD key
     ├─ attachment key                      ├─ attachment key
     └─ marker / retired keys               └─ marker / retired keys
                                            │
                                   wrapped copies of the master:
                                     • under the server root (KMS → tenant key
                                       → AES-GCM wrap), held by the server
                                     • under each enrolled device's X25519 key
                                       (crypto_box sealed box, `seal` feature
                                       of the crate the extension channel uses),
                                       delivered at enrollment and rotation
                                   on each member device:
                                     • the unwrapped master goes into `Keyring`
                                       and a `WRAPPED_KEY_FILE` sidecar under
                                       the app key, exactly like any
                                       non-primary workspace today
```

- Add `KdfParams::External { kid: String, gen: u32 }` (`crypto/kdf.rs`,
  serialised as `{"algo":"external","kid":"…","gen":n}`): the master is not
  password-derived; obtain it from the broker named `kid`. `derive` returns
  `Error::Crypto` for it; `validate` accepts it; `next_generation` mints a new
  `kid` with `gen + 1`. `commands/auth.rs::unlock` on a workspace whose sidecar
  is `External` looks for the wrapped master (`Keyring`, else the server) instead
  of running Argon2id, and refuses with a clear message when neither is there.
  The sidecar and every pack header carry the descriptor verbatim, so
  `sync::engine::decode_pack`'s string comparison of headers keeps working
  across devices (every device stores the server's exact descriptor string).
- `VaultKey::from_material(material, true)` already constructs an Argon2-shaped
  key from raw 32 bytes; an org lockbox's unwrapped master goes through it. No
  change to `SqliteStore`, `PayloadCipher`, packs or merge.
- **Device enrollment key (❌).** Each install generates an X25519 keypair once
  (`crypto_box::SecretKey::generate`), stored under the app key beside the
  other workspace keys. Its public half is what a server wraps lockbox masters
  to. Enrollment of a new device is approved from an already-enrolled device
  or by an admin (`02`), so a stolen single sign-on session alone does not
  yield wrapped masters.
- **Rotation of an org lockbox master.** Reuse the password-change saga
  (`commands/auth.rs::rekey_open_workspace`, `retired.rs`): re-key SQLCipher,
  re-seal payloads, retire the old epoch (its `External` descriptor string and
  material go into `RetiredKey` as today), then re-wrap the new master to every
  device and to the server root. Devices that pull a pack under a retired epoch
  already carry it onto the current one (`sync/engine.rs`,
  `opened_under_retired`).

**Agent-access flag (❌).** A per-workspace boolean in the registry
(`workspace.rs`, `Workspace.agents: bool`, default false) in Tier 1, and a
lockbox attribute on the server in Tier 2. `Host::request_grant` refuses with
`GrantDenied` for a lockbox whose flag is off before policy is consulted.

**What the threat model must say after phase 0 (✅ I4: `docs/threat-model.md`
gained an "Agents" section mirroring this document for the local tier, with
the server-tier items named as not shipped; `docs/agents.md` is the user
guide).** In "The browser extension":
*"There are no per-site prompts, by design — a fill asks nothing"* becomes
*"a fill asks nothing while a person is driving; while agent mode is on, or
when a client of kind `agent` requested it, a fill is a grant and asks or is
covered by a standing grant"*. Add an "Agents" section mirroring this document,
including the residuals above.
