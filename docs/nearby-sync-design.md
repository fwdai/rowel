# Nearby sync: device to device on the local network

Status: in progress. §3 (exchange), §4.1 to §4.3 (channel and the key
proof), §5 (discovery and sessions) and §7 (Settings UI) are built for
desktop. §4.4 (the two ceremonies) and §6 (setting up a new device) are
designed and not built; they, and iOS, follow.

Issue #688. Two Rowel apps on the same network, both open and unlocked, sync
their lockboxes directly: no cloud account, no relay, no server. It runs
beside cloud sync, not instead of it: a lockbox may use both.

## 1. Rules

1. **Nothing leaves a device until the receiver proves it holds the lockbox
   key.** Not ciphertext either. A device that connected by mistake, or with
   a photographed QR code, receives nothing.
2. **Two trust levels.** *Mine*: the device proved a lockbox key, and may sync
   that lockbox. *Connected*: the device only passed a ceremony (§4.4), and
   never syncs. Sharing an item with another person, later, is a message to a
   *connected* device; nothing here assumes two devices share a lockbox.
3. **Keys never travel.** Only packs do (`rowel_core::sync::pack`), the same
   bytes a cloud account holds. A new device derives its keys from the master
   password typed on it.
4. **A lockbox is addressed and proved by its key.** Never by name. Each
   lockbox has its own KDF salt and so its own key, so a pack for another
   lockbox does not open, and nothing is merged from it. Not by its vault id
   either: that lives inside the encrypted database, where it cannot be read
   while the lockbox is closed.
5. **Only while unlocked.** A locked app does not advertise, listen or dial.
6. **Org lockboxes are excluded** (`org::lockboxes::is_org_lockbox_dir`): the
   Rowel Server is their one source.

## 2. What exists and is reused

| Need | Existing code |
|---|---|
| Merge that converges in any order | `SqliteStore::merge` (a join: last writer wins, purge shells absorbing, audit rows as a union) |
| "Are we equal?" | `SqliteStore::state_digest` |
| Open a pack, merge it, take a newer name | `sync::engine::merge_pack` over `LocalVault` |
| Opening a pack, key epochs after a password change | `engine::decode_pack`, `retired` |
| The open lockbox / one that is not open | `SessionVault`, `WorkspaceVault`, keys from `AppState.keyring` |
| A pack into a new lockbox | `sync::restore::restore_at` |
| Subkeys of a lockbox key | `hkdf_subkey`, as `VaultKey::delete_marker_key` |
| One run at a time | `RunClaim` in `commands/sync.rs` |
| TLS | `rustls` 0.23 (ring) |
| Whole-status events | the `sync:status` pattern in `commands/sync.rs` |

## 3. Exchange (`src-tauri/src/nearby/exchange.rs`, `session/sync.rs`)

Each device only ever locks and writes its own lockbox. Per lockbox, on a
connection that proved its key, one script, mirrored on the two sides and
strictly in turns (a pack can be large; each side writes on one thread):

1. The dialling side sends `sync/state { lockbox, revision, now_ms }`, the
   lockbox named by its current key's id (§4.3); the other answers with its
   own.
2. Clocks more than 5 minutes apart: both stop with a clock error.
   Last-writer-wins is only as good as the clocks. Equal revisions: done.
3. The dialling side sends `sync/pack { lockbox, revision }` and its pack,
   the revision read after packing, whose reclaim can move it.
4. The other merges it, then answers `sync/state` if its revision now equals
   the packed one, or else its own `sync/pack`, packed after the merge.
5. Given a pack back, the dialling side merges it and sends a last
   `sync/state`.

A **revision** is the state digest plus the lockbox name and its stamp, as
one string. The two are in sync exactly when the last two revisions stated
are equal; otherwise each side shows its own error, or, having none, that the
sync could not finish. No error crosses the wire: a side whose step fails
states its last revision in its turn, and an answering side that cannot read
its lockbox at all ends the connection.

A merge is `engine::merge_pack`, the first steps of a cloud pull: a pack that
does not open (another lockbox's, a password changed on one side) is refused
before the store is touched, with the error a cloud run gets. Nothing writes
the cloud bookkeeping: the pack is opened with `decode_direct`, which leaves
alone the note of the password the account was last seen under. A device that
proved the current key says nothing of where the account is, and noting it
would have the next cloud run refuse an account still on the old password.

**Tombstones** are the engine's to reclaim, on the rule it has for an account:
after 90 days. Nearby sync adds nothing to it. A device that syncs with
nothing for longer than that, over the network or through an account, can
bring back what was deleted meanwhile; it never loses anything. Holding
tombstones for each device met was tried and taken out: it needed a clock
and a record per device, and was wrong whenever a delete arrived by another
route. A stronger rule belongs in the engine, for both paths at once.

Not in this version: sending only the rows that changed. Every exchange that
finds a difference moves whole packs.

## 4. Channel and trust (`src-tauri/src/nearby/`)

Blocking I/O on its own threads (`std::net`, `rustls::StreamOwned`), like the
OAuth loopback listener; no async runtime.

### 4.1 Identity (`identity.rs`)

One Ed25519 key pair per install, in an owner-only file under the app's data
root, made on first use. The **device id** is the hex SHA-256 of its
SubjectPublicKeyInfo. A self-signed certificate over it is generated at load.

### 4.2 Channel (`channel/`)

TLS 1.3 only, both sides present their certificate, ALPN `rowel-nearby/1`.
The verifiers accept any self-signed certificate whose handshake signature
verifies, and report the peer's device id; `connect_over` may be given the id
it expects and fails on any other. The **binding** is 32 bytes of exported
keying material; it differs for every session, and for the two halves of an
interception.

Messages are length-prefixed frames of at most 1 MiB; a larger message (a
pack, up to `pack::MAX_PACK_BYTES`) is sent as consecutive frames. Reads and
writes have deadlines.

A control message is JSON, `{ "t": "<area>/<name>", … }`, in the areas
`hello`, `trust` and `sync`. `hello` carries the protocol version and the
device's name and platform. Two devices sync only when their protocol versions
are equal, and any change to the wire bumps it; a message of a type this build
does not know is a protocol error, as a malformed one is.

### 4.3 Mine: proof of a lockbox key (`trust.rs`)

Two subkeys of a lockbox's key, derived as `delete_marker_key` is:
`nearby-proof-key` and `nearby-discovery-key` (§5). Every key, current or
retired, has a **key id**: the hex of the first 16 bytes of
`HMAC(discovery key, "rowel-nearby lockbox id\0")`.

`proof = HMAC-SHA256(proof key, binding ‖ role ‖ key id)`, compared in
constant time. A proof names the id of the key it is made under; the receiver
looks that key up among its own and checks the proof under it alone. Bound to
the session, a proof cannot be replayed or forwarded. The KDF salt is never
sent between devices that both hold the lockbox, so a proof is not a guessing
target for the password.

A device at an older key epoch proves a retired key. It is told the password
changed and, when it asks, handed an empty pack under the current key: the
key descriptor over a vault with nothing in it, which the new password is
proved against and adopted from (§5). Nothing of the lockbox is sent to it.

### 4.4 Connected: the two ceremonies

For two devices that share no lockbox: a new install, or later a share.

**QR code.** A computer shows it, a phone scans it, whichever holds the vault.
`rowel://nearby#v1.<device id>.<secret>.<host:port,…>`: 16 random bytes,
single use, 3 minutes. The scanner connects expecting that device id and
sends `HMAC(secret, binding)`. Both sides are then authenticated with nothing
to compare.

**Typed code.** Between two computers. Inside the channel: A sends a
commitment to a nonce, B sends its nonce, A reveals. Both compute 6 digits
from the two device ids, the two nonces and the binding. The device that
holds the vault shows them; the user types them on the other, which compares
with its own. The code is always typed on the device that will enter the
master password: it is the one that then sends a proof built from a salt it
was given, so it is the one that must know there is nobody in the middle.

A ceremony makes a device *connected* and nothing more.

## 5. Discovery and sessions (`src-tauri/src/nearby/{discovery,session,app}/`)

Desktop only for now. Everything is under `src-tauri/src/nearby/`; the app
reaches it at the session's start and end (`session::announce_opened`,
`announce_ended`), at `sync_now` (a local change) and after a cloud run that
merged rows or took a newer name (`commands::sync`). The session logic runs
against a small `Host` trait (the lockboxes and their keys, a step on one
under the run claim, a merge announced, the clock), so its tests run several
managers in one process over loopback with an in-memory discovery; the app's
host is `nearby::app::host`.

**One connection is one sync.** One device dials, the two prove their keys,
every lockbox they hold under the same key is synced (§3), and the connection
closes. Nothing outlives it but what Settings shows. There is no standing
connection to keep in step with the keys or the network.

**Which lockboxes.** The open one (`SessionVault`) and every other lockbox
whose key is in `AppState.keyring` (`WorkspaceVault`), org lockboxes and
lockboxes without a KDF descriptor left out. A step finds its lockbox by the
current key's id, opens the open one through the session, and any other on a
connection of its own; never the open one's database a second time. The set is
read again every 3 seconds (the unlock fills the keyring a moment after the
session opens; a password change changes the keys): when its keys changed, the
advertisement follows, every advertisement seen is read again with them, and a
sync with each device is due at once. A connection proves the keys as they
were read when it opened.

**Discovery.** Service `_rowel-sync._tcp`, advertised only while unlocked and
enabled, through `mdns-sd` on Windows, Linux and macOS (iOS will need the
system Bonjour API, which refuses an app's own multicast). The instance and
host names are random per session. TXT carries `v` (the protocol version), `n`
(a random 16-byte nonce, new every session) and, for each lockbox key this
device holds (current ones first, then retired epochs, at most 16), `t<i>`:
the tag `HMAC(discovery key, nonce)` truncated to 8 bytes, followed by the
device id XOR `HMAC(discovery key, label ‖ nonce)`. A device that holds the
lockbox recomputes the tag, recognises it, and unmasks the id; nobody else can
link two sessions, tell which lockboxes a device holds, or which device it is.
Knowing the id before dialling is what lets a device skip one it removed or
paused, and dial expecting that id (so a record copied and pointed at another
address fails at the handshake). The device's name is not
in the TXT; showing it while "Add a device" is open comes with §6.

**Connections.** One IPv4 listener on every interface; a connection from an
address that is not private, link-local or this machine is closed before TLS.
At most 16 connections are open at once, and each has 15 seconds from its first
byte to the end of its handshake.

**When a device dials.** A sync with each device it recognises and has not
removed or paused is due every 30 seconds, and at once on first sight of its
advertisement, when what that advertisement lets it recognise changes, when
the keys change here, when a lockbox changes here
(`sync_now`, which the frontend's debounce sends whenever cloud or nearby sync
is on, or a merge from the account or another device), and on a resume. Each
dial puts the next sync 30 seconds off, and so does the end of every
connection with the device, however it ended: 30 seconds for the side that
dialled and 45 for the side that answered, so one of a pair keeps dialling and
the other stands by. A sync asked for while a connection was open stays due.
A device shows as here while its advertisement is seen.

After `hello`, the dialling side sends a `trust/proof` for every lockbox it
recognised in the other's advertisement (and a `trust/proof-retired` under
each of its retired keys the advertisement carried), then `trust/done`. Each
proof names its key's id (§4.3). The answering side proves back exactly the
ones that verified, then `trust/done`; a device that proved nothing hears
nothing else, not even a key id. The lockboxes proved both ways under current
keys are the ones the connection syncs. A `trust/proof` under a key the
receiver has retired means the other side is behind: the newer side answers
with a `trust/proof-retired` under that old key, which is the older side's
current one, so both learn it from a proof. The newer side shows the device
as `needsPassword`, the older as `passwordChanged`.

**A changed password.** The older side asks for the pack to adopt the password
from: `adopt/ask { lockbox, have }`, the lockbox named by the id of its own
current key and `have` the ids of the keys sealing the packs it already keeps.
The answering side sends its asks with its proofs, since from then on it only
answers; the dialling side sends its own after the lockboxes it syncs. The
answer is `sync/pack { lockbox, revision }` and an empty pack
(`pack::pack_empty`, about 74 KB), the revision being the id of the key that
seals it, or a `sync/state` naming that id when `have` lists it. An ask is
answered once for each key its sender proved; one for any other key, or a
second for the same, ends the connection. A pack is taken only in answer to an
ask, once, and only up to the size an empty one can be (1 MiB): anything else
is a protocol error. At most three are kept per lockbox (a lockbox keeps three
retired epochs), the oldest going first, in memory until an adopt succeeds or
the session ends.

The prompt and the command are the cloud's (`sync_adopt_password`,
`docs/password-change-sync-design.md`): the typed password is proved by
opening a pack, the account's where the lockbox syncs to one and then the
kept ones, and the device re-keys onto that pack's descriptor through the
same saga, the lockboxes beside it following. An open lockbox with nothing to
adopt does not stop the ones beside it: the password is then proved by the
first of them it opens, and counts as a wrong one when it opens none and one
refuses it. A pack that cannot be read leaves its lockbox as it is and does
not stop the others. An account that cannot be reached does not stop an adopt
a kept pack can serve. When nothing was adopted while a nearby device shows as
having the new password, its pack is not here yet, and the command says the
password cannot be checked yet rather than report success. Two cases are left out:
devices that each changed the password apart share only a retired key and do
not recognise each other, and a device more than three changes behind is not
recognised (three retired epochs are kept).

**Steps.** The dialling side names each lockbox in turn with its
`sync/state` (§3) and closes when all are done. Every local step (read the
state, pack, merge) takes the process's one run claim (`AppState::syncing`)
for that step alone, never across a wait on the network, and opens the
lockbox itself. A claim held elsewhere is waited for up to 5 seconds; then
the connection ends and the dial is tried again later. A cloud run asked for
while a step held the claim runs when it lets go.

One gate comes before every step: the device is neither removed nor paused
here, and the lockbox's keys, read again under the claim (which a password
change waits for), are the ones the connection proved. A gate that fails ends
the connection without a word.

Inside a merge's step, the claim still held, rows written into the open
lockbox emit `vault:merged` (`commands::sync::announce_merged`) and a newer
name is mirrored into the registry by the function a cloud run uses
(`sync::mirror_name`), which reads the name the lockbox holds then. Once that
lockbox's exchange
is over, a cloud run is asked for and a sync with the other devices here is
due: not sooner, since the cloud run takes the claim the exchange still needs.

**Pause and Remove.** Kept per install in `nearby/state.json` under the app's
data root, beside `settings.json` and kept the same way (a JSON file with
defaults, written atomically), next to the identity key `nearby/identity.key`:
enabled, paused device ids, removed device ids, and the devices seen (id,
name, platform, last sync). A paused or removed device is refused through one
check in three places: it is not dialled, a connection from it is closed as
soon as TLS says whose it is, and the gate ends one with it before its next
step. Both are local and neither is announced: the other side keeps trying
every 30 seconds and shows this one as `waiting`. Remove is a Pause for good,
its row hidden. Nothing about it is sent to or taken from another device (a
list that travelled would let any key holder cut the owner's devices off from
each other), so the others sync with it until it is removed on each. Remove
does not erase what the device holds; that takes a master-password change.

**Locking** (and a lockbox switch, which ends the session) stops the manager
under the session guard: the advertisement goes, the listener and every
connection close, and a sync in flight ends at its next read or write. While a
nearby step holds the claim, a switch is refused (`SyncBusy`) as it is during a
cloud run.

## 6. Setting up a new device (next)

After a ceremony: the vault holder sends each lockbox's vault id and KDF
descriptor; the user types the master password on the new device; it derives
each key, proves the ones that open, and receives those packs
(`restore::restore_at`). Names travel after the proof. Wrong proofs are
counted, and a few void the invitation.

## 7. IPC contract

Commands (desktop builds; elsewhere none is registered, the status read
fails, and the frontend draws nothing of nearby sync):

| Command | Args | Result |
|---|---|---|
| `nearby_status` | — | `NearbyStatus` |
| `nearby_set_enabled` | `{ enabled }` | — |
| `nearby_pause` | `{ deviceId, paused }` | — |
| `nearby_remove` | `{ deviceId }` | — |

One event, `nearby:status`, carrying the whole status on every transition,
as `sync:status` does; the store keeps the one with the higher `seq`. "Add a
device" and its invite come with §6: neither its commands nor its fields are
in this contract yet.

A device's state is derived in this order: `paused`; `offline` while its
advertisement is not seen; `syncing` while a connection with it syncs; then
the outcome of the last attempt with it. An attempt that reached no verdict
(none yet, could not connect, or either side ended it) leaves none, shown as
`waiting`. A connection's outcome is its first lockbox that failed, or a
success only when every lockbox synced, so `upToDate` follows only a sync that
succeeded.

```ts
interface NearbyStatus {
  /** This build can do it at all. */
  available: boolean
  enabled: boolean
  deviceName: string
  devices: NearbyDevice[]
  seq: number
}

interface NearbyDevice {
  id: string
  name: string
  platform: 'macos' | 'windows' | 'linux' | 'ios'
  state:
    | 'paused'
    | 'offline'        // known, its advertisement not seen now
    | 'syncing'        // a connection with it is syncing
    | 'waiting'        // here, no verdict from the last attempt
    | 'upToDate'       // here, the last attempt synced every lockbox
    | 'passwordChanged' // that device changed the master password: adopt it here
    | 'needsPassword'  // that device is on a password this one has changed
    | 'clockSkew'
    | 'error'
  /** ISO time of the last exchange that converged, or null. */
  lastSyncedAt: string | null
  /** The backend's sentence for `clockSkew` and `error`, and its kind, as
   *  `SyncStatus.error` and `errorKind` are: the kind is what gets translated
   *  (`describeError`, through `nearbyDetailText`), the sentence the fallback. */
  detail: string | null
  detailKind: BackendErrorKind | null
}
```
