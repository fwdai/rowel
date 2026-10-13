# Sharing a secret — by link, over the sender's own Drive

Status: v1. Backend module `src-tauri/src/share`, frontend `src/components/Main/Share`.

Rowel has no server, and sharing does not add one. A share is a sealed copy of
one entry that lives for a day in the sender's own Google Drive. The recipient
opens it with a link, and the link is the only secret.

What this is, precisely: a bearer-link transfer of a snapshot. It does not
authenticate a named recipient, does not propagate later changes, and is not
burn-after-reading. Whoever holds the link can open the share until it expires
or is revoked; revoking stops future downloads but cannot retract a credential
already imported. The UI is written to promise exactly that and no more.

## 1. What the user is trying to do

- Hand a colleague the Wi-Fi password, a shared inbox login or an API key
  without pasting it into a chat where it stays forever.
- Do it from a phone as easily as from a laptop.
- Receive one without signing up for anything.

Two flows follow: **send**, which produces a link, and **receive**, which turns
a link into a new entry in the recipient's vault. There is no live link between
the two copies: once received, the entry is the recipient's, and later edits on
either side stay where they were made.

## 2. Trust model in one paragraph

The sender's app generates a fresh random 256-bit key, seals the entry with
AES-256-GCM under it, uploads the ciphertext to a `Rowel/Shares` folder in the
sender's Drive, and marks that one file readable by anyone who knows its id.
The link carries the file id and the key. Google holds ciphertext and sees that
a share was made, but never the key. The recipient's app downloads the file with
a public API key (no login) and unseals it locally. Whoever gets the link gets
the credential, so the link must travel over a channel the sender already
trusts. Rowel refuses to open it after 24 hours or once revoked; the file
itself is gone only once revoked or swept (section 5 has the difference).

## 3. Formats

Link, one pasteable token:

```
rowel://share#v1.<driveFileId>.<key>
```

`key` is the 32 key bytes, base64url without padding. Nothing fetches this URL;
the scheme exists so the token is unambiguous and could register as a deep link
later.

Envelope (plaintext before sealing):

```json
{ "v": 1, "expiresAt": 1700086400000, "entry": { ...sanitized Entry... } }
```

Sealed with the same AES-256-GCM helper the vault uses for entry payloads
(`crypto::seal_aead`): a random 16-byte nonce is prepended to the ciphertext.
The expiry sits inside the ciphertext, so it is authenticated by the same tag
as the entry: nobody holding the file, Drive included, can extend it.

Sanitizing strips what must not travel: the entry id (the recipient assigns a
fresh one), the timestamps, the favorite flag, and any passkeys — a copied
passkey is a second authenticator nobody registered. Tags and custom fields are
kept. It runs on **both** ends. On send, so nothing leaves that should not. On
receipt, because the plaintext is whatever the link's author sealed: a crafted
envelope naming an existing row's id must not be able to replace that row.
Receipt also refuses an unknown entry kind and anything past its expiry.

A login whose only secret is a passkey is refused at share time with a plain
message, rather than arriving as a username and nothing else.

## 4. Drive layout

```
Rowel/                      the existing sync folder
  Lockboxes/                accounts created before the rename keep Vaults/,
                            and are read from there
    <vault-id>.rowel        a vault pack, untouched by sharing
  Shares/
    <random>.rowelshare     one share, appProperties: rowelShare, entryId, kind,
                            expiresAt, vaultId
```

Every name here is minted by `sync::layout`, sharing included. A vault is
addressed by its vault id — the opaque identity of the data, never the user's
label for it, which does not reach Drive at all. A share is named by randomness
alone.

Shares stay account-level rather than moving under a vault: the sweep must be
able to clear an expired file whichever vault it came out of, and every one of
the sender's devices runs it. What a share belongs to is a property, not a
folder — see `vaultId` below.

`appProperties` is the only metadata written. `rowelShare=1` is the marker
every share carries, and the only thing listing selects on: shares are found by
it wherever they sit, so two devices racing to create `Shares/` and uploading
into their own copies hide nothing from the sweep or the revoke list. The
folder is only created by an upload, never by a listing, so accounts that never
share never gain one. `entryId` is the sender's local id, an opaque UUID that
lets the sender's own UI show a title without revealing it to Drive. `kind` is
the entry type. `expiresAt` is unix milliseconds, a duplicate of the
authenticated copy inside the envelope, there so the sweep can date a file
without its key. `vaultId` is the id of the vault that published the share
(`store::identity`), which is what lets one account hold several vaults' shares
and still show each vault only its own — and, because every listed row carries a
Revoke, it is also what a revoke is checked against.

A share carrying no `vaultId` predates the property and belongs to nobody. It is
not shown to every vault, which would hand one workspace a Revoke over another's
link, and it is not adopted by any: ownership is two concrete ids agreeing, and
an absent id matches nothing. Such a share is listed by no vault, revocable by
none, and removed by the account-wide sweep when its 24 hours are up — which is
the whole of its remaining life, since every share written by this build carries
an id. Listings follow `nextPageToken` to the end.

## 5. Lifetime and control

- **Sending requires a synced vault**, not merely a connected one. Every share
  is stamped with the publishing vault's id and every later list and revoke is
  checked against it, so a vault with no id has nothing to publish a link
  under — it could neither list nor take back what it sent. A vault keeps its id
  only after a sync run has succeeded, so one that has just connected Drive, or
  whose first run failed, is refused with `shareNeedsSync` ("sync this workspace
  once before sharing from it"). The check sits in `commands::share::sendable`
  alongside unlocked and sync-configured, which is why `share::create`, `list`
  and `revoke` take a `&str` id rather than an optional one.
- Fixed 24-hour lifetime in v1, backed by two guarantees that are not the same
  strength. **Rowel refuses to open** an envelope whose authenticated
  `expiresAt` has passed, whether or not the file is still there; that holds
  on every Rowel client. **Deletion is best effort**: until a revoke or sweep
  removes the file, the ciphertext is still downloadable, and a leaked link
  plus a client that ignores the expiry decrypts it past the 24 hours. Every
  promise in the UI and in `docs/threat-model.md` is worded for that split.
- **Revoke** fetches the file by id and deletes it only if it carries the
  `rowelShare` marker and the asking vault's own `vaultId`. The id in the
  request is whatever the webview sent and authorizes nothing by itself, so a
  share another vault published is refused with `shareNotOwned` and left where
  it is — the same test `list` applies, so a vault can revoke exactly what it
  can see — and a Drive file that is not a share of this app's is refused at any
  id. Ownership is two concrete ids agreeing, never two absences: a share with
  no `vaultId` belongs to nobody at all. The fetch is a `files.get`, not a listing: Drive's query index lags its
  files, the send dialog offers Revoke seconds after the upload, and a share the
  index had not caught up with would otherwise read as already gone and be left
  live for its whole 24 hours. A file Drive no longer has does count as revoked.
  From the send dialog right after creating the link, or later from Settings ›
  Sync › Shared links.
- **Sweep** lists every marked share and deletes anything past `expiresAt`. It
  runs at the end of every successful sync and whenever the active shares list
  is opened. It stays account-wide where listing and revoking are per-vault: an
  expired share is dead whichever vault published it, and whichever of the
  sender's devices notices should say so. Because the truth lives in Drive
  rather than a local ledger, any of the sender's devices can revoke or clean
  up, and nothing new is stored locally. A share with no readable expiry is
  never treated as expired.
- The recipient's app treats a missing file as "expired or revoked" and says so.
- The recipient downloads at most 2 MiB, checked against the declared length
  and again while streaming, with a 30-second deadline: the id in a pasted link
  can name any public file on Drive. The sender is held to the same cap at seal
  time, before anything is uploaded, and told which entry is too big.

Burn-after-reading cannot be enforced without a server. Expiry plus revoke is
the honest approximation, and the UI says "expires" and "revoke", not "one
time".

## 6. What each side needs

| | Google account | Network | Env |
|---|---|---|---|
| Send | Drive connected and synced at least once (existing sync OAuth, `drive.file` scope) | yes | `GOOGLE_OAUTH_CLIENT_ID` |
| Receive | none | yes | `GOOGLE_API_KEY` |

The API key is a public identifier restricted to the Drive API, baked in at
compile time like the OAuth client id. A build without it can send but not
receive, and the receive dialog says which key is missing.

## 7. Backend API

`share::create(remote, entry, vault_id: &str, now_ms) -> Created { link, file_id, expires_at }`
`share::open(fetch, link, now_ms) -> Entry` (sanitized, empty id, unexpired)
`share::revoke(remote, file_id, vault_id: &str)` (fetches by id; refuses anything not this vault's)
`share::list(remote, vault_id: &str, now_ms) -> Vec<ActiveShare>` (this vault's; also sweeps)
`share::sweep(remote, now_ms) -> usize` (account-wide)

The vault id is a `&str` on all three sender-side verbs, not an `Option`: a
share belongs to a vault by carrying that vault's id, and there is no such thing
as a share owned by the absence of one. The commands supply it from
`store::identity::vault_id` and refuse with `shareNeedsSync` when there is none
(§5), so the verbs below never have to answer what a nameless vault may do.

Generic over `ShareRemote` and `PublicFetch`, so the whole lifecycle is tested
against an in-memory fake.

Exposed to the frontend as `share_create`, `share_open`, `share_revoke`,
`share_list`. All Drive calls run off the async runtime via `spawn_blocking`,
the way sync does.

## 8. Out of scope for v1

Live one-way and two-way shares (a separately keyed pack with Drive per-user
permissions, syncing through the existing engine), configurable expiry, QR
codes, deep-link opening of the token, and any recipient-side burn-on-read.
