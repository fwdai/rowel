# Rowel v1 — Next Work Checklist

The post-remediation work batch for `v1-0-0`. The audit-remediation checklist
([`v1-remediation.md`](./v1-remediation.md)) is **complete** — this is where the next
three initiatives are tracked.

Same working conventions as the remediation doc: each task is a self-contained unit with
scope, concrete steps, and an acceptance gate a subagent can verify; standing code style is
the simplest thing that works, DRY, short implementations; definition of done includes the
repo gates staying green (`bun run typecheck` · `bun run lint` 0 warnings · `bun run test` ·
`cargo test` · `cargo clippy -D warnings`). Zoom out before building — look at how leading
products solve it, don't patch symptoms.

## Status legend

| Mark | Meaning |
|---|---|
| ✅ | Done |
| 🟡 | In progress / partial |
| ❌ | Not started |

---

## 1. GDrive sync ✅

> **Done** — landed as PRs #284 (store merge primitives), #286 (`.swsync` pack format +
> fresh-install restore), #287 (async Drive engine), #288 (post-landing cleanup). Design doc
> with the full algorithm and adversarial review: the "Drive Sync v2" artifact. The 🔴 review
> point was resolved as: sync the whole SQLCipher snapshot in a thin plaintext-KDF-header
> container (Enpass/KeePass model), not per-entry envelopes.
>
> **Drive layout, post-rebrand** — "one pack per vault under `Rowel/Vaults/`" replaced the
> single `Rowel/vault.swsync`: packs are now `Vaults/<vault-id>.rowel` (same extension as the
> local backup, which is the same bytes) and shares `Shares/<random>.rowelshare`, all named by
> `sync::layout`; the `SWSY` magic bytes are a format tag and stay. The layout changed inside
> the alpha with **no migration**, and the old file is never read again — not by sync, not by
> onboarding. Upgrade path: update the device that holds the vault first (its data is local and
> its first sync pushes it to the new layout), then reset every other device and restore it from
> that new pack. `Rowel/vault.swsync` can then be deleted by hand. The id a run settles on is
> written into the vault's `meta` mid-run, at the one point that is past a fetched, decoded and
> merged pull and short of every push: a failed import never leaves a vault pointed at a pack it
> did not merge, and no pack is ever uploaded under an id the vault does not already answer to.
> The packs folder stays `Rowel/Vaults/` for now and is read as `Rowel/Lockboxes/` too. A client
> from before the lockbox rename that finds no `Vaults/` recreates it and uploads there, so while
> such clients are supported no client may create or rename to `Lockboxes/` — a fresh account under
> the new name would fork the first time one of them synced. The cutover (rename in place, new name
> for fresh accounts) is a later release; finding both folders at once stops the run rather than
> picking a side.
>
> Onboarding's probe lists every live pack it finds and the first run picks which lockbox to
> restore or to archive, so an account holding two installs' primaries never has one chosen
> for it.
>
> **Per-lockbox sync** — every lockbox (a *workspace* in the code: `workspace.rs`,
> `workspaces.json`) now connects its own Drive account and syncs its own
> pack, so `guard_primary` is left covering biometric unlock alone (one keychain slot per
> install). Archive-by-rename went with it (and "start fresh" beside an existing lockbox went
> after it — see *Join, don't fork* below). Shares carry a
> `vaultId` appProperty so each lockbox lists only its own, while the sweep stays account-wide.
> Lockboxes share no credentials and no pack, but they do share the OAuth *grant*: Google
> retires one per account and client, across every lockbox and every device. Disconnecting
> a lockbox therefore deletes only its own token file and never revokes at Google — a
> lockbox cannot tell who else holds a token for the account (other token files are sealed
> under keys it does not have, and other devices are invisible to it), so any revocation it
> attempted would be either too broad or, guessed from local files alone, wrong.
>
> **Restore into a lockbox** — Settings › Lockboxes can now add a lockbox by restoring one
> of the connected account's *other* lockboxes, so a second device reaches every lockbox on the
> account rather than only the one onboarding picked. It reuses onboarding's keyless connect,
> probe and picker outright (`commands::setup::connect_pending`) and mirrors `workspace_create`
> for the rest; a pack that is already a lockbox on the device is refused by lockbox id. The
> open lockbox is asked live; every other one is locked and unreadable, so the registry
> records each lockbox's id as its syncs settle it (`Workspace::vault_id`, not a
> secret — it is the pack's file name in the user's own Drive) and the restore asks that.
> While a restore runs, Settings cannot be closed or moved off the section: the backend holds
> the pending account for its length, so a Cancel or a navigation that appeared to back out
> would have let the restore finish and switch lockboxes behind the user's back.
>
> **Join, don't fork** — the account is the source of truth for which lockboxes exist: every
> device connected to it syncs the lockboxes it holds, and a new pack is minted only into an
> empty account (the first device) or by creating a lockbox on a device that is already
> connected. Connecting a lockbox (Settings › Sync › Connect on a phone that "started fresh",
> say) used to mint a second pack beside the desktop's; every connect now goes through
> onboarding's keyless connect and probe (`sync_connect`, with no fork — a lockbox is given an
> id at creation, so the id says nothing about whether it has synced), and
> `sync_adopt_pending` lists the account's packs with the pending tokens and adopts only an
> *empty* account or one that already holds this lockbox's id. Otherwise it refuses
> (`vaultNotInAccount`), the tokens stay pending, and Settings shows the account's lockboxes
> with the same picker and restore form Settings › Lockboxes uses. The first run's
> conflict screen lost "Start a new vault" for the same reason, and `plan_vault_id` refuses
> to mint beside an existing lockbox as belt and braces. "Import from Google Drive" (the
> `Intent::Import` merge) is gone: a pack is sealed under its own lockbox's KDF salt, so a
> lockbox with a salt of its own could never decode it — the empty-vault hero's restore only
> ever worked for a lockbox restored from that very pack. Joining therefore *is* the restore;
> the lockbox that was open stays beside the restored one.
>
> **Every lockbox on every device** — the second half of the same principle. Each sync run
> lists the account's packs anyway (to settle the lockbox id), so it now reports the ones no
> lockbox on this device holds (`sync::publish_remote_vaults` → `workspaces:remote`,
> registry lockbox ids plus the run's own), and Settings › Lockboxes offers them under "In
> your Google account" with the shared picker and restore form. The restore
> (`workspace_restore_from_account`) is `workspace_restore_from_pending` with the sign-in taken
> out: the open lockbox's own tokens do the download and a copy is sealed under the restored
> key — one account, connected once, in as many lockboxes as the account has packs. In the
> other direction, `workspace_create` on a connected device inherits the account: the tokens
> are sealed under the new key and the lockbox is stamped with a minted id *at creation*, so
> its first sync addresses a pack of its own rather than being refused as a nameless lockbox
> beside the account's others; the pack then shows up on every other device's list after their
> next sync.
>
> **Same password, no prompts** — the moment a password is in hand (a password unlock, a
> first-run restore, a lockbox restore) it is tried against the account's packs this device
> lacks, and the ones it opens become lockboxes beside the open one without switching to them
> (`commands::autojoin`, reporting `workspaces:added` per lockbox and a refreshed
> `workspaces:remote`; the frontend re-probes and shows a passing notice). The password lives
> exactly as long as that blocking-pool task, as a second `Zeroizing` copy, and is never
> written; the task stops starting candidates after a 60-second budget, so that is the budget
> plus one download — see the threat model's unlock section. The task is bound to the account
> it began on: the refreshed tokens are written back to the originating lockbox under the
> generation guard before each install, a disconnect meanwhile ends the task, and the
> "already a lockbox here" refusal is asked again with the step held and once more inside
> the registry write. A pack sealed with a different password stays on offer in Settings ›
> Lockboxes. Biometric unlock has a key and no password, so it cannot do this. Follow-up: pull on foreground and on a timer, since an idle device only syncs
> on unlock and after its own writes.
>
> **Follow-ups (not blockers):** wire `sync::restore` into onboarding ("Restore from Drive"
> on a fresh install); cross-device master-password change — designed and built, see
> `docs/password-change-sync-design.md`; OAuth scope audit (`drive.file`); an explicitly global "Revoke Google
> access" action, the one place the grant is retired (a per-lockbox disconnect deliberately
> never does); **release gate: a production Google OAuth client ID (owner task)**.

Re-enable Google Drive sync on the new SQLite storage engine. Postponed during remediation;
picking it back up now. **The storage groundwork already exists** — do not rebuild it:
`entries` carry `updated_at` / `deleted_at` (LWW + tombstones), and `SqliteStore::export_for_sync`
already yields all records including tombstones. This is the sync *algorithm + Drive transport*,
not a storage change.

**Scope:**
- **Per-row last-writer-wins merge with tombstones** (replaces the legacy whole-vault merge).
  Reconcile local vs remote record sets by `id`, resolving each by the newer `updated_at`;
  a tombstone (`deleted_at` set) wins over an older live edit. Deterministic, no whole-file overwrite.
- **Deterministic Drive-folder handling** — stable app folder discovery/creation, no duplicate
  folders on repeated setup; handle the "folder deleted remotely" case.
- **Token handling** — the Drive OAuth token blob is already encrypted with the session key
  (`VaultKey::cryptor()`); wire the actual sync push/pull around it.

**Prereqs / owner input needed:** a Google OAuth client ID for the app (release gate — not an
agent task). Confirm the encrypted-payload sync unit (do we sync the opaque per-entry AEAD
payloads as-is, re-keyed by the recipient, or a portable envelope?) before building — this is
the 🔴 review point.

**Acceptance:** two installs sharing a vault converge (add/edit/delete on each side reconciles
correctly); a delete on one device propagates as a tombstone, not a resurrection; repeated
setup never creates duplicate Drive folders; sync round-trip covered by tests.

_(Legacy audit refs: T-SYNC-1 / T-SYNC-2, D2–D4, S13.)_

---

## 2. Frontend redesign ✅

> **Done** — the Keyring-inspired redesign landed as the PR stack #268–#276 (+ rescues #277,
> #282) with follow-up refinements through #283: ruled Tailwind v4 token system
> (`src/styles/theme.css`), lucide icons, new brand mark, favicons in the list, card brand
> marks + interactive card face. `data-testid` selectors were preserved throughout.

A visual/UX refresh of the app. Tailwind was considered and deferred during the dependency
modernization — revisit as part of this. The React stack is current (React 19 + zustand +
Vite + bun); this is design/UX, not a framework migration.

**To define before building** (owner input — this is a product/design decision, not purely
technical):
- Design direction / reference — what does "redesigned" look like? (mockups, a reference app,
  or a described aesthetic).
- Tailwind adoption: yes/no. If yes, introduce it alongside the existing styles and migrate
  incrementally (component-by-component), not a big-bang rewrite.
- Scope: full re-skin vs. targeted screens (unlock, entry list, entry detail, settings).

**Constraints to preserve:** the `data-testid` selectors the E2E suite relies on (don't break
`tests/e2e/`); split complex components into folders with an `index.tsx` + logical subcomponents
per the standing React convention; keep theme/accessibility intact.

**Acceptance:** TBD once direction is set — at minimum, E2E smoke suite still green, no
regressions in the core flows.

---

## 3. E2E specs port from legacy 🟡

> Live coverage matrix (legacy floor, planned additions, and what is deliberately not
> e2e-able): [`tests/e2e/COVERAGE.md`](../tests/e2e/COVERAGE.md).

Port the legacy Electron E2E coverage onto the new `tauri-plugin-webdriver` harness landed in
the remediation phase (`tests/e2e/`, mirrors the reticle app; one smoke spec today). The legacy
app had **18 feature specs** under `test/features/` (on `master`) driven by **Spectron** —
Spectron itself is dead (Electron-only, archived 2022), but it was built on WebdriverIO, so the
spec *logic* ports nearly 1:1 (`app.client.$` → global `$`; `setValue`/`click`/`getText` identical).

**Legacy specs to port** (prioritized by value):
- **High:** setup validation (`setup/password`), login CRUD (`logins/create|edit|delete`),
  search (`logins/search`), audit (`audit`), change-password (`settings/change_password`).
- **Medium:** cards (`cards/create`), notes (`notes/create`), vault settings (`settings/vault`).
- **Low / thin:** empty states (`logins/empty`, `tags/empty`), tags filter (`tags/filter`),
  launch scope.

**Porting notes:**
- Map class selectors (`.body .list`) to `data-testid` where the redesign (item 2) will churn
  class names — coordinate ordering with the frontend redesign so specs aren't ported twice.
- `before({storage: 'empty'|'pristine'})` → the new `ROWEL_DB_DIR` temp-dir isolation
  (`pristine` = no vault → Setup; `empty` = vault, no entries).
- **Drop/adjust stale assertions** from the rebrand + disabled sync — e.g. `setup/password`
  asserts old welcome copy mentioning "Import from Gdrive"; several assert sync UI that's off.
- Batch the port (an agent per group), high-value flows first.

**Acceptance:** the ported specs pass in CI (the `e2e` job); each covers the flow its legacy
counterpart did, minus sync assertions until item 1 lands.

**Sequencing note:** ideally after (or interleaved with) the frontend redesign (item 2), so
selectors are ported against the final UI, not the current one.

_(Legacy audit ref: T-CI-2 extended — the smoke spec is done; this is the breadth.)_

---

## Suggested order

Sync (1) and the redesign (2) are done. Only the E2E port (3) remains — and its sequencing
condition is now satisfied: the UI is final, so specs port once, against the shipped design.
Sync assertions the legacy specs dropped can come back too, now that item 1 landed.
