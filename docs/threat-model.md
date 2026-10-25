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
vault is a set of locally stored, encrypted SQLite databases under the OS
app-data directory (on iOS, the App Group container; see "iOS AutoFill"), one
per lockbox (Personal, Family, Work), one lockbox open at a time. Where this
document speaks of the vault's database or key, it means the open lockbox's.
The code calls a lockbox a *workspace* (`workspace.rs`, `workspaces.json`);
see Vocabulary in the README. There is no account server and no backend that
holds user secrets. The
webview is locked down by a strict CSP (`default-src 'self'`, `connect-src
'self'`, `object-src 'none'`, `frame-src blob:`, `base-uri 'none'`, and no
`'unsafe-inline'` anywhere: the two `<style>` blocks in `index.html` run under
the per-load nonce Tauri adds to `style-src`; see `src-tauri/tauri.conf.json`.
`blob:` is allowed for images and frames only: the attachment preview shows a
picture, or a PDF in the engine's own viewer, through an object URL of bytes
the webview already holds, and WebKit's `'self'` does not match `blob:`),
and external links are opened through the OS only for `http`/`https` URLs
(`opener:allow-open-url` scope in `capabilities/default.json`; plain `http` is
kept because routers and intranet logins have no other address, and the scope
exists to shut out `file:`, `javascript:` and custom schemes, not to upgrade
transport). Seven commands read a path the webview names — `scan_image` (a photo
of a card or identity document), `read_env_file`, `attachment_add`,
`import_entries` (a password manager's export), `import_swftx` (a legacy
backup merged into the open vault), `setup_restore_from_file` (a `.rowel`
backup restored on a fresh install) and `setup_restore_from_legacy` (a `.swftx`
made the vault of a fresh install, under its own password) — and none trusts
the path it is handed; all seven read only granted paths. A path is readable only if it was granted by
one of three events the core itself observes: the OS file dialog run from Rust
(`pick_file`; the webview holds no `dialog:allow-open` permission to open one of
its own), an OS drag-and-drop onto the window, or the OS opening a `.rowel` or
`.swftx` with the app (`opened.rs`) (`PathGrants`, one-shot, consumed on use).
A grant carries the purpose it was made for — the picker's kind (`image`,
`env`, `attachment`, `import` or a backup), for a drop the file's kind, and for
a file the OS opened a backup — and each reader spends only a grant of its own
purpose: a path the user chose in an env-file dialog cannot be read by
`scan_image`, so the dialog's framing is part of what was consented to. The
one exception is a drop, which `attachment_add`, `import_entries`,
`import_swftx`, `setup_restore_from_file` and `setup_restore_from_legacy` may also spend: the file was
handed to the app itself, the drop target says what it is, and each of those
takes the file into the vault and returns no more of it than the kind's own
reader would. An import's preview and a backup read again after a mistyped
password check the grant without spending it; the write or the restore that
succeeds spends it. The session must then still be the one that asked — the epoch
is re-checked after the read, so a lock or a lockbox switch mid-read discards
the bytes. Only then is the file classified: the extension against a fixed
image list (`scan/mod.rs`), the `.env` name-or-parse check and the 1 MiB cap
(`commands/env.rs`). That classification is a sanity check on a file the user
already chose, not the authorization; the grant is. Two commands read no path
at all: `scan_image_bytes` and `attachment_add_bytes` take a file the webview
already holds — the photo the phone's camera just took through its
`<input type="file" capture>`, which has no path on disk — as the request's
raw body, or as a JSON array of the same bytes when Tauri has fallen back to
its `postMessage` IPC (and, for an attachment, the entry and the file's name as
its headers). The photo library and Files on the phone go through `pick_file` and
a grant like any other. No grant applies to the camera's photo because none
is needed: the bytes are data the webview had anyway, so neither command can be
used to read anything it did not; the result is the recognized fields, or a
sealed file under a name kept to its last path component. The
zeroizing here is best effort and covers the Rust working copy only: the
command clones the body (Tauri lends it borrowed) and that clone and the
recognized text are zeroized once parsed, while the IPC body Tauri holds, the
`NSData` copy Vision decodes, and the webview's own buffers have ordinary
lifetimes and are dropped, not scrubbed.

## What sits on disk

Each lockbox is a **SQLite database sealed with SQLCipher** (`vault.db`,
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
  `reveal_entry` in `src-tauri/src/commands/vault.rs`). Custom fields (`extra`)
  ride in the whole-entry seal only: one the user marks concealed is masked on
  screen and exported as a hidden field, but is not sealed again on its own.
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
  so a lock or a lockbox switch during a lookup discards the result rather
  than writing it into whichever lockbox is open when the fetch returns. The
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

A third label, `attachment-aead-key`, keys the attached files (see
"Attachments" below). One KDF pass covers all of them, and no subkey reveals
another.

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
>   and a password change writes fresh params (one generation on) and a fresh
>   sidecar, keeping the old epoch in the retired-keys list (rolling all three
>   back together if it is interrupted; see "Key lifecycle" 5). A database that exists with *no*
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
  (`NotFound`) rather than saved as a credential that could never sign.
- The one way a passkey key leaves the app is an **explicit user-initiated
  export** (a `.rowel` backup, which is the encrypted vault snapshot itself, or
  Bitwarden JSON, which is plaintext by construction and carries the key as
  base64url) — the same deliberate exposure the password export already is.
  Both exports ask for the master password again (`export_vault`,
  `export_entries`): an unlocked session alone does not carry the vault away.
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

### Attachments

Any entry but an env file can carry files (`commands/attachments.rs`). A file
is never put inside the entry: an entry is sealed and revealed whole, so its
bytes would reach the webview on every reveal.

- **Where they live.** One row per file in an `attachments` table beside
  `entries`, inside the same SQLCipher database. `entry_id`, `name`, `mime`,
  `size` and the timestamps are plaintext columns, protected at rest by
  SQLCipher like an entry's title. The `blob` is the file sealed with the app
  AEAD on top.
- **How they are keyed.** AES-256-GCM under a third HKDF subkey of the master
  (`attachment-aead-key`, beside the SQLCipher and payload labels in
  `crypto/vault.rs`), with a fresh nonce per file and the attachment id as
  associated data, so a blob moved under another row fails its tag
  (`crypto::AttachmentCipher`). A master-password change re-seals every blob
  under the new subkey with the payloads, before the `PRAGMA rekey`. A
  legacy-keyed (sidecar-less) vault is refused attachments rather than given a
  second scheme.
- **The webview never sees the bytes.** It names a file; Rust reads it, seals
  it and stores it. The path must carry a grant (`PathGrants`): the attachment
  picker (`pick_file` with the `attachment` purpose) or an OS drop on the
  window, which a drop may be spent on whatever its kind, since attaching hands
  nothing back. A save unseals in Rust and writes through the same save-dialog
  writer as an export (`save::save_export`: owner-only on desktop, staged in
  the sandbox for the iOS exporter). The webview receives name, type and size.
  Add and save re-check the session epoch after their off-thread step, so a
  lock or lockbox switch mid-read discards the bytes; every command fails on
  a locked vault.
- **The caps.** 10 MiB per file (`MAX_ATTACHMENT_BYTES`), enforced on the read
  itself (`read_regular_file_capped`), refused as `fileTooLarge`. 128 MiB for
  all of a lockbox's files together (`MAX_VAULT_ATTACHMENT_BYTES`), archived ones
  included since they ride every pack until purged, refused as `vaultFull`
  before the file is sealed. That is half the 256 MiB pack and backup-restore
  cap, which also has to hold the entries, the indexes and the WAL. The entry's
  list mentions the budget only once more than 80% of it is spent. As a net
  for a lockbox that got past it some other way (an older build, a hand-edited
  database), the sync engine refuses to upload a pack larger than the download
  cap, so no device is handed a pack it cannot pull.
- **Delete, purge, sync.** A file's state is its entry's: a file that still
  holds its bytes is live under a live entry and tombstoned under an archived
  one, and under a purged or missing entry it is a purged shell. Removing a
  single file goes straight to that shell, since nothing restores it — so a
  tombstone that still has a blob can only be an archive, and what makes a
  file restorable is that it has its bytes, never the instant it was archived
  at. Deleting an entry archives its live files at the same instant; restoring
  it brings back every file it still holds, including one a peer archived at
  some other instant, and an entry a merge brings back live (a peer edited it
  after it was archived here) gets its files back the same way. "Delete
  forever" (and a purge arriving through sync) empties every file's blob, name
  and type in place, leaving a content-free shell zeroed by `secure_delete`.
  Sync carries the rows in the pack and merges them per row, last-writer-wins
  on `updated_at`, with the shell absorbing as an entry's is, so a removed
  file does not come back from a peer that still holds it. Every write that
  touches both tables (delete, restore, purge, merge, the tombstone sweep) is
  one transaction, and each of them ends by bringing every file into line
  with its entry (`reconcile_attachments`, whose stamps come from the rows
  alone so every device writes the same thing), so a file a peer added before
  it saw the purge, or one a crash left behind, becomes a shell on the next
  merge, and no file is left hidden under an entry that is on screen. Listing and
  saving a file also require its entry to be live. Tombstones are reclaimed
  with the entries' after 90 days. Shares, the Bitwarden/CXF/CSV exports and
  the legacy `.swftx` export carry no attachments; a `.rowel` backup is the
  database snapshot and carries them.

### Password history

A login keeps up to ten passwords it has replaced (`passwordHistory`, newest
first), **inside the sealed payload** like the password itself — no column, no
table, nothing sync or the store can read. It is written only by the core:
every save that can change a password (`save_entry`, the browser extension's
`save_login`) unseals the row it replaces and runs `Entry::record_password_change`,
which keeps the stored list and stamp and ignores whatever the caller sent, so
the webview cannot plant a "previous" password or backdate a rotation. Clearing
(`clear_password_history`) rewrites the stored row in the core. A share strips
the history before it is sealed and again on receipt, the audit never reads it,
and it leaves the app only in an explicit export (a `.rowel` backup, or
Bitwarden JSON's `passwordHistory`, plaintext by construction).

## Key lifecycle across the process split

1. **Derive off the UI thread on unlock.** The user enters the master passphrase
   in the webview. It is passed once to the Rust core, which derives the key
   material and opens SQLCipher (which runs its own internal KDF) on a blocking
   thread (`spawn_blocking` in `commands/auth.rs`), so the UI never stalls. The
   passphrase itself is not stored. One thing outlives the unlock by a bounded
   stretch: when the vault syncs, a second `Zeroizing` copy of the passphrase is
   moved into a blocking-pool task that tries it against the Google account's
   packs this device lacks and adds the ones it opens as lockboxes
   (`commands/autojoin.rs`). The copy is spent on those Argon2id derives, is
   never written anywhere, and is scrubbed when the task ends. How long that is:
   the task stops starting on new candidates 60 seconds in (`JOIN_BUDGET`), so
   the copy lives for at most that budget plus the one download in flight when
   it ran out — and that download is bounded by the 256 MiB cap over the slowest
   rate that still counts as progress, not by a deadline, since a steady trickle
   never trips the read-stall one. A large pack on a slow link stretches the
   copy's life by the length of that download. Whatever was not tried stays on
   offer in Settings › Lockboxes. Biometric unlock has no passphrase and does
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
   a lockbox switch, also **clears the clipboard** if it still holds a secret
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
5. **One unlock for every lockbox (`appkey.rs`).** The primary lockbox's
   key is the *app key*. Every other lockbox keeps a copy of its own key
   sealed under it — AES-256-GCM under an HKDF subkey of the app key, bound to
   the lockbox id as associated data — in a `0600` sidecar beside its
   database (`vault.key.sealed`). Opening the primary, by passphrase or by the
   biometric item above (which stores the app key and nothing else), unwraps
   every sidecar into an in-memory ring, so a lockbox switch opens the next
   database without a prompt and without an Argon2id run. The ring lives as
   long as a session would: it ends with every lock, the auto-lock included,
   and the sidecars are worth nothing without the app key. They are also local
   to the device on purpose — the pack a lockbox syncs stays under its own
   passphrase-derived key, so another device restores it with the passphrase
   exactly as before. A lockbox that has never been opened with its
   passphrase on a device has no sidecar there; its first switch lands on its
   lock screen, and that unlock writes one.

   **The rule: one vault passphrase opens every lockbox.** The vault passphrase
   is the one the primary opens with, and no lockbox is sealed under the app
   key on the strength of any other (`commands/vault_password.rs`). A create
   proves the typed passphrase against the app key before it makes anything.
   Every path that opens or installs another lockbox with a typed passphrase —
   its lock screen, a restore from a signed-in account or from the account's
   other vaults — first lets that lockbox prove the passphrase, then derives it
   under the primary's descriptor and compares the result with the app key in
   constant time (or, before the primary has been opened, asks the primary's
   own database), under the primary's failed-attempt backoff: a restore can
   bring in a pack sealed under any passphrase an attacker likes, so the
   comparison must not be a free oracle on the vault passphrase. The vault
   passphrase opens the lockbox and seals it under the app key. Any other
   leaves it closed — a restore keeps it installed and recorded but unsealed —
   and is answered with `VaultPasswordRequired`; the user is asked for the
   vault passphrase, and `workspace_adopt_vault_password` proves both
   passphrases (each under its own backoff), re-keys the lockbox under the
   vault passphrase with a fresh salt and its next generation through the same
   saga a change runs, re-seals its token file, and only then seals its key
   under the app key. So a lockbox on a passphrase of its own cannot exist: it
   is moved onto the vault passphrase the first time it is opened, behind a
   vault-passphrase prompt. (Auto-join only ever tries a passphrase that has
   just been proved to be the vault passphrase, so what it adds needs no
   prompt.)

   Every step that settles which key the app key is and then reads or writes
   sidecars by it — a proof filling the ring, a passphrase change replacing
   the key, a delete promoting a lockbox, a move onto the vault passphrase —
   runs whole under one lock (`AppState::sidecars`), so a slower proof of a
   retired key cannot reseal a sidecar under it, or remove a fresh one as
   stale, behind a change that has already landed. A passphrase *change*
   (`change_master_password`) keeps the rule true: after the open lockbox is
   re-keyed, every other lockbox is re-keyed too — each under a fresh salt of
   its own, through the same snapshot-guarded saga on its own files, and with
   its Drive token re-sealed under its new key — so no lockbox stays
   restorable on another device only under the passphrase the user has just
   replaced. A lockbox the passphrase does not open should no longer exist, and
   is logged as an error and left untouched (its own passphrase is not the one
   that was proved); one whose re-key fails stays on the passphrase it had.
   Both are reported back so the UI can say the new passphrase does not open
   everything on the device.

5. **A passphrase change reaches the other devices of a synced lockbox**
   (`docs/password-change-sync-design.md`). Every re-key moves the lockbox to a
   new *key epoch*: a descriptor with a fresh salt and a generation one past
   the old one (`KdfParams::gen`, written only when non-zero so every older
   descriptor keeps its bytes). The epoch left behind — the old descriptor
   verbatim and the old key material — is kept beside the vault in
   `vault.retired.sealed`, newest first and at most three, sealed with
   AES-256-GCM under a subkey of the *current* key
   (`VaultKey::retired_keys_key`) with the workspace id as associated data. It
   is only readable by someone who already holds the current key, so keeping it
   does not weaken the rotation, and it is never written to the account. It is
   written inside the re-key saga and snapshotted and rolled back with the
   sidecar; a list that does not open reads as empty, which only costs the
   merge of an old-epoch pack. (A primary deleted and replaced by a promoted
   lockbox changes that lockbox's id, so its list stops opening; the same
   degradation.)

   The changing device syncs once before the change and starts a run after it;
   that run opens the account's pack with the retired key, merges it, and
   pushes it under the new epoch even when nothing else differs. Every other
   lockbox the change re-keyed is synced the same way straight after its
   re-key, without being opened as the session (`sync::run_workspace`, under
   the run claim the change holds), so the account holds every lockbox under
   the new passphrase within the change and the other devices answer one
   prompt for all of them. An adopt does the same for each lockbox it moves,
   and a move onto the vault passphrase for the lockbox it moved. These runs
   are best effort: one that fails leaves that lockbox's account copy to its
   next run, which the retired key covers. A pack under
   a retired epoch — a slower device's push, or an old revision put back — is
   always *merged* and re-sealed, never installed, so it cannot move a device
   back onto the replaced passphrase. A pack whose header is neither current
   nor retired is told apart by generation: the same or later is "changed on
   another device" (`PasswordChangedRemotely`), earlier is "under a passphrase
   this device replaced and lost the key of" (`VaultUnderRetiredPassword`), and
   two generation-0 descriptors are another install's vault, as before. None of
   those runs pushes. A "delete everywhere" marker opens under the current
   marker key or any retired epoch's.

   The account's header is attacker-writable. Adopting the new passphrase
   (`sync_adopt_password`) derives the typed passphrase under that header —
   bounded by the KDF ceilings, so the worst a hostile header asks for is a
   1 GiB derive — and proves it only by opening the pack's snapshot with the
   result, which shows the pack was sealed by someone who knew that
   passphrase. A wrong one leaks nothing and is penalized exactly like a wrong
   unlock (the same backoff, consulted first), so the prompt is not a free
   oracle against the pack. The adopted header becomes the sidecar byte for
   byte, through the same saga a local change runs; lockboxes beside it that
   the ring holds and that share the passphrase follow, and the rest are
   reported. Replacing the account's copy with this device's
   (`sync_replace_remote`) is allowed only while the last run failed with one
   of the two errors above — including "changed on another device", for a user
   who never changed the passphrase and is looking at a copy that is not
   theirs — behind a confirmation that names the loss: what only the account's
   copy held is gone. The upload is conditional on the head revision it read.
   A device that joined after an epoch cannot verify a pack from before it, and
   that replace is the one place a user can lose edits through this flow; they
   are told. Old revisions the provider keeps stay sealed under the passphrase
   they were written with.

## Fresh start and explicit import

`v1-0-0` starts with an **empty SQLite vault**. `app_status.initialized` is true only when
the encrypted database exists; a legacy `vault.swftx` file alone does **not**
count. Nothing is migrated automatically on unlock.

Bringing existing data forward is an **explicit** action, and there are three paths:

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
  in Settings, for a vault that already exists.
- **Restore a legacy vault as a new vault** (`setup_restore_from_legacy`,
  `commands/setup.rs`): the first-run twin of the import above, for an install
  with no vault to merge into. The `.swftx` is opened under its own password
  first — a wrong one writes nothing — then a vault is created under that same
  password and the entries are re-sealed into it by the same loop. A failure
  past the open removes the fresh vault again.

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
  sync channel yields ciphertext, not secrets. The one thing in the account
  other than a pack is the "deleted everywhere" marker (`<vault-id>.deleted`)
  that stops every device syncing a lockbox, and it is authenticated rather
  than trusted for existing: its body is sealed with AES-256-GCM under a
  subkey of the lockbox's own key (`VaultKey::delete_marker_key`), with the
  vault id as associated data (`sync::marker`). A marker that does not open —
  empty, as older builds wrote it, planted by someone with write access to the
  folder, or copied from another lockbox — is logged and treated as absent.
  And no device deletes a pack because it found a marker, a valid one
  included: only the device that ran "delete everywhere" removes the pack
  (beyond a first upload taking back the pack it has just written, when a
  valid marker landed while it was writing — that one file, by the id its
  upload returned, never another device's pack of the same name), so
  a forged or replayed marker can at most stop sync, and the UI then offers to
  re-upload the copy on the device (which clears the marker) as well as to
  delete it there.
- **A stalled or hostile network peer.** Every HTTPS client in the app shares a
  connect deadline and a read-stall deadline (`sync::http_client_builder`), so
  a connection that stops answering fails the run instead of holding the
  `syncing` flag — and with it manual sync and lockbox switching — until the
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
  sanitized again (id, timestamps, favorite, passkeys and password history
  stripped), its kind is
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
KeePassXC-Browser protocol (`src-tauri/crates/broker`, hosted by the app's
`src-tauri/src/browser/`), so the stock
KeePassXC-Browser extension fills from it; there is no extension code of ours.
This is a trust boundary of its own: the browser is a far larger and more
exposed process than the webview, and it is not ours.

- **What the extension can reach** is what the user could copy out of the app
  for the site in front of them: the logins whose website is the page's host or
  a parent of it (`accounts.example.com` gets `example.com`'s, not the other
  way round, and `notexample.com` gets neither). The page's scheme and port
  count too (`host::site_matches`): a login saved for `https://` is not served
  to an `http://` page (bar `localhost`, `127.0.0.1` and `::1`), one saved with
  an explicit port only to that port, and one typed with no scheme at all by
  host alone. That is decided on the website inside the entry's sealed
  payload, after it is opened; the plaintext `url_host` column only picks the
  candidates, so a row re-pointed at another site by someone holding the
  SQLCipher key is not served there. Also within reach: the current TOTP code
  of one of those, a password generated to the Settings defaults, and the lock. It
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
  began in: a lock or a lockbox switch while the user decides fails it
  rather than landing it in another lockbox.
- **How it is admitted.** An extension asks once per lockbox. The app comes
  forward with a dialog, one ask at a time and refused after a minute
  unanswered, and the extension gets in only if the user gives it a name.
  What is remembered is that name and the extension's identification key, as
  a row inside the lockbox it was let into (`browser::clients`), sealed with the
  rest of it: the key is the whole credential — the protocol never has the
  extension sign with the private half — so on disk it is as unreadable as
  what it opens. An association is to that lockbox alone; another lockbox on
  the device does not answer to it. Every later request presents the key
  inside the sealed channel, and a connection's key is checked against the
  open lockbox's list on every request, so forgetting an extension in Settings ›
  Browser extension, or switching lockbox, ends its access at its next
  request rather than its next reconnect. Switching the host off ends the
  connections that are up as well.
- **Transport.** The app listens on a local socket beside the vault, created
  owner-only (`browser.sock`, mode `0600`); on macOS a second one, served
  exactly alike, is in the App Group container the app shares with its Safari
  extension (`~/Library/Group Containers/UFBL3F444A.app.rowel.desktop`, `0700`),
  the one place that sandboxed extension can connect to — any process of the
  same user can reach it as it can the first, and gets no more from it; on
  Windows it is a named pipe
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
  any login it was handed, for as long as a person is driving: in agent mode
  each of those fills is a grant and costs a dialog or a standing rule (see
  **Agents** below). With passkeys it gets less: it can ask for a
  registration or an assertion for any site, but each one costs a click in
  the app's dialog, which names the site, and nothing it can ask for hands it
  a private key. This is the same position as KeePassXC's; the guard is what
  the user lets in, and taking it back. A local process running
  as the same user can connect to the socket and ask to associate, which gets
  it nothing unless the user approves the dialog.
- **Residuals.** The extension keeps its copy of the identification key in the
  browser profile, in the clear: a process running as the same user can lift
  it from there and present it, which is the compromised-OS case below. There
  are no per-site prompts, by design, while a person is driving — a fill asks
  nothing, and URL matching is the guard; while agent mode is on, or when a
  client of kind `agent` requested it, a fill is a grant and asks or is
  covered by a standing grant (see **Agents** below). And the host registers
  under KeePassXC's own native messaging
  name, since that is the name the extension looks for: on a machine with
  KeePassXC installed as well, a manifest that is not Rowel's is never
  overwritten or removed; Settings reports that browser as a conflict, and it
  stays KeePassXC's until the extension is forked under a name of Rowel's own.

## Agents

Desktop only, and off twice over until the user turns it on. This section
covers what the extension's agent mode, the `rowel` command line and the local
socket let an agent do with the vault, and what a person sees of it afterwards.
It describes the free, local tier this build ships. The server tier the spec
plans (org lockboxes with server-held keys, `rowel serve`, Rowel Server, Rowel
Cloud, approvals pushed to a phone, OAuth token mode, remote browsers, policy
as code, workload identity) is not in it, and is named below only where a
reader would otherwise assume it.

**The one honest line.** Rowel's promise is that no backend holds the user's
secrets: a lockbox is sealed under a key derived from the master password and
nothing else can open it. An unattended agent breaks that promise by
definition, since something that is not the user would have to open an item
while nobody is there. Rather than blur the promise, two properties of a
lockbox are kept separate and visible, and both lines are drawn where the
product already has one, at the lockbox:

- **Where the key lives.** Every lockbox in this build is *device-held*: its
  key derives from the master password, it syncs as an opaque pack through a
  cloud account the user owns, and no server can read it. A server-held *org
  lockbox*, whose key would be escrowed to a broker so that it can act while
  nobody is present, is a planned server tier and not in this build. Nothing
  here serves an agent while the vault is locked or the person is away.
- **Whether agents may use it.** Every lockbox has an agent-access switch,
  *Agents can use items in this lockbox* in its settings (`Workspace.agents`
  in `workspaces.json`, off by default, device-local like the colour). Above
  it sits the **Agents** switch in Settings › Browser & agents
  (`settings.browser.agents.enabled`, off by default). Both must be on. A
  lockbox with agent access off never answers an agent, whatever the rules
  say: `AppHost::request_grant` refuses it before policy is consulted. The
  recommended shape is a dedicated lockbox for the items agents need, which is
  the same gesture as splitting Personal from Work.

The two properties are independent. A device-held lockbox with agent access on
serves agents only while the app is unlocked and a person is there to approve,
or has written a standing rule.

**What each component can read**, for a device-held lockbox with agent access
on:

| component | reads |
| --- | --- |
| the app | plaintext, while unlocked |
| the browser extension | the logins for the page in front of it, after URL match; under a grant, the one item the grant admits |
| a local agent (`rowel mcp`, kind `agent`) | nothing: grant ids and states, and the title and username of the items a grant admits |
| a local sidecar (`rowel run`, `rowel proxy`, `rowel redact`, `rowel hook post`, kind `sidecar`) | the value of one field per `resolve`, for a grant that is active and covers that item at that place in that mode |

The invariant that holds in every cell: **a client of kind `agent` never
receives a secret value.** Only `extension` (to fill) and `sidecar` (to inject
at the edge) do, and only for the items a grant names. An agent is refused
every action whose reply carries a secret (`get-logins`, `get-totp`, the
passkey ceremonies, `resolve`) by its kind, so an MCP tool result never
contains a value by construction rather than by filtering.

**Why not an environment variable on the box.** The usual answer for an
agent's secrets fails on an agent's machine for eight reasons, and the design
above is an answer to each: the agent can read it with one shell call, and
will if a web page tells it to; it is standing, alive for the box's lifetime
and every task; it is unscoped, good for every endpoint; it is unattributed,
since the upstream sees the key and not the run or the person; it cannot wait
for a human on a sensitive action; it cannot log into a website; it leaks
through transcripts, logs, screenshots and child processes; and rotation is a
redeploy that burns the key everywhere at once. For a deterministic service a
key in the environment is fine; this design is specific to agent workloads.

**Agent mode and grants.** With both switches on, a fill in agent mode is a
*grant* (`rowel_broker::grant`): this client may use this item at this origin
in this mode until this time. The extension names the page's logins; policy
(`rowel_broker::policy`) decides deny-by-default against the baseline rule,
which lets any client *ask* to fill any login of the lockbox for a 15-minute
window, and against the lockbox's own standing rules; and the app's dialog
asks the person, who allows once, allows for N days, or denies. Allowing for
N days writes a standing rule for that client, that login and that site, kept
sealed in the lockbox's `meta` (`agent_rules`) and device-local like the
extension associations, since it names a client key that exists only on this
device; it is listed and revocable in Settings › Browser & agents. Cards and
identities take a biometric prompt (Touch ID, Face ID, Windows Hello) or the
master password on top, and no rule lowers that floor; a passkey ceremony is
never silent, agent or not, because the tap is the user-presence gesture. A
request the policy does not cover is denied, not asked: asking is itself
something policy grants, so an agent cannot spam the person for items it was
never meant to touch, and a client that reaches a person more than ten times a
minute, or is denied three times in a row, is slowed down (`policy::Limits`)
with an audit row and no dialog.

The grant carries one login to one origin. A second request for the same page
under it is served without asking again, the TOTP step is served under it, and
a page that echoes the secret in text ends it (`exposed`): the extension scans
the page after the fill and again after the submit, clears the fields and
reports it, within same-origin frames and text only, no shadow roots and no
pixels. The extension's own in-page UI is hidden for the duration, so the
agent cannot drive the extension itself. Grants live in memory only: a lock,
an auto-lock or a lockbox switch drops every one, so nothing is served from a
locked vault.

The ask goes through the same consent slot as an `associate` and a passkey
ceremony: one security decision on screen at a time, refused after a minute
unanswered. A grant request that finds the slot taken is told to retry after
a delay rather than queued or refused, and does not count against its rate
limit, so a dialog another client holds can neither be jumped nor used to
starve the one waiting. A credential is released (a fill served, a code
served, a grant reused or activated by a rule) only inside one critical
section under the session and agents locks, in that order, and only if
nothing about access changed since the request looked: the switch state lives
under the agents lock with an epoch beside it, every change to either switch
bumps the epoch and revokes the active grants when the open lockbox's access
is now off, and every release point re-reads both. A request that read "on"
before a switch went off sees the bumped epoch and is refused; locking,
auto-locking or switching lockbox resets the state, bumps the epoch, drops the
grants and closes the ask that is up. The residual is the extension's own,
unchanged: a process running as the same user can lift the association key
from the browser profile and present it as the extension, and the dialog, not
the key, is then what stands between it and a fill; a standing rule it matches
removes that dialog for its site, which is why the rule names one client, one
login and one origin, and ends.

**Agent clients and the MCP server.** A coding agent reaches the same socket
through the `rowel-cli` sidecar (`rowel mcp`, a stdio MCP server for Claude
Code, Codex and Cursor) and associates like an extension does, saying what it
is (`hello { kind: "agent", platform }`); the dialog names the kind and
platform, and an agent is refused outright while the Agents switch is off. The
kind is stored with the association and gates every later request: a client
of kind `agent` may ask for a grant, wait on it, end it and list what its own
rules let it see, and nothing else. An agent's grant to log into a site is
approved in the same dialog, with the agent's stated reason shown as text, and
reaches the browser extension as a content-free wake-up (`rowel-signal`)
followed by a sealed fetch; the extension with a tab at that origin fills
through the ordinary fill path, and the first extension served under the grant
claims it, so a second one asking under the same id is refused. When the grant
ends, by expiry, by the agent, from Settings, by an exposure or by a switch
going off, the extension deletes that site's cookies in the browser profile
that filled it, in every cookie store and partition the browser lists, so the
login the grant enabled ends with the grant; the dialog says so before the
person allows. Safari's extension has no cookies API, and there session end
does not run. The agent's own key is the first of the two CLI keys below.

**The sidecar.** An API key or an `.env` value reaches a command or an HTTP
request without the agent ever holding it. The agent names it by a placeholder
(`rowel://<lockbox>/<item>/<field>`), which is a name and not a capability,
and asks for a grant in `env` or `header` mode naming the full command line or
the host, path and methods; the dialog shows that place and the person allows
once, for N days (a rule for exactly that command line or that request
pattern), or not at all. `env` is never silent unless a rule names the full
line, because a child process can print what it is given, and a rule for
`node deploy.js*` does not match `node -e …`. The value is released by one
protocol action, `resolve`, that only a client of kind `sidecar` may say
(`rowel run`, `rowel proxy`, `rowel redact`, `rowel hook post`), under the
session's switches and the same locks as a fill, for a grant that is active in
the open lockbox and covers that item at that concrete place in that mode;
each resolve is a counted use and an audit row naming the sidecar, the field
and the place, never the value. `rowel run` sets the value in one child's
environment only and passes the child's output through a redactor that
replaces the value and its common encodings (base64 in both alphabets,
percent-encoding, JSON escaping) with the placeholder; the proxy, which listens
on the loopback address only, swaps it into the outgoing request's headers or
its JSON or form body, never the URL, and scrubs the response; the hook
rewrites `NAME=rowel://… cmd` into `rowel run` before the agent's shell runs it
and ends a grant whose value shows in a command's output.

The CLI holds two keys, and the split is the point: the *agent* key, which
`rowel mcp` presents and which cannot resolve whatever it is asked to do, and
the *sidecar* key, which the model-facing surface never holds. On macOS they
are two generic passwords in the login keychain (service `app.rowel.cli`,
accounts `agent` and `sidecar`); on Windows two Credential Manager entries;
elsewhere files readable by the user alone, `~/.config/rowel/agent.json` and
`sidecar.json`. The store is not gated by Touch ID or Hello, since the CLI
runs unattended, but another program reading the item makes the OS prompt the
user, which a file cannot do. What this path does not do: the redactor is a
safety net, not a guarantee (a value transformed any other way passes, and
values shorter than six bytes are never matched, since a needle that short
would redact ordinary text); the child process itself holds the value and can
do anything with it, which is what the full-command-line rule and the dialog
are for; and the grant id is a bearer capability between agent and sidecar,
128 random bits bound to one lockbox, one item set, one place and one mode for
15 minutes, so a process that reads it can resolve exactly what the person
allowed, no more, and only while the grant is active.

**The audit log.** Every decision and every use is a row in the lockbox's
`audit` table (`rowel_broker::audit`), inside the same SQLCipher file as the
items: `requested`, `granted`, `denied`, `used`, `exposed`, `ended` and
`revoked` for grants; `associated` and `revoked_client` for clients;
`rule_added` and `rule_removed` for standing rules; `rate_limited`; and
`pruned_through`. A row carries the actor's kind and name, the agent's run id,
the grant, the item id, the target, the mode, the approval and a small detail
object, and never a secret value, a TOTP code, passkey material or a URL's
query: the target is the page reduced to scheme, host and path, or the command
line clamped to 300 characters. Rows are insert-only and hash-chained per
writer: `hash` is SHA-256 over the writer's previous hash and the row's
canonical JSON, with zeros before the first row, and a writer never writes
two rows at one `seq`. The writer is the install's own id (`storage::install_id`, minted once
into an owner-only file beside the vault; never the workspace id, which is
"default" on every install and would collide after a sync or a restore), so
two devices' rows sit side by side rather than breaking each other's chains.
Rows travel in the sync pack and merge as a union of rows (a row being its
whole contents, never trusted to its stored `hash`), never last-writer-wins
and never tombstoned, so devices converge on the union of their logs and no
device removes another's rows through sync. A copy altered behind an
unchanged `hash` is therefore kept beside the original on every device rather
than taken for it, and verification names it as altered everywhere. A forked chain — two rows at one writer's `seq`, as when an app
folder copied to a second machine carries its install id — is kept whole on
every device rather than each keeping its own side, so the digests agree and
verification names the fork at the same `seq` everywhere; the pack's
digest includes them, so a device whose only change is new audit rows still
pushes, and a restore from a pack carries the history with it. Retention is
365 days: at sync time a device reclaims every writer's leading run of rows
older than that, never past a younger row (so one row with an old stamp cannot
take the history below it), and in its own chain writes a `pruned_through` row naming
the cutoff `seq`, so the chain stays verifiable from there on. Vault Health › *Agent activity* shows the last seven
days of `used`, `denied` and `exposed` events, standing rules older than 30
days and agents not seen in 90 days, and *Verify the log* walks every writer's
chain in `seq` order and names the first break: a missing row, two rows with
the same number, a row that does not follow the one before it, or a row that
was altered. Verification is per
writer and detects a row altered or removed behind the chain's back. The chain
carries no signature, so whoever holds the lockbox key can rewrite a writer's
chain whole and recompute it; that is the compromised-OS case below, and not
something a log inside the lockbox can see.

**Residual risks, stated.**

- **Same-user local processes.** A process running as the same OS user can
  lift the extension's association key from the browser profile, or the CLI's
  two keys from where they live (the login keychain on macOS, Credential
  Manager on Windows, owner-only files elsewhere), and present it. The secure
  store makes a foreign read prompt the user; a file does not. Grants,
  approval modes, the floors, full-command-line targets and the rate limits
  bound what a lifted key buys; they do not remove it. A lifted *agent* key
  buys the right to ask; a lifted *sidecar* key plus a live grant id buys what
  that grant covers, for its lifetime.
- **The logged-in session.** After a fill, the agent's browser holds the
  site's cookies. Session end deletes them with the grant, but an agent that
  controls the browser through the devtools protocol can read them *during*
  the grant, so the end bounds the window, not the capability; a site that
  issues long-lived tokens elsewhere is out of reach; and on Safari the
  deletion does not run.
- **The child process in `env` mode.** It holds the value and can print,
  store or send it. Policy floors the mode to `ask`, a standing rule names the
  full command line, and the redactor is the second net. The mode exists for
  tools that take nothing else and is labelled the weakest for a reason.
- **Vision.** A screenshot taken after a fill can show an autofilled username
  or a password field's dots, and a page that renders a secret as pixels
  passes the exposure check. Redaction covers text, not pixels. An agent that
  needs vision should submit before the next capture.
- **The broker's memory.** Applies only to the server tier that is not
  shipped, where an org lockbox's key would sit in a server's memory while it
  serves; nothing in this build holds a key anywhere but the unlocked app.

## iOS AutoFill

On iOS the app hands the system the names of its accounts so QuickType can
suggest them (`credential_identities`): for each login with a site, a
username (or, lacking one, its email) and a password, the domain and that
name, and for each
passkey its rpId, user name, credential id and user handle — no password, no
private key, nothing sealed. They stay with iOS while the vault is locked, as
they do for other password managers, since a lock changes no account, and a
suggestion tapped is what makes the AutoFill extension ask for the unlock. So
that the extension, a separate process with a sandbox of its own, can open the
same vault, the data directory lives in the App Group container
(`group.app.rowel.mobile`) rather than the app's own sandbox — moved there
once, entry by entry, from where older installs kept it (the log directory
Tauri keeps inside it is not the vault's and stays behind, so that a launch
finding it there again does not take it for a vault); a move that cannot
finish is undone, and the app runs from the old directory, whole, until a
launch that can. A container found holding a vault of its own (one an
earlier install left, while a build without the container ran from the
sandbox) is not written over: that data dir is set aside whole, renamed in
one step to `<data dir>.stale-<time>` beside where it was, never deleted
and put back if the move that follows fails, and the app's — the one it
has been running from, and the one its biometric key opens — moves in —
and the
biometric-gated key is stored under that group's keychain access group, which
only the app and its extension are entitled to read; the key's biometric
access control is unchanged.

A shared container has one rule of iOS's own: a process suspended while it
holds a lock on a file there is ended (`0xdead10cc`), and an idle WAL
connection holds one on the database's `-shm` for as long as it is open. So
while the app is in the background the session keeps no connection to the
vault between uses — the one it held is closed as the app goes, and each use
until the app is back opens and closes one of its own (`Session::set_background`,
`SqliteStore::park`). The key stays in memory, as it does in the foreground:
the vault is as unlocked as it was, and the auto-lock is what ends that.

The extension (`app.rowel.mobile.autofill`: Swift for the sheet, and the
`rowel-autofill` crate over `rowel_core` for the vault) reaches exactly those
two things: the active lockbox's database in the shared container, and that
keychain item. It opens nothing before the user passes Face ID or Touch ID —
the item's access control demands it — and then opens the vault with the same
app key the app's own biometric unlock uses, so a lockbox not yet sealed
under that key (one never opened on this device, or still waiting to be moved
onto the vault password) stays closed to it, and a device with biometric unlock
turned off gives it nothing. It never creates a vault, and writes to one only
to keep a passkey it registered; while a password change the app was
interrupted in awaits its rollback — the recovery snapshot beside the database
says so — it opens nothing at all. The list it shows holds names only — title,
user name, site — and, as passwords, only logins that have one to fill; a
password is unsealed for the one row the user picks, handed to iOS to fill,
and not kept. What iOS holds is a name, never a grant: every fill names the
sites iOS is filling for, and the vault decides by the same rule the list
does, so a suggestion iOS still shows for a site its login has since been
moved off, or a login since deleted, is refused rather than filled.

Passkeys are served on iOS 17 and later, by the same authenticator the desktop
browser host drives (`rowel_core::passkey`). iOS is the WebAuthn client: it
checks the relying party against the calling app or page, writes the client
data, and hands the extension only its hash, so the extension signs exactly
what the OS hands it and has no origin of its own to get wrong. The Face ID
unlock is the user verification the relying party is told of, and the sheet is
the user's say: the account tapped in the list (or the QuickType suggestion,
which names one passkey) for a sign-in, an explicit "Save Passkey" naming the
site and the account for a registration. A passkey registered in the extension
is kept in the vault at once, but reaches QuickType only the next time the app
runs and publishes its identities; until then the sheet's list offers it.

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
  on the Google account, and re-deleting anything a rollback brings back. The
  deletion marker has the same limit: it carries no counter, so a copy of a
  genuine sealed marker put back after the user re-uploaded the lockbox is
  honoured again. That stops sync and nothing more — the pack is never deleted
  on a marker's word — and re-uploading clears it again. After a passphrase
  change, old revisions stay sealed under the passphrase they were written
  with: rotating a leaked passphrase does not re-encrypt the provider's
  history, and anyone holding that passphrase and the account can still read
  them. Putting one back is merged and re-sealed under the current passphrase
  by every device that holds the retired key (see "Key lifecycle" 5).
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
what older formats wrote. An agent gets none of this to hold: it asks for a
grant, a person or a standing rule answers, the extension or a sidecar uses
the credential at the edge, and every decision and use is a chained row in the
lockbox's own audit log. Every lockbox in this build is device-held; a server
that could act for an agent while nobody is present is not part of it.
