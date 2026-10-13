# E2E coverage matrix

The live tracker for the E2E port (item 3 of [`docs/v1-next.md`](../../docs/v1-next.md)).
Four PRs: **PR 1** is the foundation (per-spec isolation, helpers, `data-testid` hooks);
**PRs 2–4** are spec batches that can land in parallel once PR 1 is in.

Every spec opens with an explicit `resetPristine()` or `resetEmpty(password)` — the suite
runs one app process against one data dir, so nothing may depend on file order.

---

## Legacy floor — the 18 Spectron specs on `master`

| # | Legacy spec (`test/features/…`) | Lands in | New spec | Notes |
|---|---|---|---|---|
| 1 | `setup/launch.spec.js` | PR 2 | `setup.spec.ts` | Choice screen renders; Setup vs Restore. |
| 2 | `setup/password.spec.js` | PR 2 | `setup.spec.ts` | Weak-password gate, mismatch error. Drop the old welcome copy assertion (rebrand). |
| 3 | `authentication/password.spec.js` | PR 2 | `unlock.spec.ts` | Wrong password → `unlock-error`; correct password → `main-view`. |
| 4 | `launch/scope.spec.js` | PR 2 | `unlock.spec.ts` | Boot lands on unlock when a vault exists (`resetEmpty`). |
| 5 | `logins/create.spec.js` | PR 2 | `logins-crud.spec.ts` | |
| 6 | `logins/edit.spec.js` | PR 2 | `logins-crud.spec.ts` | Includes the discard guard (see additions). Editing happens in the detail pane. |
| 7 | `logins/delete.spec.js` | PR 2 | `logins-crud.spec.ts` | Two-press confirm, not a dialog (see additions). |
| 8 | `logins/password.spec.js` | PR 2 | `logins-crud.spec.ts` | Reveal/hide + copy of the password field. |
| 9 | `logins/search.spec.js` | PR 2 | `search-and-scopes.spec.ts` | One search field for the whole app, in the list column: `search-input` / `search-clear-button`, focused by ⌘F, ⏎ selects the first row left standing. |
| 10 | `logins/empty.spec.js` | PR 3 | `list.spec.ts` | Empty-vault hero: `empty-vault` in the detail pane, `create-first-entry-button` inside it, nothing in the list column. |
| 11 | `cards/create.spec.js` | PR 3 | `cards.spec.ts` | Plus brand mark + interactive face (see additions). |
| 12 | `notes/create.spec.js` | PR 3 | `notes.spec.ts` | |
| 13 | `tags/filter.spec.js` | dropped | — | The tag chip row was removed with the All Items redesign; tags stay on entries and are matched by the search field (`search-and-scopes.spec.ts`). |
| 14 | `tags/empty.spec.js` | dropped | — | No chip row to assert on any more. |
| 15 | `audit/index.spec.js` | PR 4 | `audit.spec.ts` | Groups are **Weak / Reused / Breached** now, not the legacy buckets. |
| 16 | `settings/change_password.spec.js` | PR 4 | `change-password.spec.ts` | Reached via `settings-nav-security` → the row's "Change…" control. |
| 17 | `settings/password.spec.js` | PR 4 | `settings.spec.ts` | Generator *defaults* card in Settings › Security; `generator.spec.ts` covers the ⌘G dialog itself. |
| 18 | `settings/vault.spec.js` | PR 4 | `settings.spec.ts` | Minus the sync assertions the legacy spec made against the old Drive UI. |

## Planned additions — flows the legacy suite never had

| Flow | Lands in | Hook |
|---|---|---|
| In-place editing, no form sheet | item 8 | `entry-sheet` is the editing container **inside the detail pane** (not a sliding sheet any more); it appears on `edit-entry-button` (a row of the `more-actions-button` menu, via the `startEdit` helper) or a kind-picker choice and disappears on save/discard, which stays the "write landed" signal. The read cluster (`primary-action-button`, `more-actions-button`) is unmounted while it is up, and the list column is dimmed and `inert` (`list-column`) — so nothing over there is clickable or keyboard-reachable mid-draft. |
| Discard-changes guard on a dirty edit | PR 2 | `cancel-entry-button` pressed twice ("Discard changes?"); Escape runs the same two-press guard. A clean draft closes on the first press. |
| Card expiry as one MM/YY box | item 8 | `input[name="expiry"]` replaces `name="month"` + `name="year"`; typing `0429` saves month `04`, year `29` and reads back as `entry-value-expires` = `04/29`. `helpers/entries.ts` still takes the pair and composes the box. |
| Type-aware editor fields | item 8 | Each row is one borderless input keyed by `name`, so the selectors are unchanged: `title` (the pane's heading input), `website`, `username`, `password`, `email`, `otp`, `number`, `cvc`, `pin`, `name`, plus `textarea[name="note"]`. The card number groups itself as it is typed, so it reads back as `4111 1111 1111 1111`. |
| Archive confirm | PR 2 | `delete-entry-button` → `delete-entry-confirm` inside `more-actions-button` (the testids keep the backend op's name; the copy reads "Archive" / "Archive entry?") |
| Kinds as places under All Items | PR 2 | In All Items the title (`list-title`) is a button that opens the scope menu: `scope-option-all` / `scope-option-<kind>` for every kind, `aria-checked` on the open one, each with a `…-count`. Picking a kind retitles the column ("Secure notes"); in every other view the title is plain text. A mixed list is sectioned by kind under sticky captions (`group-<kind>`), which in All Items are buttons into the kind; a list of one kind, a kind view and a search are flat. |
| All Items in the rail; Vault Health via Settings | PR 2 | `view-items` in the rail; the health tile is parked, so the view is reached through `settings-button` → `settings-nav-audit` → `settings-open-health`. The pane title is `list-title`. |
| Favorites and Archive views in the rail | item 14 | `view-favorites` / `view-archive`, in rail order All Items · Favorites · Archive · Tags · spacer · `lock-vault-button` · Settings (the generator has no tile: `generator-button` is a row in the Add picker, and ⌘G / the palette open the same dialog). Each view retitles `list-title` ("Favorites", "Archive"); search applies in all three item views, and a kind is opened from All Items alone. |
| Starring an entry | item 14 | `favorite-toggle` in the detail header (absent on a tombstone, which cannot be starred). Under Recent, starred rows are **pinned** above the rest — `list.spec.ts` proves it by touching another entry afterwards, since starring restamps `updatedAt` and recency alone would otherwise explain the order. Alphabetical is deliberately unpinned. Starred rows carry `entry-item-star`. |
| Archive: archive → restore → purge | item 14 | `archive.spec.ts`. An archived entry leaves All Items (`empty-vault`) and appears under `view-archive`, its row stamped "Deleted …" rather than the usual bare relative time. An archived entry is read-only: `restore-entry-button` and `purge-entry-button` are the only actions, and `edit-entry-button` / `more-actions-button` / `primary-action-button` are all absent (`reveal_entry` does not serve deleted rows). `restore-entry-button` puts it back in All Items, editable again; `purge-entry-button` → `purge-entry-confirm` ("Delete forever?") removes it from both views. `empty-archive` is the empty state. |
| A tombstone is un-editable by any route | item 14 | The read-only-ness is enforced in `editEntry` itself, not per-accelerator, so ⌘E over an archived row is a no-op and no `entry-sheet` appears — covered as a unit test (`archive.test.tsx`) since it is a store guard rather than a rendering. ⌘⏎ on an archived row still calls `reveal_entry`, which refuses it; `copySecret` swallows the rejection, so nothing reaches the clipboard and nothing throws. |
| Sort control | PR 3 | `sort-menu` → `sort-option-recent` / `sort-option-alpha`, assert `entry-item-title` order |
| Card face: reveal + copy | PR 3 | `card-reveal-button`, `entry-value-number` / `-expires` / `-cvc` / `-pin` |
| Card brand mark | PR 3 | brand slug derived at save time; assert per-network glyph |
| Empty states, one system for both panes | PR 3 | One variant at a time, each with its own hook: `empty-vault` (hero, detail pane, holds `create-first-entry-button`), `empty-kind` + `empty-kind-add` and `empty-search` + `empty-search-clear` / `empty-search-widen` (compact, list column), `empty-select` (quiet, detail pane), `empty-health`, plus `empty-favorites` and `empty-archive` (item 14). The list column and the detail pane never both show a hero, so any of the whole-view heroes means an empty column. |
| "Add a secret" kind picker | PR 3 | `add-entry-button` (or `create-first-entry-button`) → `add-secret-modal` → `add-kind-login` / `add-kind-card` / `add-kind-note`; `modal-close` or Escape dismisses. Every create flow goes through it, so `helpers/entries.ts` owns the two clicks. |
| Keyboard path through the list | PR 2 | ⌘F → type → ↓↓ → ⏎ → ⌘E in `search-and-scopes.spec.ts`, no pointer. Rows are `role="option"` with `aria-selected` inside the column's `role="listbox"` scroller, so the selection is read off the row rather than a class; the arrows leave the caret in `search-input`. |
| Command palette | PR 4 | `command-palette`, `command-palette-input`, `palette-item`. Commands only — a query matching a vault entry must leave `palette-item` empty; entries are `search-and-scopes.spec.ts`'s job. |
| Password generator | PR 4 | `generator-mode-random` / `-memorable`, `generator-amount`, `generator-regenerate`, `generator-output`, `generator-use-button` |
| Change master password | PR 4 | `change-password-submit`, `change-password-error`, `change-password-success` |
| Settings shell: nav + section titles | PR 4 | `settings-modal`, `settings-nav-<sync\|security\|audit\|import\|language>` (`aria-current="page"` on the active item), `app-version` in the pinned footer, `modal-close` |
| Session preferences | PR 4 | `settings-autolock-<secs>` and `settings-clipboard-<ms>` segments, asserted through `autolockSecs` / `clipboardTimeoutMs` in the persisted settings (`readSettings()`). |
| Generator defaults | PR 4 | `settings-generator-symbols`, asserted through the persisted `generator` record. `settings-generator-length` and `-numbers` exist but nothing asserts them yet. |
| Breach monitoring switch | PR 4 | `settings-breach-toggle`; weak/reused are informational rows with no control, so the section holds exactly one `role="switch"` |
| Import tiles | PR 4 | `import-tile-<1password\|lastpass\|enpass\|bitwarden\|keepass>` in the source grid plus `import-tile-other`, whose menu holds `import-tile-<chrome\|cxf\|csv>` (mounted only while open; Escape closes it); `import-tile-swftx` on the restore-backup row, `import-dropzone`; export lives in the same section behind an Import/Export switch with format cards |
| Theme and date format | PR 4 | `settings-theme-light` / `-dark` (asserted on `<html data-theme>`), `settings-date-format-trigger` then `settings-date-format-<pattern>` (asserted through the persisted `dateFormat`). `settings-theme-system` resolves against the OS, so the suite leaves it alone. |
| Sync indicator reports a local-only vault | PR 4 | `sync-indicator`; the chip is mounted in every state, so with no provider configured `sync-indicator.spec.ts` asserts `data-tone="local"` rather than an absence. The tone, not the label — the label is localised. A connected reading is not e2e-able (see below). |
| Copy toast | PR 4 | `copy-toast` — raised only by a copy whose control is gone (the generator's "Use & copy"), mounted only while `ui.copied` holds (2s), so it is absent at rest; assert *visibility* either way. Controls that stay on screen confirm in place with `copied-mark` |

---

## Deliberately not e2e-able

These are not gaps in the suite; the coverage lives elsewhere and belongs there.

| Area | Why not e2e | Where it is covered |
|---|---|---|
| Drive sync loop | Needs a live Google account + OAuth consent; a WebDriver run cannot hold credentials. | Rust tests around `sync::engine` against a fake `Remote`, plus the pack/restore round-trip tests. |
| Biometric unlock | Gated by the OS (Touch ID prompt); no WebDriver surface, and the secure store is machine-bound. | `secure_store` / `biometrics` are behind a platform trait; unit-tested through it. |
| `.swftx` import / export, backup restore, third-party imports | Every import tile and the portable export open a **native** file dialog outside the webview — the driver cannot reach it. The drop zone needs a real OS drag. | `import::` and `store::` Rust tests cover parse, reseal-on-import and the export round-trip; `settings.test.tsx` drives the tile → password → import flow against mocked commands. |
| Updater | Talks to the release endpoint and stages a signed bundle. | `api/autoUpdate.test.ts` against a mocked plugin. |
| HIBP breach check | Network call to the k-anonymity range API. | `hibp` Rust unit tests; the audit spec runs with the breach check off. |
| Site favicons | Fetches from each entry's own host. | E2E asserts only the **glyph fallback**; fetching/caching is covered in `favicon` Rust tests. |

---

## Harness reference

- `helpers/reset.ts` — `resetPristine()`, `resetEmpty(password)` (the backend wipe takes
  `settings.json` with it; both then re-seed `locale = "en-US"`, so every spec selects on
  English labels without pinning it itself)
- `helpers/settings.ts` — `readSettings()`, `setSettings(patch)` over the dev-only
  `window.__e2eSettings` bridge; preferences are Rust's, not localStorage's
- `helpers/vault.ts` — `setupVault(password)`, `unlock(password)`, `lockVault()`
- `helpers/entries.ts` — `createLogin`, `createCard`, `createNote`, `entryItems()`,
  `visibleTitles()`, `expectTitles(titles)`, `openEntry(title)`, `toggleFavorite()`
  (each `create*` opens the kind picker from `add-entry-button`, presses its `add-kind-<kind>`
  tile, fills the pane editor by field `name`, and saves — `createCard` composes the one
  `expiry` box from the `month` / `year` pair its caller still passes. Rows carry no
  per-entry testid, so `openEntry` matches on title text)
- `helpers/app.ts` — `waitFor(testid)`, `waitForAppReady()`
- `helpers/keys.ts` — `chord(key)` (Meta on macOS, Control on CI), `pressEnter()`, `pressArrowDown()`

Reset goes through `window.__e2eReset` (installed only when `import.meta.env.DEV`) onto the
`e2e_reset` Tauri command, which is compiled out of release builds and additionally refuses
to run unless `ROWEL_E2E=1` **and** `ROWEL_DB_DIR` are set — so it can only ever erase the
suite's own temp dir. See `src-tauri/src/commands/e2e.rs`.
