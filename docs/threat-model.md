# Rowel Threat Model

This describes what Rowel protects, what it deliberately does not, and how the
master key moves through the app. It reflects the code merged on `v1-0-0`; where
the current implementation differs from the planned design, that is called out.

## Architecture in one paragraph

Rowel is a Tauri 2 desktop app: a trusted Rust core plus a system-webview
frontend (React/TypeScript). All cryptography and key handling live in Rust. The
webview never sees the master key; it talks to the core through a fixed,
enumerated list of commands (`src-tauri/src/lib.rs`) and receives non-secret
entry metadata for the list plus one decrypted entry at a time on reveal. The
vault is a locally stored, encrypted SQLite database under the OS app-data
directory (on iOS, the App Group container; see "iOS AutoFill"). There is no account server and no backend that holds user secrets. The
webview is locked down by a strict CSP (`default-src 'self'`, `connect-src
'self'`, `object-src 'none'`, `frame-src 'none'`, `base-uri 'none'`, and no
`'unsafe-inline'` anywhere: the two `<style>` blocks in `index.html` run under
the per-load nonce Tauri adds to `style-src`; see `src-tauri/tauri.conf.json`),
and external links are opened through the OS only for `http`/`https` URLs
(`opener:allow-open-url` scope in `capabilities/default.json`; plain `http` is
kept because routers and intranet logins have no other address, and the scope
exists to shut out `file:`, `javascript:` and custom schemes, not to upgrade
transport). Two commands read a path the webview names — `scan_image` (a photo
of a card or identity document) and `read_env_file` — and neither trusts the
path it is handed. A path is readable only if it was granted by one of two
events the core itself observes: the OS file dialog run from Rust
(`pick_file`), or an OS drag-and-drop onto the window (`PathGrants`, one-shot,
consumed on use). A grant carries the purpose it was made for — the picker's
kind (`image` or `env`), or for a drop the file's kind — and each reader spends
only a grant of its own purpose: a path the user chose in an env-file dialog
cannot be read by `scan_image`, so the dialog's framing is part of what was
consented to. The session must then still be the one that asked — the epoch
is re-checked after the read, so a lock or a workspace switch mid-read discards
the bytes. Only then is the file classified: the extension against a fixed
image list (`scan/mod.rs`), the `.env` name-or-parse check and the 1 MiB cap
(`commands/env.rs`). That classification is a sanity check on a file the user
already chose, not the authorization; the grant is.

## What sits on disk

The vault is a **SQLite database sealed with SQLCipher** (`vault.db`,
`src-tauri/crates/core/src/store/`, `rusqlite` with `bundled-sqlcipher`). SQLCipher encrypts
the **entire database file at rest** — page contents, the schema, free pages, and
the write-ahead log — with AES-256 and per-page authentication. On disk, nothing
is readable without the database key.

Inside the decrypted database:

- An `entries` table keeps **non-secret metadata in plaintext columns** — `id`,
  `kind`, `title`, `tags`, `url_host`, `created_at`, `updated_at`, `deleted_at` —
  so the list and search work without unsealing anything. These columns are
  protected at rest by SQLCipher, but they are *not* additionally app-encrypted:
  anyone holding the database key sees them in the clear.
- Each row also carries an opaque `payload` BLOB. The storage layer never
  inspects or encrypts it (`src-tauri/crates/core/src/store/sqlite.rs` documents the payload
  as caller-owned); the application applies its **own AEAD** on top. The whole
  entry is serialized to JSON and sealed as AES-256-GCM (base64-wrapped), and
  each individually sensitive field (login password, OTP secret, secure-note
  body, card PIN) is **also** sealed as its own AES-256-GCM value nested inside.
  So a secret stays ciphertext even inside the *decrypted* database and is
  unsealed only when the user reveals or copies it (**decrypt-on-reveal**,
  `reveal_entry` in `src-tauri/src/commands/vault.rs`).
- **A payload is bound to its row.** Because the metadata columns are in the
  clear inside the decrypted database, someone holding only the SQLCipher key
  could otherwise move one row's sealed payload under another row's title and
  host. Payloads sealed by current builds carry the entry id as AEAD
  associated data, and every payload — including those sealed before the AAD
  existed — is checked after unsealing to carry the id of the row it came from
  (`crypto::PayloadCipher`). A payload moved between rows fails to open.
- **Freed content is zeroed.** The connection runs with `secure_delete` on,
  so a "delete forever" (which empties the row's payload) or a reclaimed
  tombstone leaves zeros in the freed pages rather than the old ciphertext,
  and a purge also checkpoints and truncates the write-ahead log so the page's
  previous image does not linger there. Someone who later obtains the database
  key finds no purged secrets in free space. A master-password change re-encrypts
  every page through `PRAGMA rekey`, which is an ordinary write transaction: in
  WAL mode the re-encrypted pages would land in `vault.db-wal` while the main
  file still opened under the *old* key. The rekey folds the WAL back and
  truncates it before returning (`sqlite.rs`), so no page under the old key
  survives the change in either file. The checkpoint is verified rather than
  assumed: its busy column and its log-versus-checkpointed counts have to say
  every frame was folded in. A checkpoint that fails fails the rekey, which
  rolls the database back to the pre-change snapshot.
- A small `meta` key/value table holds app data — the KDF descriptor and similar
  settings — not schema versioning (that rides SQLite's `user_version`).
- **The favicon cache is inside SQLCipher.** A `favicons` table holds one row per
  host — `host`, `uri` (NULL records a miss, aged out by `fetched_at`) — and is
  the whole of the icon cache (`store/sqlite.rs`, `favicon.rs`). It replaces the
  loose `icons/` files named after each host, which put the vault's host list,
  live and deleted, in the clear beside the encrypted database where any
  file-level backup picked it up. Both the read and the write go through the
  session epoch the command captured when it started (`Session::store_at(epoch)`),
  so a lock or a workspace switch during a lookup discards the result rather
  than writing it into whichever workspace is open when the fetch returns. The
  legacy directory is removed at startup, not after a lookup.

The practical shape of this two-layer design: an attacker who never obtains the
key sees only SQLCipher ciphertext for everything. Metadata confidentiality rests
on SQLCipher alone; secret fields get a second, app-level AEAD layer whose purpose
is defense-in-depth and decrypt-on-reveal, not hiding metadata from someone who
already has the database key.

Writes are **per-row and atomic** (WAL mode), not a whole-file rewrite: saving one
edited entry re-seals only that row's payload. Deletes are **tombstones**
(`deleted_at` is stamped and the row is retained so a later sync can propagate the
deletion), not hard deletes. On-disk file and directory modes are tightened on
Unix (`0600` file / `0700` dir): the database file is created owner-only
*before* SQLite opens it, so it — and the `-wal`/`-shm` files SQLite creates
with the same mode — never spend the open under the umask's default. The
Windows ACL equivalent for the database is still a TODO (`restrict` no-ops off
Unix in `sqlite.rs`). Plaintext exports, saved `.env` files and the sealed
Drive token file, written through `storage::atomic_write_private`, are
owner-only on both: `0600` on Unix, and on Windows a protected DACL granting the
current user and SYSTEM alone (`owner_only.rs`), supplied when the temp file is
created so it never exists, even briefly, under the folder's inherited
permissions. Every small state file beside the vault (the KDF and lockout
sidecars, the settings, the biometric marker, the token file) is written by
atomic rename, so a crash leaves the previous complete file rather than a
truncated one.

### Key derivation (KDF)

From the master password the core derives one **Argon2id** master key — `m=64
MiB, t=3, p=4`, a fresh 32-byte random salt, the password fed in directly with
no pre-hash (`crypto/kdf.rs`) — and from that master two independent subkeys by
`HKDF-SHA256` under distinct context labels (`crypto/vault.rs`):

- **The SQLCipher database key** — the `sqlcipher-db-key` subkey. SQLCipher opens
  the file with this raw key; a wrong password derives a wrong key and the open
  fails verification, which the app surfaces as an invalid password.
- **The payload key** — the payload subkey, used directly as an AES-256-GCM key
  with a fresh random nonce per value and no per-payload KDF. This is the key
  that seals each entry payload and each nested secret field.

One KDF pass covers both, and neither subkey reveals the other.

The parameters and the salt have to be readable *before* the encrypted database
can be opened, so they live in a plaintext sidecar beside it, `vault.kdf.json`
(`storage::KDF_SIDECAR_FILE`). It holds nothing secret — an algorithm tag, three
costs and a salt — and it is authoritative: `create_vault` writes it before the
database exists, and `derive_key` reads it on every unlock. The same descriptor
is mirrored into `meta` for reference, and it is what a `.rowel` pack carries in
its header so another device can derive the key for the snapshot inside.

> **Where the KDF stands:**
> - **Argon2id is the live derivation.** `session::create_vault` mints
>   `KdfParams::default_argon2id`, writes the sidecar, and derives the master;
>   `session::derive_key` re-derives it from that sidecar on every unlock,
>   feeding the password to Argon2id directly — no `SHA-512` pre-hash on this
>   path. A restore derives from the descriptor the pack carries in its header,
>   and a password change writes fresh params and a fresh sidecar (rolling both
>   back together if it is interrupted). A database that exists with *no*
>   sidecar predates this wiring (an interim/dev vault) and still opens under the
>   old deterministic key, but it cannot sync — `SessionVault::capture` refuses a
>   vault with no descriptor to put in the pack header.
> - **PBKDF2 is the legacy path.** `PBKDF2-HMAC-SHA512` at 100,000 iterations over
>   `base64(SHA512(password))` — the Electron-era `Cryptor` — survives to read
>   legacy `.swftx` backups and to open those sidecar-less vaults. Nothing new is
>   sealed under it. It remains a shape `KdfParams` can parse, which is why the
>   bound below applies to it too.
> - **Failed unlocks are throttled.** Three attempts are free; each wrong password
>   after that doubles the wait — 2s, 4s, 8s, … capped at five minutes
>   (`FREE_ATTEMPTS` and `MAX_DELAY_SECS` in `auth.rs`). `commands::auth::unlock`
>   refuses outright while a lockout stands, and a successful unlock resets the
>   counter. Only SQLCipher's own wrong-key failure counts; an I/O error or a
>   schema-too-new refusal does not. The state is a plaintext sidecar beside the
>   vault, `vault.lock.json` — it has to be, since a wrong password never opens
>   the database the counter would otherwise live in. It fails open only where
>   the file says nothing: a sidecar that is absent, or JSON that does not parse,
>   reads as "no lockout", because the throttle is not a security boundary. A
>   sidecar the app cannot read at all is different — `LockoutState::load`
>   propagates the I/O error out of `storage::read_lockout_sidecar` and
>   `commands::auth::unlock` returns it, so the unlock fails rather than
>   proceeding past a counter it could not consult. So it bounds guessing *at
>   this app's lock screen* and nothing else: see "Offline brute force at scale"
>   below.
> - **Untrusted KDF parameters are bounded.** A descriptor is read off things
>   this build did not write — a `.rowel` backup, a pack pulled from Drive, the
>   sidecar on disk — and the Argon2 crate would otherwise accept a memory cost
>   of 4 TiB. `KdfParams` refuses costs past a ceiling (1 GiB, 64 passes, 64
>   lanes; 10M PBKDF2 iterations) both when parsed and when derived from, so a
>   crafted header is an error rather than an abort or a derivation that never
>   returns.

### Passkeys

A login entry can hold WebAuthn credentials (`models::Passkey`, one per site
account). They live **inside the sealed payload**, in the entry's `passkeys`
list — there is no passkey column and no separate table, so a credential's
private key gets exactly the protection an entry's password gets: SQLCipher at
rest, the app AEAD on top, unsealed only for the operation that needs it. Sync
carries them as part of the opaque payload and never sees them.

- **The private key never leaves the core.** `src-tauri/crates/core/src/passkey/` unseals a
  login, converts the stored PKCS#8 key to a COSE key in memory, signs, and drops
  it. No command returns a private key to the webview: `reveal_entry` hands out
  `Entry::redacted`, which blanks every passkey's `private_key` (blank is omitted
  on the wire, so the field is absent from the frontend `Passkey` type too), and
  a share is sanitized of its passkeys altogether before it is sealed. The
  webview therefore sees a credential's identity — which site, which account,
  when it was made — and nothing that could sign with it.
- **A save puts the key back rather than trusting the webview for it.** Since an
  edit comes back without the keys it was never given, `save_entry` unseals the
  row it is replacing and copies each `private_key` across, matched on
  `credential_id` (`Entry::restore_passkey_keys`). Only the passkeys still on the
  incoming entry are completed, in the order it lists them, so removing one in
  the editor removes it and reordering reorders. A blank key whose credential id
  is not in the stored row — or that has no stored row at all — is refused
  (`NotFound`) rather than saved as a credential that could never sign. The
  stored row is unsealed only when a blank key is present.
- The one way a passkey key leaves the app is an **explicit user-initiated
  export** (a `.rowel` backup, which is the encrypted vault snapshot itself, or
  Bitwarden JSON, which is plaintext by construction and carries the key as
  base64url) — the same deliberate exposure the password export already is.
- **User verification is the unlocked session plus the user's consent.** The
  vault being unlocked is the identity half of WebAuthn's "user verified" bit;
  the intent half is asked for on every registration and sign-in through the
  `passkey::UserConsent` seam, which whatever feeds requests in must supply —
  an `Authenticator` cannot be built without one, so a transport has to bring
  its confirm prompt rather than inherit a silent yes. A refusal ends the
  ceremony as denied. The one transport is the browser extension host
  (`browser::passkeys`, see "The browser extension" below): it plays the
  WebAuthn client for the page and asks the user through the app's own dialog,
  one ceremony at a time.
- **Signature counters stay at zero.** Credentials sync across devices, so a
  per-device counter would look to a relying party like a cloned authenticator.
  New credentials are created with the constant zero the spec recommends for
  synced keys and are not incremented on sign-in; an imported credential that
  arrived with a non-zero counter keeps counting.
- **The AAGUID is a model identifier, not a device one.** One fixed value for
  every Rowel install (`passkey::AAGUID`), so it cannot be used to correlate a
  user across relying parties.

## Key lifecycle across the process split

1. **Derive off the UI thread on unlock.** The user enters the master passphrase
   in the webview. It is passed once to the Rust core, which derives the key
   material and opens SQLCipher (which runs its own internal KDF) on a blocking
   thread (`spawn_blocking` in `commands/auth.rs`), so the UI never stalls. The
   passphrase itself is not stored. One thing outlives the unlock by a bounded
   stretch: when the vault syncs, a second `Zeroizing` copy of the passphrase is
   moved into a blocking-pool task that tries it against the Google account's
   packs this device lacks and adds the ones it opens as workspaces
   (`commands/autojoin.rs`). The copy is spent on those Argon2id derives, is
   never written anywhere, and is scrubbed when the task ends. How long that is:
   the task stops starting on new candidates 60 seconds in (`JOIN_BUDGET`), so
   the copy lives for at most that budget plus the one download in flight when
   it ran out — and that download is bounded by the 256 MiB cap over the slowest
   rate that still counts as progress, not by a deadline, since a steady trickle
   never trips the read-stall one. A large pack on a slow link stretches the
   copy's life by the length of that download. Whatever was not tried stays on
   offer in Settings › Workspaces. Biometric unlock has no passphrase and does
   not do this.
2. **Hold in Rust only.** The derived secret lives in the Rust session
   (`state.rs`, `Session.master_key`) wrapped in a zeroizing buffer, alongside the
   open encrypted store handle — never a fully decrypted vault. It never crosses
   back to the webview. The webview receives entry metadata plus, on an explicit
   `reveal_entry`, one decrypted entry.
3. **Zeroize on lock.** The key buffer is scrubbed from the heap (`zeroize`) when
   it is dropped or replaced, and the store connection is closed. Locking happens
   several ways: an explicit Lock action, the **inactivity auto-lock** (60
   seconds by default, configurable up to a day — `autolock.rs`), and on app
   exit (the session is dropped). The auto-lock is armed for the whole of an
   unlocked session and re-armed by every sign of the user — input in the
   webview, throttled to one ping every few seconds (`touch_activity`), and the
   window gaining or losing focus — so the vault seals that long after the
   *last* one whether the window is in front or behind. **Coming back is not a
   fresh start when the time away already spent the whole timeout**: the focus
   event locks instead of re-arming, and it judges by the wall clock rather than
   the timer's `Instant`, which does not advance across a sleeping Mac or a
   suspended iOS process (`autolock::handle_event`). Every lock, including
   a workspace switch, also **clears the clipboard** if it still holds a secret
   the app copied, so a copied password does not outlive the session it came
   from even with the clipboard timeout set to "Never". On iOS the clear is the
   expiry instead — reading the pasteboard back to compare would raise the system
   paste banner over a value the user never asked to paste — and **every iOS
   write now carries one**: what the user asked for, or a 24-hour cap when they
   asked for "Never", and never longer than that cap
   (`IOS_MAX_PASTEBOARD_TTL`). On Linux the copy carries no concealed marker at
   all: off Apple and Windows `commands/clipboard.rs` falls back to a plain
   write, so a clipboard manager there may record the value.

   One thing a lock does not reach: a sync run in flight holds key copies of its
   own. `SessionVault::capture` copies the SQLCipher key into the run's vault
   handle (`sync/engine.rs`), and the run's `DriveRemote` owns a `Cryptor`
   derived from the same master — both living on the blocking thread the run
   occupies. A lock ends the session but not that thread, so those copies
   outlive it until the run's thread does. What bounds them is the run, not the
   lock — and the run has no whole-request deadline. The sync client's
   deadlines (`sync/mod.rs`: 15 s to connect, 60 s of silence mid-body) end a
   download that stops making progress; one that keeps trickling in resets the
   stall deadline with every chunk, so the 256 MiB pack cap bounds a download
   only in principle, at one chunk per stall window. An upload has less than
   that: the read deadline watches the response, not the body being sent, so a
   server that drains the pack slowly holds the run for as long as it takes. A
   slow but steady transfer in either direction therefore keeps those key copies
   alive for as long as it lasts. This is a **residual**: closing it means a
   whole-run deadline, or a lock that aborts the run, and neither exists today.
4. **Optional biometric unlock (opt-in).** Instead of re-entering the passphrase,
   the same key material can be stored in the OS keychain behind a biometric gate
   (`secure_store.rs`):
   - **macOS (`GateMode::Protected`):** a data-protection Keychain item with a
     `SecAccessControl` of `kSecAccessControlBiometryCurrentSet` and a protection
     class of `kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly` — so the item
     never rides along in an encrypted backup to be restored onto another
     device, and it stops existing the moment the user removes their passcode,
     which is the same moment biometrics stop meaning anything. Touch ID is
     enforced by the OS on *read*, and the item auto-invalidates if the enrolled
     fingerprints change.
   - **macOS fallback (`GateMode::Prompt`):** the data-protection keychain is the
     only one that honours a biometric `SecAccessControl`, and reaching it needs
     the profile-gated `keychain-access-groups` entitlement. A build without it
     is refused with `errSecMissingEntitlement`, and `secure_store.rs` then
     stores the key as an **ordinary keychain item — no access control, no OS
     gate** — under an account of its own, and makes the app's own `LAContext`
     policy evaluation the gate, run before the read. That is a weaker
     guarantee, and deliberately a different one: the biometric check is enforced
     in-process, so it binds this app and not the keychain, where the OS would
     have refused the read outright. The mode is recorded at enrollment and never
     re-derived — an item is never read under a gate it was not stored behind —
     and the Settings row names which one is in force rather than letting the two
     read as one feature. The release build does not ship the fallback by
     accident: `scripts/check-macos-entitlements.mjs`, run from
     `.github/workflows/release.yml`, fails the build when the bundle requests a
     profile-gated entitlement that no embedded provisioning profile grants.
   - **Windows:** only an AES-256-GCM blob goes into Credential Manager, sealed
     under a key derived from a Windows Hello key-credential signature, so
     opening it requires passing the Hello prompt rather than merely being the
     logged-in user.
   - **Linux and others:** unsupported; the app reports biometrics unavailable
     rather than store an ungated key.
5. **One unlock for every workspace (`appkey.rs`).** The primary workspace's
   key is the *app key*. Every other workspace keeps a copy of its own key
   sealed under it — AES-256-GCM under an HKDF subkey of the app key, bound to
   the workspace id as associated data — in a `0600` sidecar beside its
   database (`vault.key.sealed`). Opening the primary, by passphrase or by the
   biometric item above (which stores the app key and nothing else), unwraps
   every sidecar into an in-memory ring, so a workspace switch opens the next
   database without a prompt and without an Argon2id run. The ring lives as
   long as a session would: it ends with every lock, the auto-lock included,
   and the sidecars are worth nothing without the app key. They are also local
   to the device on purpose — the pack a workspace syncs stays under its own
   passphrase-derived key, so another device restores it with the passphrase
   exactly as before. A workspace that has never been opened with its
   passphrase on a device has no sidecar there; its first switch lands on its
   lock screen, and that unlock writes one. The same passphrase for every
   workspace is the rule (a create proves it against the app key), which is
   what lets a non-primary unlock prove the passphrase against the primary in
   the background and fill the ring from there. Every step that settles which
   key the app key is and then reads or writes sidecars by it — a proof filling
   the ring, a passphrase change replacing the key, a delete promoting a
   workspace — runs whole under one lock (`AppState::sidecars`), so a slower
   proof of a retired key cannot reseal a sidecar under it, or remove a fresh
   one as stale, behind a change that has already landed. A passphrase *change*
   (`change_master_password`) keeps that rule true: after the open workspace is
   re-keyed, every other workspace that shares the passphrase is re-keyed too —
   each under a fresh salt of its own, through the same snapshot-guarded saga on
   its own files, and with its Drive token re-sealed under its new key — so a
   workspace created before the change no longer restores on another device only
   under the passphrase the user has just replaced. A workspace on a passphrase
   of its own is left untouched (its own passphrase is not the one that was
   proved), and one whose re-key fails stays on the passphrase it had; both are
   reported back so the UI can say the new passphrase does not open everything
   on the device.

## Fresh start and explicit import

`v1-0-0` starts with an **empty SQLite vault**. `app_status.initialized` is true only when
the encrypted database exists; a legacy `vault.swftx` file alone does **not**
count. Nothing is migrated automatically on unlock.

Bringing existing data forward is an **explicit** action, and there are two paths:

- **Restore a backup as a new vault** (`setup_restore_from_file`,
  `commands/setup.rs`): a `.rowel` backup is the same pack the sync engine
  uploads to Drive — the plaintext KDF descriptor followed by the SQLCipher
  snapshot (`sync/pack.rs`) — written by `export_vault`. Restoring installs it
  through the same `restore_from_pack` path as a Drive restore: fresh installs
  only, the password is validated by SQLCipher opening the snapshot, and the
  source device's sync bookkeeping is scrubbed.
- **Import a legacy vault into the currently-unlocked vault** (`import_swftx`,
  `commands/vault.rs`): the Electron-era `.swftx` is independently encrypted and
  carries its own master password. Each entry is decrypted under the backup's
  key and re-sealed under the current session key, then merged by id. This
  CPU-bound re-encrypt loop runs **off the UI thread** and emits
  `import:progress` events so the UI can show progress. It is an import feature
  in Settings, not a first-run path.

## What Rowel defends against

- **Device theft / a lost or stolen laptop.** The vault is a local, SQLCipher-
  encrypted database; each entry's secrets carry an additional app-AEAD layer.
  Without the master passphrase (or a biometric unlock on that specific enrolled
  device) the contents are unreadable. The key is never persisted in plaintext,
  and auto-lock limits how long an unlocked session stays open.
- **Cloud-provider or sync compromise.** Sync is optional and, in `v1-0-0`,
  disabled (`sync::ENABLED = false`). When enabled it uploads only the already-
  encrypted vault blob to the user's own Google Drive; the master passphrase and
  derived keys never leave the device. A compromised Drive account or a tapped
  sync channel yields ciphertext, not secrets.
- **A stalled or hostile network peer.** Every HTTPS client in the app shares a
  connect deadline and a read-stall deadline (`sync::http_client_builder`), so
  a connection that stops answering fails the run instead of holding the
  `syncing` flag — and with it manual sync and workspace switching — until the
  process exits. Downloads are capped and the cap is enforced on the bytes as
  they arrive: 256 MiB for a sync pack, 2 MiB for a share, 256 KiB for an icon.
  The desktop OAuth loopback listener binds an OS-chosen port and gives up
  after five minutes, so an abandoned consent tab frees the flow rather than
  wedging a fixed port. Favicon fetches run only for an unlocked vault, only
  over HTTPS, never to an IP literal or a local-only name (`.local`, `.lan`,
  `.internal`, `.home.arpa`, `.localhost`), and follow redirects only to URLs
  that pass the same bar.
- **Remote server breach.** There is nothing central to breach. Rowel is
  offline-first with no account server, no telemetry, and no phone-home. The only
  outbound request at launch is the signed updater check to GitHub Releases; Drive
  sync (when enabled) runs from the Rust core over HTTPS, not from the webview.
- **The app-switcher snapshot.** While the unlocked vault is on screen but the
  window is not in front of the user, an opaque cover is drawn over the whole of
  it (`components/elements/PrivacyScreen`). iOS snapshots the webview for the app
  switcher the moment the scene resigns active and keeps that image on disk until
  the app is foregrounded; the same image is what Mission Control shows and what
  a screenshot of an unfocused window captures. The cover goes up on the focus
  event that precedes the snapshot, with no animation — a fade would hand the
  system a half-transparent frame — and it is mounted with the unlocked vault
  only, never over the lock screen. It starts *covered* when the document is
  unfocused or hidden at mount, so a session that opens behind another window is
  not briefly exposed to a snapshot, and reconciles against the window's own
  `isFocused()` once that answer is in. If the window focus subscription fails,
  it falls back to DOM `focus`/`blur` events rather than staying uncovered.
- **Passive at-rest access and backups.** Because the database is SQLCipher-
  encrypted whole-file, file-level backups (Time Machine, disk images, cloud file
  backups) carry only ciphertext.
- **A tampered update.** Updater artifacts are minisign-signed and verified
  against the public key baked into `src-tauri/tauri.conf.json`; an unsigned or
  modified artifact is rejected. A signed update is downloaded and **staged** in
  the background and applied on the next launch — but the app does **not** relaunch
  silently. It surfaces a restart toast, and the update is applied only when the
  user consents to restart (or the next time they quit and reopen). This replaces
  the earlier silent-on-launch install.

## Sharing a secret

A share (see `docs/share-design.md`) is one entry, sanitized, sealed under a
fresh random 256-bit AES-GCM key and uploaded to the sender's own Drive as an
"anyone with the link" file. The link carries the file id and the key.

- **Google** holds ciphertext, the file's creation time, the entry kind and the
  sender's opaque local entry id. It never holds the key and cannot read the
  entry. Neither the vault key nor the master passphrase is involved.
- **The link is the secret.** Anyone who obtains it can open the share while
  the file exists. Rowel cannot defend the channel the sender chose to send it
  over. What bounds the exposure is two guarantees of different strength, and
  they should not be read as one:
  - **Rowel refuses to open a share after 24 hours.** The expiry is inside the
    authenticated ciphertext, so every Rowel client honours it whether or not
    the file is still on Drive.
  - **Deleting the file is best effort.** Revoke deletes it at once. Otherwise
    the sender's devices sweep expired shares after each successful sync and
    whenever the shares list is opened, which needs a device to be on. Until
    then the ciphertext is still downloadable, and a leaked link in the hands
    of someone using their own AES-GCM code rather than Rowel decrypts it past
    the 24 hours. The hard stop is deletion; revoke is the only immediate one.
- **Integrity.** AES-GCM authentication means a modified or substituted file
  fails to open rather than yielding a tampered entry. The expiry is inside the
  ciphertext, so it is authenticated too, and the recipient enforces it even
  when the sender's device never got to delete the file.
- **Sender-controlled content is not trusted.** Encryption proves the sender
  held the key, not that the entry is well-formed. On receipt the entry is
  sanitized again (id, timestamps, favorite, passkeys stripped), its kind is
  checked, and it is always saved as a fresh row, so a crafted envelope cannot
  overwrite an existing entry. Downloads are capped at 2 MiB and time-limited,
  because a pasted link can name any public Drive file; the same cap is applied
  to the sealed entry before upload, so a share nobody could open is never
  published.
- **The public API key** used to download shares is an identifier, not a
  credential: it grants no access to anything not already public.
- **This is a bearer-link snapshot, not delivery to a person.** Nothing
  authenticates the recipient, nothing propagates later edits, and without a
  server nothing can count reads. A share stays openable until it expires or
  is revoked; revoking stops future downloads but cannot retract a credential
  already imported or erase a downloaded copy.

## The browser extension

Desktop only, and off until the user turns it on in Settings. Rowel speaks the
KeePassXC-Browser protocol (`src-tauri/src/browser/`), so the stock
KeePassXC-Browser extension fills from it; there is no extension code of ours.
This is a trust boundary of its own: the browser is a far larger and more
exposed process than the webview, and it is not ours.

- **What the extension can reach** is what the user could copy out of the app
  for the site in front of them: the logins whose website is the page's host or
  a parent of it (`accounts.example.com` gets `example.com`'s, not the other
  way round, and `notexample.com` gets neither), the current TOTP code of one
  of those, a password generated to the Settings defaults, and the lock. It
  can also save a login: a new one for the page, or a new username and
  password over a login it was handed. Every one of these but a generated
  password needs an association, the lock included, so a stranger on the
  socket cannot end the user's session. It gets no list of the vault and no
  other kind of entry. **Nothing while locked:** every request but the key
  exchange is refused before it is even opened, and the one thing a locked
  vault does for the extension is bring the app forward to be unlocked. Lock
  and unlock are pushed to every connected extension, so it stops offering
  fills the moment the vault seals.
- **Passkeys.** A page's `navigator.credentials.create` or `.get` reaches the
  host through the extension as the page's options plus the page's origin,
  and the host plays the WebAuthn *client* (`browser::passkeys`): the origin
  has to be `https://` (or `http://localhost`), the rpId the page claims has
  to be the origin's host or a registrable suffix of it — `login.example.com`
  may claim `example.com`, not `co.uk` and not `other.example.com` — and
  `clientDataJSON` is written here, so the page never gets to choose what is
  signed over. The CTAP2 half is the vault's own authenticator (the "Passkeys"
  section above); the private key never leaves the core, and what goes back
  is the attestation or assertion, inside the sealed reply. **Every ceremony
  is asked for**, in a dialog that names the rpId the passkey is for and,
  under it, the page that asked — so a subdomain asking for its parent's
  passkey is in plain sight — and refused after a minute unanswered. It is
  the same dialog slot an `associate` waits in: one security decision is on
  screen at a time, whichever kind, and a second ask of either kind while
  one is up is refused rather than queued. A sign-in names the account it
  would be as, and when
  the vault holds several passkeys for the site that the request admits (a
  site asking for a discoverable credential names none), the dialog offers
  the choice: the request is narrowed to the one picked before the
  authenticator sees it, so a yes never signs as an account the user was
  not shown. A registration whose `excludeCredentials` names a passkey the
  vault holds is refused before the user is asked, and a sign-in the allow
  list leaves nothing for is answered "no credentials" before they are
  asked, as KeePassXC does. A new passkey is put on the one login for
  that site, or on a new login when there is none or several (the same rule
  `passkey::store` had). The write is pinned to the session the ceremony
  began in: a lock or a workspace switch while the user decides fails it
  rather than landing it in another vault.
- **How it is admitted.** An extension asks once per vault. The app comes
  forward with a dialog, one ask at a time and refused after a minute
  unanswered, and the extension gets in only if the user gives it a name.
  What is remembered is that name and the extension's identification key, as
  a row inside the vault it was let into (`browser::clients`), sealed with the
  rest of it: the key is the whole credential — the protocol never has the
  extension sign with the private half — so on disk it is as unreadable as
  what it opens. An association is to that vault alone; another workspace on
  the device does not answer to it. Every later request presents the key
  inside the sealed channel, and a connection's key is checked against the
  open vault's list on every request, so forgetting an extension in Settings ›
  Browser extension, or switching workspace, ends its access at its next
  request rather than its next reconnect. Switching the host off ends the
  connections that are up as well.
- **Transport.** The app listens on a local socket beside the vault, created
  owner-only (`browser.sock`, mode `0600`); on Windows it is a named pipe
  whose name carries a digest of the data directory. The browser reaches it
  through a native messaging manifest that names this same binary: launched by
  the browser, it only relays frames between the browser's stdio and the
  socket, holds no key and reads nothing. Every message after the key exchange
  is a NaCl box (X25519, XSalsa20-Poly1305) under keys the extension makes per
  browser session, with a fresh nonce per request and the reply sealed under
  that nonce plus one. What travels in the clear is the key exchange, a
  refusal's error code, and the bare lock and unlock signals.
- **What it does not defend against.** A compromised browser profile, or a
  malicious extension holding an association key, can pull every login it
  asks for by URL — and ask for URL after URL — and overwrite the password of
  any login it was handed. With passkeys it gets less: it can ask for a
  registration or an assertion for any site, but each one costs a click in
  the app's dialog, which names the site, and nothing it can ask for hands it
  a private key. This is the same position as KeePassXC's; the guard is what
  the user lets in, and taking it back. A local process running
  as the same user can connect to the socket and ask to associate, which gets
  it nothing unless the user approves the dialog.
- **Residuals.** The extension keeps its copy of the identification key in the
  browser profile, in the clear: a process running as the same user can lift
  it from there and present it, which is the compromised-OS case below. There
  are no per-site prompts, by design — a fill asks nothing, and URL matching
  is the guard. And the host registers under KeePassXC's own native messaging
  name, since that is the name the extension looks for: on a machine with
  KeePassXC installed as well, a manifest that is not Rowel's is never
  overwritten or removed; Settings reports that browser as a conflict, and it
  stays KeePassXC's until the extension is forked under a name of Rowel's own.

## iOS AutoFill

On iOS the app hands the system the names of its accounts so QuickType can
suggest them (`credential_identities`): for each login with a site and a
username (or, lacking one, its email) the domain and that name, and for each
passkey its rpId, user name, credential id and user handle — no password, no
private key, nothing sealed. They stay with iOS while the vault is locked, as
they do for other password managers, since a lock changes no account, and a
suggestion tapped is what makes the AutoFill extension ask for the unlock. So
that the extension, a separate process with a sandbox of its own, can open the
same vault, the data directory lives in the App Group container
(`group.app.rowel.mobile`) rather than the app's own sandbox — moved there
once, entry by entry, from where older installs kept it — and the
biometric-gated key is stored under that group's keychain access group, which
only the app and its extension are entitled to read; the key's biometric
access control is unchanged.

## What Rowel explicitly does NOT defend against

- **A compromised operating system.** Code running as the user — malware, a
  malicious app with the same privileges, an attacker at local root — can read
  process memory while the vault is unlocked, tamper with the binary, or inject
  into the webview. Rowel cannot protect secrets from the platform it runs on.
- **Keyloggers and screen capture.** The master passphrase (as typed) and any
  revealed secret (as displayed) can be captured by such tools.
- **A coerced or observed unlock.** If the user is compelled to unlock, or a
  biometric is used under duress, the vault opens. Biometric unlock trades some
  resistance here for convenience: anyone who can pass the OS biometric gate on
  that device can unlock.
- **A known master passphrase.** The passphrase is the single root of trust.
  Whoever knows it can decrypt the vault; there is no second factor on the
  encryption itself.
- **Offline brute force at scale.** The failed-unlock backoff described above is
  local state — a counter in a sidecar beside the vault — so it bounds guessing
  at this app's lock screen and nothing else. Someone holding a stolen copy of
  the database deletes the sidecar, or never runs Rowel at all, and attacks the
  file directly. What bounds that is the KDF cost alone: one Argon2id derivation
  at `m=64 MiB, t=3, p=4` per guess, which is why the primitive rather than the
  throttle is the defence that matters here.
- **The clipboard window.** Copied secrets go to the system clipboard. Rowel
  marks them as concealed where the platform has a marker for it and auto-clears
  after a timeout (and on lock), but other apps can read the clipboard during
  that window — and on Linux there is no marker to set, so a clipboard manager
  may keep a copy of its own that no clear of ours reaches.
- **Rollback through Drive's revision history.** Sync is a whole-state merge:
  a device pulls the pack, merges it, and pushes the union. Deletions travel as
  tombstones, and tombstones are reclaimed after 90 days so the vault does not
  grow without bound. Google keeps earlier revisions of the pack, and anyone
  who can write to the Drive account — the account's owner, or someone who has
  taken it over — can restore a revision older than that window. Every device
  then merges the old pack, and a credential deleted (or "deleted forever")
  more than 90 days ago has no tombstone left to refuse it, so it comes back
  everywhere, along with any other state the old revision held. Within the
  window the tombstone wins and the rollback is absorbed. Rowel treats the
  Drive account as the user's own and does not defend against its owner; the
  practical guard is the same as for the account itself: strong authentication
  on the Google account, and re-deleting anything a rollback brings back.
- **Physical memory attacks.** Cold-boot or DMA attacks against an unlocked
  session are out of scope.

## Summary

Rowel's security rests on a SQLCipher-encrypted database, a second app-AEAD layer
that keeps each entry's secrets sealed until reveal, keys that are derived on
unlock, held only in the Rust process, and zeroized on lock, and the absence of
any server that could be breached. It assumes the user's device and operating
system are trustworthy while the vault is unlocked. It does not try to defend a
device that is already compromised. Live key derivation is Argon2id, from a
versioned descriptor in a sidecar beside the vault; PBKDF2 remains only to read
what older formats wrote.
