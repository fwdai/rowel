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
4. **A lockbox is addressed by its vault id and proved by its key.** Never by
   name. Each lockbox has its own KDF salt and so its own key, so a pack for
   another lockbox does not open, and nothing is merged from it.
5. **Only while unlocked.** A locked app does not advertise, listen or dial.
6. **Org lockboxes are excluded** (`org::lockboxes::is_org_lockbox_dir`): the
   Rowel Server is their one source.

## 2. What exists and is reused

| Need | Existing code |
|---|---|
| Merge that converges in any order | `SqliteStore::merge` (a join: last writer wins, purge shells absorbing, audit rows as a union) |
| "Are we equal?" | `SqliteStore::state_digest` |
| Pull, merge, decide, push | `sync::engine::sync` over `Remote` and `LocalVault` |
| Opening a pack, key epochs after a password change | `engine::decode_pack`, `retired` |
| The open lockbox / one that is not open | `SessionVault`, `WorkspaceVault`, keys from `AppState.keyring` |
| A pack into a new lockbox | `sync::restore::restore_at` |
| Subkeys of a lockbox key | `hkdf_subkey`, as `VaultKey::delete_marker_key` |
| One run at a time | `RunClaim` in `commands/sync.rs` |
| TLS | `rustls` 0.23 (ring) |
| Whole-status events | the `sync:status` pattern in `commands/sync.rs` |

## 3. Exchange (`src-tauri/src/nearby/exchange.rs`)

A peer is driven through the existing engine. One side initiates; the other
answers three calls about one lockbox.

```rust
pub trait PeerLink {
    /// The peer's revision and its clock.
    fn head(&self) -> Result<PeerHead>;            // { revision, now_ms }
    /// The peer's pack, with the revision it was packed at.
    fn fetch(&self) -> Result<RemoteFile>;
    /// Hand the peer a pack to merge; its revision afterwards.
    fn absorb(&self, pack: &[u8]) -> Result<String>;
}
```

- A **revision** is the state digest plus the lockbox name and its stamp, as
  one string. Equal revisions mean nothing to do, and no pack moves.
- The answering side is three functions over its own `LocalVault`: `head`,
  `fetch` (`LocalVault::pack`, revision read after the pack's reclaim) and
  `absorb` (decode, merge, adopt a newer name). `absorb` is the first three
  steps of `engine::sync`, extracted so both use one implementation.
- The initiating side wraps the link as an `engine::Remote` (`fetch` is the
  peer's pack, `head_revision` its revision, `upload` is `absorb`) and calls
  `engine::sync`. `upload` being a merge rather than a replace is always safe:
  the merge is a join.
- A peer run must not write the cloud bookkeeping (`sync_remote_rev`,
  `sync_last_ms`): `note_push` is a no-op for it.

`sync_with_peer(link, local, device_id, now_ms)`:

1. `head`. Clocks more than 5 minutes apart: stop with a clock error naming
   the difference. Last-writer-wins is only as good as the clocks.
2. Equal revisions: done.
3. `engine::sync`. Its errors mean what they mean for a cloud run
   (`PasswordChangedRemotely`, `VaultUnderRetiredPassword`, a foreign vault).
4. Record that this lockbox converged with `device_id` at `now_ms`.

**Tombstones.** `engine::reclaim` drops tombstones older than 90 days on the
assumption they were pushed long ago. Between peers that may not be true. So
each lockbox keeps, in `meta` under `SYNC_META_PREFIX` (local bookkeeping,
scrubbed by a restore), the time it last converged with each known device,
and the reclaim cutoff is never later than the oldest of those. That time
covers the deletes made here since, which this device's own clock stamped. It
says nothing of a delete that arrived since from somewhere else, whatever its
stamp (one brought the long way round, or made on a device whose clock runs
behind), so a merge that writes rows resets every device's time, and a device
whose time is reset holds every tombstone back until it converges again.
Forgetting a device (§5, Remove) deletes its row.

Not in this version: sending only the rows that changed. Every exchange that
finds a difference moves whole packs.

## 4. Channel and trust (`src-tauri/src/nearby/`)

Blocking I/O on its own threads (`std::net`, `rustls::StreamOwned`), like the
OAuth loopback listener; no async runtime.

### 4.1 Identity (`identity.rs`)

One Ed25519 key pair per install, in an owner-only file under the app's data
root, made on first use. The **device id** is the hex SHA-256 of its
SubjectPublicKeyInfo. A self-signed certificate over it is generated at load.

### 4.2 Channel (`channel.rs`)

TLS 1.3 only, both sides present their certificate, ALPN `rowel-nearby/1`.
The verifiers accept any self-signed certificate whose handshake signature
verifies, and report the peer's device id; `connect` may be given the id it
expects and fails on any other. The **binding** is 32 bytes of exported keying
material; it differs for every session, and for the two halves of an
interception.

Messages are length-prefixed frames of at most 1 MiB; a larger message (a
pack, up to `pack::MAX_PACK_BYTES`) is sent as consecutive frames. Reads and
writes have deadlines.

A control message is JSON, `{ "t": "<area>/<name>", … }`, with areas `hello`,
`trust`, `sync`, `setup`, and `share` reserved. `hello` carries the protocol
version, the device's name and platform, and the areas it speaks.

### 4.3 Mine: proof of a lockbox key (`trust.rs`)

Two subkeys of a lockbox's key, derived as `delete_marker_key` is:
`nearby-proof-key` and `nearby-discovery-key` (§5).

`proof = HMAC-SHA256(proof key, binding ‖ role ‖ vault id)`, compared in
constant time. Each side sends its proof for a vault id and checks the
other's. Bound to the session, a proof cannot be replayed or forwarded. The
KDF salt is never sent between devices that both hold the lockbox, so a proof
is not a guessing target for the password.

A device at an older key epoch proves a retired key. It is told the password
changed, and sent nothing else.

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
merged rows (`commands::sync`). The session logic runs against a small `Host`
trait (the lockboxes and their keys, a run on one under the run claim, its
end, forgetting a device, the clock), so its tests run two managers in one
process over loopback with an in-memory discovery; the app's host is
`nearby::app::host`.

**Which lockboxes.** The open one (`SessionVault`) and every other lockbox
whose key is in `AppState.keyring` (`WorkspaceVault`), org lockboxes and
lockboxes without a KDF descriptor or a vault id left out. A run opens the open
one through the session, and any other on a connection of its own; never the
open one's database a second time. The set is read again every 3 seconds (the
unlock fills the keyring a moment after the session opens; a password change
changes the keys): when its keys changed, the advertisement follows and every
connection is closed, to be proved again under the new keys. Each side reads
it again as a run begins too, under the run claim, which a password change
waits for: no run goes ahead under keys its connection was not proved under.

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
Knowing the id before dialling is what lets a device apply the rule below
and dial expecting that id (so a record copied and
pointed at another address fails at the handshake). The device's name is not
in the TXT; showing it while "Add a device" is open comes with §6.

**Connections.** One IPv4 listener on every interface; a connection from an
address that is not private, link-local or this machine is closed before TLS.
At most 16 connections are open at once, and each has 15 seconds from its first
byte to the end of its handshake. Of two devices that see each other, the one
with the smaller device id dials; of two connections between one pair, each
side keeps the one the smaller id dialled (else the newer), so both keep the
same one.

After `hello`, the dialling side sends a `trust/proof` for every lockbox it
recognised in the other's advertisement (and a `trust/proof-retired` under
each of its retired keys the advertisement carried), then `trust/done`. The
answering side verifies them and proves back exactly the ones that verified,
then `trust/done`; a device that proved nothing hears nothing else, not even a
vault id. The lockboxes proved both ways under current keys are the ones the
connection syncs. A proof that verifies only under a retired key means that
side is behind: the newer side answers with a `trust/proof-retired` under that
old key, which the older side verifies under its current one, so both learn it
from a proof; that device shows as `needsPassword` on both sides and is sent
nothing else. Whenever anything was proved, the two then exchange their
removed sets (`sync/removed`).

**Runs.** The dialling side drives every run, one lockbox at a time, with
`nearby::exchange::sync_with_peer` over a `PeerLink` that speaks `sync/head` (with
the asker's clock; answered by `sync/head-reply`), `sync/fetch` (answered by
`sync/pack` and the pack as the next message), `sync/absorb` (the pack
follows it; answered by `sync/absorbed`) and, when a run ends, `sync/done`
with the dialler's revision, or the error the run failed with: the answering
side records the converge (`note_converged`) when its own revision equals it,
and shows the error. A request that fails on the answering side is answered
`sync/error` with its kind. The answering side asks for a run with `sync/poke`
when its lockbox changed. A run starts on connect, on a local change (the
signal that schedules a cloud run: `sync_now`, which the frontend's debounce
now sends whenever cloud or nearby sync is on), and on a merge from the cloud
or from another nearby device.

Each side takes the process's one run claim (`AppState::syncing`) for a run:
the dialler before it asks, the answering side from `sync/head` to
`sync/done` (or 30 seconds without a request). A side that cannot take it
answers `sync/busy`, and the run is asked again, waiting 250 ms and doubling
to 30 seconds; a cloud run asked for while a nearby run holds the claim runs
when it lets go. A peer merge that changed rows of the open lockbox emits
`vault:merged` and requests a cloud run; a newer name is mirrored into the
registry as a cloud run's is, and passed on the same way. A run that failed
after it merged announces and passes on what it merged all the same. There
is no keepalive: a device that leaves
without closing its connection shows as connected until a run with it fails.

**Pause and Remove.** Kept per install in `nearby/state.json` under the app's
data root, beside `settings.json` and kept the same way (a JSON file with
defaults, written atomically), next to the identity key `nearby/identity.key`:
enabled, paused device ids, removed device ids, and the devices seen (id, name,
platform, last converge). Pause is local: this side runs nothing with the
paused device and answers its `sync/head` with `sync/busy`, so the other side
keeps asking, at most every 30 seconds, and shows nothing of it. Remove adds
the id to the removed set, which only grows: its connections close, its
tombstone rows are forgotten in every lockbox whose key is held
(`engine::forget_peer`, under the run claim), and the grown set goes to every
connected device at once and at every later handshake (256 ids to a
`sync/removed`, the rest after the handshake), each keeping the union, so one
Remove reaches the owner's other devices as they meet. The next connection
with the removed device, which the smaller id of the two dials as ever, is
refused (`trust/refused`) by the side that removed it, after the two have
proved a key to each other, so the refusal is a holder's word. The device
refused turns nearby sync off. Remove does not erase
what the device holds; that takes a master-password change.

**Locking** (and a lockbox switch, which ends the session) stops the manager
under the session guard: the advertisement goes, the listener and every
connection close, and a run in flight ends at its next read or write. While a
nearby run holds the claim, a switch is refused (`SyncBusy`) as it is during a
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
    | 'syncing'        // a run is in flight
    | 'upToDate'       // connected, nothing to send
    | 'offline'        // known, not on the network now
    | 'paused'
    | 'needsPassword'  // the master password changed on one side
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
