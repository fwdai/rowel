# Master-password change across synced devices

## Problem

A lockbox's pack in the account is sealed under the key its master password
derives, and the pack header carries the KDF descriptor (`kdf_params_json`)
that key came from. Every device of one lockbox holds a byte-identical copy of
that descriptor in `vault.kdf.json`, and `engine::decode_pack` refuses any pack
whose header differs from it, as a "vault from a different install".

A master-password change (`change_master_password`) re-keys the local database
under a fresh salt and rewrites the sidecar. From that moment:

- the device that changed it (A) refuses the account's pack (old descriptor)
  with the "different install" error and never pushes again;
- the pack in the account stays sealed under the **old** password, and every
  other device (B, C) keeps merging new secrets into it under that password;
- B and C can never learn that the password changed.

A user who rotates a leaked password has not protected the cloud copy, and
their devices drift apart for good. This document is the design that makes a
change propagate.

## Goals

1. After A changes the password, the account's pack is re-sealed under the new
   password as soon as A is online. Nothing B pushed meanwhile is lost.
2. B tells "the password changed on another device" apart from "someone else's
   vault", asks for the new password once, re-keys itself with the existing
   crash-safe saga, and carries on syncing with its own unsynced edits intact.
3. Lockboxes beside the primary that share the password follow in one prompt.
4. Works the same over Google Drive, Dropbox and iCloud; needs no provider
   feature beyond what the engine already uses.
5. An old pack put back into the account (a Drive revision restore, an
   attacker with the old copy) cannot silently move a device back onto a
   retired password.

## Non-goals

- Hiding old revisions the provider keeps (Drive's revision history still
  holds packs under the old password; the threat model says so).
- A generation counter for entries themselves (rollback protection for data
  is a separate item). The counter here is for key epochs only.
- Vaults without a KDF sidecar (they cannot sync today and still cannot).

## Vocabulary

- **Epoch**: one `(kdf_params_json, VaultKey material)` pair. A password change
  retires the current epoch and starts a new one.
- **Generation (`gen`)**: an integer in the KDF descriptor, bumped by every
  re-key of that lockbox. Fresh vaults have none (reads as 0).
- **Retired keys**: the epochs this device has moved away from, kept sealed
  under the current key so the device can still open a pack a slower device
  pushed under a previous epoch.

## Design

### 1. The descriptor carries a generation

`KdfParams::Argon2id` gains `gen: u32`, serialized as `"gen"` **only when
non-zero** (`#[serde(default, skip_serializing_if = "is_zero")]`), so every
descriptor written before this change keeps its exact bytes, and
`from_json(to_json(p)) == p` and `to_json(from_json(s)) == s` hold for every
descriptor the app has ever written (test this).

- `KdfParams::default_argon2id()` is unchanged (gen 0).
- `KdfParams::next_generation(&self) -> KdfParams`: fresh salt, default costs,
  `gen = self.gen() + 1`. `change_master_password` and `rekey_one` use it in
  place of `default_argon2id()` so every re-key bumps the generation of the
  lockbox it re-keys (each lockbox has its own counter).
- Restore, autojoin and adopt (below) install the header **verbatim**, so a
  device that joins later inherits the generation.
- Older builds ignore the field when deriving (serde ignores unknown fields;
  the enum has no `deny_unknown_fields`, confirm) and keep failing the string
  comparison as they do today for any password change. No worse than now.

### 2. Retired keys are kept, sealed under the current key

A new per-workspace file beside `vault.kdf.json`: **`vault.retired.sealed`**.

- Plaintext: JSON `{ "entries": [ { "kdf": "<kdf_params_json verbatim>",
  "material": "<base64 VaultKey::biometric_material()>", "retired_at": <ms> } ] }`,
  newest first, **at most 3 entries** (prune the oldest beyond that).
- Sealed with `crypto::seal_aead` under a new HKDF subkey of the **current**
  `VaultKey` (`VaultKey::retired_keys_key()`, label `retired-keys-key`, next
  to `delete_marker_key`), with the workspace id as AAD (the pattern
  `crypto/mod.rs` uses for workspace keys).
- The material, not just the SQLCipher key, is stored: `VaultKey::from_material`
  gives back every subkey (SQLCipher, cryptor for the token file, delete-marker
  key), which the engine and the marker check need.
- Read: `retired::load(dir, &VaultKey, workspace_id) -> Vec<RetiredKey>`. A
  file that is missing, does not open, or does not parse is an **empty list
  plus a warn log**, never an error: losing it only degrades to today's
  behaviour for merging an old-epoch pack.
- Written **inside the re-key saga** (`auth::rekey` / `rekey_vault`), in the
  same step that writes the new sidecar: unseal the existing list under the
  old key, push the old epoch on the front, prune to 3, seal under the new
  key, atomic write. The file joins the snapshot/rollback pair exactly as the
  sidecar does (`publish_snapshot` backs it up, `roll_back` restores it,
  `discard_snapshot` retires the backup), so a crash on either side of the
  commit leaves sidecar, database and retired list agreeing.
- The file is added to every list `storage.rs` keeps of per-workspace files
  (move, delete, stale-suffix handling, tests), and to the iOS App Group
  layout if a list exists there.
- Zeroize on drop for the decoded material (`Zeroizing`).

### 3. The engine opens a pack by epoch and reports what it found

`SessionVault::capture` also loads the retired list (under the session key,
once per run). `decode_pack` becomes:

```
header = unpack(bytes).kdf_params_json
if header == local.kdf        -> open with local key            (today)
else if retired has header    -> open with that retired key;
                                 snapshot.opened_under_retired = true
else:
   remote_gen = KdfParams::from_json(header).gen()   (unparsable -> foreign error, as today)
   local_gen  = local params gen
   if remote_gen == 0 && local_gen == 0 -> Error::Other(foreign_vault_error)   (today)
   if remote_gen >= local_gen           -> Error::PasswordChangedRemotely
   if remote_gen <  local_gen           -> Error::VaultUnderRetiredPassword
```

Why those three:

- **Retired epoch** means a slower device pushed under the password this one
  already replaced, or an old revision was put back. Both are merged
  normally (last-writer-wins per row), and then the pack **must** be rewritten
  under the current epoch even when the merged digest equals the remote's
  (otherwise the account stays under the old password). So in `engine::sync`
  step 5, the "digests equal, nothing to say" early return is skipped when
  `incoming.opened_under_retired` is set. `SyncOutcome` gains `rekeyed_remote:
  bool` for the log.
- **Higher (or equal, non-zero) generation** means another device re-keyed:
  this device has to adopt the new password. Equal-and-non-zero is the
  concurrent-change case (two devices each bumped once); the only useful
  answer is the same prompt, and a wrong password there explains itself.
- **Lower generation with no retired key for it** means the account holds a
  copy under an epoch this device retired and no longer has the key for
  (pruned, lost, or this device joined after that epoch). Pushing would
  clobber whatever that device wrote; opening is impossible. The run stops
  without pushing. The way out is either that device adopting the new
  password (its next run prompts it), or the explicit **replace** action below.

Error variants (core `error.rs`, with kinds `passwordChangedRemotely` and
`vaultUnderRetiredPassword`, mapped exactly like `VaultDeletedRemotely`):

- `PasswordChangedRemotely` — "the master password of this lockbox was changed
  on another device".
- `VaultUnderRetiredPassword` — "the account holds this lockbox under a
  password this device has already replaced".

Both reach the UI through `sync:status` (`SyncFailure` from the error) like
every other run failure.

**Markers:** `marker::verified` tries the current marker key first, then the
marker key of each retired epoch, so a "delete everywhere" written just before
or after a password change is still honoured on every device.

### 4. Adopting the new password: `sync_adopt_password`

New command on the open workspace: `sync_adopt_password(password:
Zeroizing<String>) -> Result<Vec<String>>` (returns the ids of lockboxes
beside it that stayed on another password, like `change_master_password`).

1. Guards: unlocked; sync configured; `setup::begin_step` held (it re-keys
   through the workspace paths, same as `change_master_password`); the
   failed-unlock backoff is consulted first and a wrong password here is
   penalized exactly like a wrong unlock (the pack bytes are attacker-reachable,
   so a prompt must not be a free oracle).
2. Fetch the lockbox's pack (resolve id + fetch, the same path `sync::run`
   uses; factor a `sync::fetch_pack(app, cryptor, marker_key) ->
   Option<(Vec<u8>, revision)>` out of it). No pack, or a verified delete
   marker, is the corresponding error. A pack with no header change (header ==
   local sidecar) means someone already adopted here: just launch a run and
   return.
3. Parse the header; `KdfParams::from_json` + the existing `validate` ceilings
   (a hostile header cannot ask for more than 1 GiB). Derive `K_remote =
   derive(password, header)` on the blocking pool.
4. **Verify** by opening the snapshot with `K_remote` in scratch
   (`records_from_snapshot`); discard the result. Failure → `InvalidPassword`
   (penalized). The UI copy on failure explains the two possibilities (typo,
   or the account's copy is not one this user re-keyed).
5. **Re-key** the open workspace from the session key to `K_remote` with the
   header **verbatim** as the new sidecar. Factor the body of
   `change_master_password` after `derive_both` into
   `rekey_open_workspace(app, state, old_key, new_key, params_json)` (lease,
   saga, adopt back into the session, ring update, token re-seal, biometric
   re-enrollment) and call it from both. The saga writes the retired list
   (old epoch now retired).
6. **Beside**: for every other registered workspace that is sync-configured,
   best effort and independent: open its token file with its ring key (skip
   if the ring does not hold it — a lockbox on a password of its own prompts
   when it is next opened), fetch its pack header; if it equals its sidecar,
   nothing to do; otherwise derive `password` against that header, verify by
   opening its snapshot, and `rekey_workspace(dir, ring_key, K_i, header_i)`
   with the token re-seal `rekey_one` does. A workspace whose verification
   fails is reported in the returned list and left alone.
7. Launch a normal run for the open workspace. Its pull now opens (header ==
   sidecar), merges, and pushes this device's unsynced edits.

### 5. The changing device: `change_master_password`

- **Before** the derive: if sync is configured, run one sync to completion
  (claim the run; if a run is already in flight, wait for it — the claim is
  the lock). Failure (offline) is logged and the change proceeds: the retired
  key covers the merge later. This narrows the window in which another
  device can push under the old epoch.
- The new params are `current.next_generation()` for the open workspace and,
  in `rekey_one`, `own_sidecar.next_generation()` for each lockbox beside it.
- **After** the change (after `rekey_beside`): launch a run for the open
  workspace. Its pull finds the account's pack under the just-retired epoch,
  opens it with the retired key, merges, and pushes the pack under the new
  epoch (forced by `opened_under_retired`). The account is re-sealed within
  one run of the change.

### 6. The escape hatch: `sync_replace_remote`

Offered in the `VaultUnderRetiredPassword` state and in the
`PasswordChangedRemotely` state (amended: a user who never changed the password
anywhere is looking at a copy that is not theirs, and needs the same confirmed
way out). The backend refuses it unless the open lockbox's last run failed with
one of those two kinds. Confirms, then pushes
this device's pack over the account's (upload with the current head revision
as the base, so a concurrent push still loses cleanly), and launches a run.
Edits that only existed in the account's copy are lost, and the UI says so.
The other device's next run then prompts it for the new password.

### 7. UI

- `src/api/errors.ts`: the two kinds with `describeError` copy.
- `src/api/sync.ts`: `syncPasswordChangedRemotely(sync)`,
  `syncUnderRetiredPassword(sync)`, `syncAdoptPassword(password):
  Promise<string[]>`, `syncReplaceRemote(): Promise<void>`.
- **Prompt**: when the status carries `passwordChangedRemotely`, a dialog
  (reuse the app's existing dialog/sheet component, the one the browser
  association prompt uses) titled "Master password changed on another
  device", body "Enter the new master password to keep this lockbox in sync.
  This device will switch to it.", one password field, "Use new password".
  Shown from the Settings › Lockboxes row (beside the existing deleted-remotely
  handling in `WorkspaceList.tsx`) and reachable from the header sync
  indicator's error state. On a wrong password: "That isn't the new password.
  If you didn't change it on another device, the account's copy may not be
  yours; you can replace it from Settings › Lockboxes." On success, if the
  returned list is non-empty: a note "These lockboxes use a different
  password and will ask for it when opened: …".
- **Retired state**: in the same row, a `Note` with the error text and a
  "Replace the account's copy with this device's" button behind a confirm
  that names the loss.
- All 10 locale files get the new strings (English in every file is
  acceptable if that is what the repo does for new strings; otherwise follow
  the convention used by the delete-marker change).

### 8. Security notes (for the threat model)

- The retired list is sealed under the current key with the workspace id as
  AAD; it is only readable by someone who already holds the current key, so
  keeping it does not weaken a rotation. It is never written to the account.
- A header in the account is attacker-writable. The worst it can do: ask for
  a 1 GiB derive (existing ceiling), or raise the prompt with a password the
  user does not know, which fails locally and leaks nothing (and is
  penalized). Opening the pack with the derived key is what proves the pack
  was sealed by someone who knew the password the user typed.
- A pack from a retired epoch is merged, never installed: an old revision put
  back cannot move the device to the old password; it is re-sealed under the
  current one on the next run.
- A device that joined after an epoch cannot verify a pack from before it;
  it stops and offers replace. That is the one place a user can lose edits,
  and they are told.
- Old provider revisions stay under the old password (unchanged, documented).

### 9. Tests (acceptance)

Core:
- `gen` JSON round-trips byte-for-byte; absent when 0; `next_generation`
  bumps and re-salts.
- Retired list: seal/unseal; wrong key or wrong workspace id → empty; prune
  to 3 keeps the newest; survives the saga's rollback (restored with the
  sidecar).

Engine (fake `LocalVault`/`Remote`, as the existing tests do):
- Remote under a retired epoch: opened, merged, and pushed **even with equal
  digests**; outcome says so.
- Remote gen higher: `PasswordChangedRemotely`, nothing pushed.
- Remote gen lower, no retired key: `VaultUnderRetiredPassword`, nothing
  pushed.
- gen 0 vs gen 0 mismatch: still the foreign-vault error.

Commands (two workspace dirs sharing one `FakeStore`, the "two devices" test):
1. A and B hold the same lockbox and have synced.
2. A changes the password → A's sidecar gen 1, retired list holds gen 0; the
   run after the change re-seals the account's pack (header gen 1).
3. B edits an entry, runs: `PasswordChangedRemotely`; B's local edit intact.
4. B adopts with the wrong password → `InvalidPassword`, B untouched,
   backoff recorded.
5. B adopts with the new password → B's sidecar equals A's header byte for
   byte; B's session key opens; B's retired list holds gen 0; a run then
   pushes B's edit; A's next run has it. Digests converge.
6. A device C with a copy of the **gen 0** pack pushes it back: A and B merge
   it with their retired key and re-seal under gen 1.
7. A fresh device D restores from the account: sidecar gen 1 verbatim,
   unlocks with the new password.
8. Beside: a second lockbox on the same password follows in B's adopt; one on
   its own password is returned in the list and untouched.

Frontend: the prompt renders from the status kind, submits, shows the
wrong-password copy, shows the beside list; the retired state shows the
replace action behind a confirm.
