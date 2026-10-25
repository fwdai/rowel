# Rowel browser extension

Rowel's browser extension lives in [`extension/`](../extension). It is a fork
of [KeePassXC-Browser](https://github.com/keepassxreboot/keepassxc-browser)
(GPL-3.0, like Rowel), vendored as a **git subtree** so upstream releases can be
pulled in. The desktop app already speaks KeePassXC-Browser's native messaging
protocol (`src-tauri/crates/broker`, hosted by `src-tauri/src/browser/`), so
the fork keeps the protocol, crypto,
form detection and content scripts exactly as upstream has them. It changes:

- branding: name, icons, user-facing copy ("Rowel", "vault");
- identity: its own native messaging host name, Firefox add-on ID and a pinned
  Chromium key;
- KeePassXC-only surfaces: the KeePassXC update check and links to KeePassXC's
  docs, downloads and store listings;
- paths: the source is under `extension/rowel/` (upstream's
  `keepassxc-browser/`) with the content script and icons named `rowel*`. The
  JavaScript identifiers inside (`kpxc*`, the `keepass` object, `kpxcEvent`)
  and the protocol strings are upstream's. `extension/upstream-map.json` lists
  the renames (see Syncing with upstream).

This covers Chrome, Chromium, Edge and Firefox. Safari on macOS runs the same
extension from a Safari build that ships inside Rowel.app, with no native
messaging host: see [safari-extension.md](safari-extension.md).

## Fixed identifiers

The desktop host allows exactly these (`src-tauri/src/browser/manifest.rs`).
Changing any of them breaks the connection until the host changes too.

| What | Value | Where in `extension/` |
| --- | --- | --- |
| Native messaging host | `app.rowel.browser` | `rowel/background/client.js` (`nativeHostName`) |
| Firefox add-on ID | `browser@rowel.app` | `dist/manifest_firefox.json` (`browser_specific_settings.gecko.id`) |
| Chromium extension ID (unpacked) | `dimghkhcdfaokfingegmgbnpnpcoeofj` | derived from `key` in `rowel/manifest.json` and `dist/manifest_chromium.json` |
| Chrome Web Store ID | `aajfpjaphnegnekpggjnocmgbhkabeke` | assigned by the store; not in the source |

The `key` is only the public half. It makes an unpacked Chromium build get the
same ID on every machine. The Chrome Web Store and Edge Add-ons won't take an
upload that carries a `key`, so the build strips it from the store zip. A
listing then gets an ID from the store, and that ID has to be added to the
host's `chromium_origins` before the listed extension can connect. The Chrome
Web Store's is in; Edge Add-ons will assign another when the extension is
listed there, which goes in the same list.

## Versioning

The extension has its own version, starting at `0.1.0`, independent of the
app's. `0.1.0` is KeePassXC-Browser **1.10.4** plus the Rowel changes. The
upstream base is recorded as `keepassxcBrowserVersion` in
`extension/package.json`. To bump, edit `version` (and `version_name`) in
`extension/rowel/manifest.json`, which the build stamps into both
browsers' manifests, and `version` in `extension/package.json`.

## Commands

The extension keeps upstream's tooling: npm, its own `package-lock.json`,
ESLint 9 and Playwright. None of it is in the root `package.json`, and the
root ESLint, TypeScript, Vitest and Tailwind configs don't look inside
`extension/`. Root scripts wrap it:

| Script | Does |
| --- | --- |
| `bun run extension:install` | `npm ci` in `extension/` |
| `bun run extension:lint` | checks the paths follow `upstream-map.json`, the strings are rebranded and the theme is generated, then runs upstream's ESLint |
| `bun run extension:test` | the Safari port's `node --test` suite, then upstream's Playwright tests, in Chromium and Firefox. Run `npx playwright install chromium firefox` in `extension/` once first |
| `bun run extension:build` | writes `extension/build/` (below), Safari included, then checks its IDs and host name |
| `bun run extension:strings` | re-applies the Rowel copy to every `_locales/*/messages.json` |
| `bun run extension:theme` | regenerates `css/rowel-tokens.css` from the app's design tokens (see Theme) |
| `bun run extension:icons` | regenerates every icon from Rowel's artwork |
| `bun run extension:sync` | after a `git subtree pull`: moves and rewrites whatever still carries an upstream path per `extension/upstream-map.json` (`scripts/extension-sync.mjs`), then runs `extension:strings`, `extension:icons` and `extension:theme` |

`extension:build` runs `extension/rowel-build.js`. Upstream's `build.js` is
left untouched and unused, because it pulls translations from Transifex and
zips with `tar -a`, which only produces a zip with BSD tar. It needs `zip` on
`PATH`, which macOS and Linux have. The output:

```
extension/build/
  chromium/                    unpacked, with the pinned key
  firefox/                     unpacked
  rowel_0.1.0_chromium.zip     Chrome Web Store / Edge Add-ons (no key)
  rowel_0.1.0_firefox.zip      addons.mozilla.org
  safari/                      unpacked, for the Safari app extension
```

`safari/` comes from `extension/safari/build.mjs` (`npm run build:safari`),
which runs after `rowel-build.js` because that one clears `build/`. The Safari
manifest is derived from the Firefox one; the Safari app extension that
packages it is built by `scripts/build-safari-extension.sh`
([safari-extension.md](safari-extension.md)).

## Theme

The extension's UI is upstream's Bootstrap markup, restyled by
`rowel/css/rowel.css`, a hand-written stylesheet whose every value is a token
from `rowel/css/rowel-tokens.css`, which `scripts/extension-theme.mjs`
generates from the desktop app's tokens in `src/styles/theme.css` (the light
`:root` and dark `:root[data-theme='dark']` palettes and the `@theme inline`
fonts). The tokens file mirrors `colors.css`'s four theme blocks and holds
only values: the `--c-*` tokens, the rgb triplets Bootstrap composes with an
alpha and the check/switch glyphs whose fill is baked into an SVG. `rowel.css`
`@import`s it, then, once, points upstream's `--kpxc-*` variables and
Bootstrap's `--bs-*` variables at the tokens (backgrounds →
detail/pane/list/field, borders → line2, links and primary buttons → the
accent, success/warning/danger/info → good/warn/bad/lockfield), overrides the
colours upstream hardcodes in its own CSS and
restyles the in-page surfaces (the credential dropdown as the app's menu, the
save banner as a card-like bar, notifications as its toast, the in-page
buttons as filled accent or ghost). It is loaded after `colors.css` and
Bootstrap in every page and injected after them into every in-page shadow
root, so upstream's stylesheets stay untouched, except `css/define.css` (the
field-picker chips), which is edited in place. Those surfaces sit inside
arbitrary websites: host rules cannot reach into a shadow root, but inherited
properties (font, colour, letter-spacing, text-transform) do, so each surface's
root forces them with `!important`, as upstream does. Change the tokens in
`theme.css`, never in `rowel-tokens.css`, and run `bun run extension:theme`;
CI fails if the file is stale. Rules go in `rowel.css`.

## Agent mode

For a browser profile an AI agent drives (Claude in Chrome, Comet, a Browser
Use script). The person turns it on with the **Agent mode** switch in the
popup; the popup then shows one screen ("Agent mode is on", **Turn off**) and
nothing else, and the options page stays reachable through the browser's
extension menu. It is the `agentMode` setting (`defaultSettings` in
`background/page.js`, default `false`, stored with the other settings). Spec:
`spec/05-delivery-modes.md` §2, `spec/04-protocol.md` "Extension-side
actions". While it is on:

- **Every fill is a grant.** `get-logins` carries `agentMode: "true"`; the
  host asks the person in Rowel's dialog (or applies a standing rule) and
  replies with the one admitted entry and a `grantId`. `background/grants.js`
  keeps a map *origin → `{ grantId, expiresAt }`* in `browser.storage.session`
  (15 minutes from receipt, the grant's lifetime on the host; cleared on
  `database-locked`) and sends `grantId` instead of `agentMode` while the
  origin has a live one, so the TOTP step and the second page of a
  username-first sign-in reuse it without a second dialog. `get-totp` carries
  `agentMode: "true"` always and the same `grantId` when the origin has one;
  the host refuses an agent-mode code without a covering grant rather than
  serving it the way a person's click would be. The origin is the requesting
  document's, for both: `get-logins` keys the grant by the URL the content
  script sent (its own document), and `get-totp` looks it up by the sender's
  frame URL, so a cross-origin login frame is its own origin — it asks its
  own grant, the dialog names the frame's origin, and its TOTP step finds
  that grant. One bounded retry: a `GrantExpired` answer to a
  remembered grant forgets it and asks afresh; a `GrantPending` with
  `retryAfterMs` is waited for once. The refusals (codes 40–46 in `kpErrors`,
  `client.js`) become the extension's notification: `GrantDenied` "Rowel did
  not allow this fill", `GrantPending` "Rowel is busy with another request…",
  `RateLimited` "Too many requests; wait a minute.", `AgentsDisabled` "Turn on
  Agents in Rowel › Settings › Browser & agents". The notifications carry no
  credential and no URL.
- **Grants an agent asked for.** The host pushes a clear `rowel-signal` (no
  content, beside `database-locked`; the Safari shim's `isSignal` knows it)
  and the extension answers with a sealed `get-signals`
  (`grants.onSignal()`, also run once after every reconnect so a signal
  queued while the port was down is delivered). A `fill-grant { grantId,
  origin, item, autoSubmit, expiresAtMs }` is remembered in the same map with
  the host's expiry and the origin is marked agent-driven
  (`browser.storage.session["rowelAgentGrants"]`). The host tells every
  profile and the agent drives one, so only the profile with a tab at that
  origin — now, or when one finishes loading there later
  (`tabs.onUpdated`) — acts under it: records it as one agent mode is on
  for here (`rowelAgentGrants`, which holds only those, never a grant
  merely held for another profile, since it is this map emptying that
  turns agent mode back off), turns agent mode on through the popup's own
  path if it was off (`rowelAgentModeByGrant` remembers that a grant did
  it) and tells the tab to redetect its fields, which reloads its settings
  and retrieves with the grant; the others keep the grant in memory
  (`rowelGrants`, marked `agent`) and change nothing. A login served under
  the grant marks it `served` in that profile: signed out where it was
  signed in, the sign-out at the end (and on a lock) is only that
  profile's. A `grant-ended { grantId, origins }` is session end: every cookie of
  each origin's registrable domain is deleted — in every cookie store the
  browser lists (`cookies.getAllCookieStores()`, then `getAll({ domain,
  storeId })` per store, since one call sees one store), partitioned
  cookies asked for by name (`partitionKey: {}`) and, on Firefox, every
  first-party domain (`firstPartyDomain: null`, which first-party isolation
  requires), a variant a browser rejects being a case it does not have,
  and each `cookies.remove` naming the cookie's own store, partition and
  first-party domain; Safari has no cookies API, so it is best-effort
  there — the grant is forgotten, "The agent is done: Rowel signed
  you out of the site in this browser" is shown, and agent mode goes back off
  once no agent grant is left, only if a grant turned it on. An origin a newer
  grant holds is left whole, session included: it is that grant's to end. A
  lock (`database-locked`) ends every grant with no `grant-ended` to follow,
  so `grants.onLocked()` signs out of every origin a grant was held for, the
  person's own agent-mode grants included. Signals, the lock and an
  exposure run through one queue in `grants.js`, each to its end before the
  next, so a lock's cleanup never overlaps the fill-grant an unlock brings
  right after it. Agent mode the person turned on is never touched.
- **No in-page UI.** `content/agent-mode.js` (`kpxcAgentMode.isOn()`) gates
  the login-form icons, the username/TOTP autocomplete, the save/update
  banner, the custom-fields banner and the login popup. The admitted entry is
  filled as `autoFillSingleEntry` would (`kpxcAgentMode.fill()` in place of
  `kpxc.prepareCredentials`), TOTP included, and submitted only if upstream's
  `autoSubmit` setting is on. Agent mode retrieves on its own, with or without
  the `autoRetrieveCredentials` setting (there is no icon to click). The
  context-menu entries and keyboard shortcuts keep working; they are the
  person's.
- **Exposure check.** After a fill, and after a submit (on the next page load
  of the same tab, which is a fresh content script: the values are handed over
  through `browser.storage.session` keyed by tab id, read once and dropped
  within 60 s), the content script scans `document.body.innerText` and the
  `value` of every `input:not([type=password])` and `textarea` for the exact
  password and code it filled (in the text, the code counts only when no other
  digit touches it, so a phone number or order id is not a hit), now and once
  more a second later. A hit empties
  the matching fields, sends `report-exposure { grantId }` (the host ends the
  grant and logs it), forgets the grant and shows "Rowel ended the grant: the
  page showed the password". **Clear on failure:** when the same URL loads
  again with a password field present, every field holding the filled
  password is emptied before field detection runs. Limits: each frame scans
  its own document (cross-origin frames are opaque; the after-submit check
  runs in the top frame), shadow roots are not traversed, text drawn as pixels
  is not seen, and an echo later than the re-scan without a navigation is
  missed.

Tests: `tests/agent-mode.html` + `agent-mode-tests.js` run the content scripts
behind a stub WebExtension API (`tests/browser-stub.js`) for the hidden UI,
the fill, the scan and clear, the echo fixture and clear-on-failure;
`tests/grants.spec.ts` covers the grant map and the handoff as pure functions.
The grant request itself needs the running app.

## Developing against a debug Rowel

1. Run the app: `bun run tauri:dev`. In **Settings › Browser extension**, turn
   it on. Rowel then registers the `app.rowel.browser` native messaging host
   for each browser it finds, pointing at the running debug binary (see
   `src-tauri/src/browser/manifest.rs`). Rowel is the native host itself: the
   browser launches the same executable.
2. Load the extension.
   - **Chrome, Chromium, Edge:** open `chrome://extensions` (or
     `edge://extensions`), turn on Developer mode, click **Load unpacked** and
     pick `extension/rowel/`. That is the live source, so reloading
     the extension picks up your edits. `extension/build/chromium/` works too.
     Either way the card should show ID `dimghkhcdfaokfingegmgbnpnpcoeofj`.
   - **Firefox:** run `bun run extension:build`, open
     `about:debugging#/runtime/this-firefox`, click **Load Temporary Add-on**
     and pick `extension/build/firefox/manifest.json`. The source folder holds
     the Chromium (MV3 service worker) manifest, so it won't load in Firefox
     as-is. `npm run debug:firefox` swaps the Firefox manifest into the source
     folder if you want live edits; put it back with `npm run debug:chromium`
     before committing. Temporary add-ons are removed when Firefox quits.
3. Click the toolbar icon, then **Connect**, and approve the dialog in Rowel.

## Troubleshooting

- **"Cannot connect to Rowel" / red cross on the icon.** Check that Rowel is
  running with the browser extension turned on, then look for the host
  manifest `app.rowel.browser.json`:
  - macOS: `~/Library/Application Support/Google/Chrome/NativeMessagingHosts/`
    (Chrome), `~/Library/Application Support/Mozilla/NativeMessagingHosts/`
    (Firefox)
  - Linux: `~/.config/google-chrome/NativeMessagingHosts/`,
    `~/.config/chromium/NativeMessagingHosts/`,
    `~/.mozilla/native-messaging-hosts/`
  - Windows: the registry key
    `HKCU\Software\Google\Chrome\NativeMessagingHosts\app.rowel.browser` (or the
    `Microsoft\Edge` / `Mozilla` equivalent) names the manifest file.

  Its `path` must be the Rowel executable you are running.
- **The host manifest is there but Chrome still refuses.** Its
  `allowed_origins` must contain the extension's ID. An extension loaded from
  a folder without the `key`, or installed from a store other than the Chrome
  Web Store, has an ID the host doesn't list.
- **Firefox refuses.** `allowed_extensions` must contain `browser@rowel.app`.
- The background page's console (`chrome://extensions` → **service worker**,
  or `about:debugging` → **Inspect**) logs native messaging errors, tagged
  `Rowel`. The extension's **Settings › Advanced settings › Debug logging**
  adds more.

## Syncing with upstream

Upstream tags releases as `1.10.4` etc. From the repo root, on a clean branch:

```sh
git subtree pull --prefix=extension https://github.com/keepassxreboot/keepassxc-browser.git <tag> --squash
```

This lands as a squash commit plus a merge commit. Then:

```sh
bun run extension:sync
```

Upstream's tree is `keepassxc-browser/`; ours is `rowel/`, with the content
script `content/rowel.js` and the icons `icons/rowel*` in place of
`keepassxc-browser.js` and `keepassxc*`. Git's directory rename detection
follows that on the merge, so upstream's edits and new files land under
`rowel/`. `extension:sync` catches what didn't: `scripts/extension-sync.mjs`
applies `extension/upstream-map.json`, `git mv`ing anything that exists at an
old path (a new icon upstream added as `keepassxc_256x256.png`, say, or a
file the merge left under `keepassxc-browser/`) and rewriting the old names
in the manifests, pages, stylesheets, tests and build scripts. It is
idempotent, and `--check` (run by `extension:lint` and CI) fails if anything
still carries an old name. The rest of `extension:sync` re-applies the
strings, icons and theme.

Conflicts can only happen in files the fork changed. That is the whole rebase
surface, and each file has a set way to resolve it:

| Files (under `extension/`) | Resolve by |
| --- | --- |
| `rowel/_locales/*/messages.json` | Take upstream's (`git checkout --theirs -- <files>`), then `bun run extension:strings`. If upstream reworded an English string listed in `OVERRIDES` in `scripts/extension-strings.mjs`, review the override. The Rowel-only keys (`agentMode*`, `errorMessageGrant*` and the other codes 40–46) come from `ROWEL_MESSAGES` in the same script and are re-appended to `en` |
| `rowel/icons/**` (PNG + SVG) | Take upstream's, then `bun run extension:icons`. The toolbar state badges come from upstream's SVGs, so new ones are picked up |
| `rowel/manifest.json`, `dist/manifest_chromium.json`, `dist/manifest_firefox.json` | Take upstream's changes and keep ours for `name`, `version`/`version_name`, `author`, `homepage_url`, `key`, `default_title`, `css/rowel.css` and `css/rowel-tokens.css` in `web_accessible_resources`, `content/agent-mode.js` after `content/fill.js` in `content_scripts`, and on Firefox `browser_specific_settings` (add-on ID, `data_collection_permissions`), `background/grants.js` after `background/event.js` in `background.scripts`, plus the dropped `https://api.github.com/` permission |
| `rowel/background/background_service.js` | Keep `'grants.js'` after `'event.js'` in `importScripts` |
| `rowel/background/client.js` | Keep `nativeHostName = 'app.rowel.browser'`, the Rowel codes 40–46 (`GRANT_DENIED` … `AGENTS_DISABLED`) in `kpErrors` with their `errorMessages` entries, and in `onNativeMessage` the `if (response.action === kpActions.DATABASE_LOCKED) { grants.onLocked(); }` block before the lock/unlock one and the `if (response.action === kpActions.ROWEL_SIGNAL) { grants.onSignal(); return; }` block after it |
| `rowel/background/keepass.js` | Keep the Rowel lines: `ROWEL_SIGNAL: 'rowel-signal'` in `kpActions`, `grants.sendGetLogins(tab, messageData, nonce)` in place of `keepassClient.sendMessage(...)` in `retrieveCredentials`, `Object.assign(messageData, await grants.totpFields(tab, sender))` in `getTotp`, and `await grants.onSignal()` after `testAssociation` in `reconnect` |
| `rowel/background/page.js` | Keep `passkeys: true` and `agentMode: false` in `defaultSettings`. Upstream ships passkeys off; Rowel's host always serves them (it reports 2.7.7, the version upstream gates the feature on), so a new install has them on. A setting already stored keeps its value |
| `rowel/common/global.js` | Keep `EXTENSION_NAME` and the notification title as `Rowel` |
| `rowel/content/rowel.js` | Keep the four Rowel lines: `await kpxcAgentMode.onPageLoad()` after `kpxc.isFirefox = …` in `initContentScript`; the `if (kpxcAgentMode.isOn()) { return kpxcAgentMode.fill(); }` block after the empty-credentials check in `prepareCredentials`; `\|\| kpxcAgentMode.isOn()` in `initLoginPopup`'s first `if`; `(kpxc.settings.autoRetrieveCredentials \|\| kpxcAgentMode.isOn())` in `retrieveCredentials`'s condition, so agent mode retrieves on its own with the setting off |
| `rowel/content/fill.js` | Keep `await kpxcAgentMode.afterFill(combination, selectedCredentials)` right before `performAutoSubmit` in `fillInCredentials`, and `await kpxcAgentMode.noteTotp(val)` after the empty-credentials check in `setTOTPValue` |
| `rowel/content/icon-handler.js`, `content/totp-field.js` | Keep the `kpxcAgentMode.isOn()` gate: `\|\| kpxcAgentMode.isOn()` in `kpxcIcons.initIcons`'s first `if`, and the `if (kpxcAgentMode.isOn()) { return; }` at the top of `kpxcTOTPIcons.newIcon` |
| `rowel/popups/popup.html`, `popups/popup_login.html`, `popups/popup_httpauth.html` | Take upstream's markup and re-apply: the Rowel links; the `<link rel="stylesheet" href="../css/rowel.css">` after the last stylesheet, with `popup.css` moved to just before it (after Bootstrap, so popup.css needs no `!important`); the `.rowel-brand` header row (mark + "Rowel") as the first child of `#settings`; `rowel-ghost` in place of `btn-success`/`btn-warning`/`btn-danger` on the toolbar buttons and on the secondary actions (`#username-only-button`, `#allow-iframe-button`, `#btn-dismiss`); in `popup.html` the `<script defer src="agent-mode.js">` after `popup.js` and the `#rowel-agent-mode` and `#rowel-agent-mode-on` blocks as the last children of `.container`. Every ID and `data-i18n` key stays upstream's |
| `rowel/popups/popup.css` | Keep ours: it is a rewrite (Rowel tokens, no `!important` except over Bootstrap's `.bg-*`). Diff upstream's for new selectors (states, IDs) and add them |
| `rowel/options/options.html` | Take upstream's markup and re-apply the Rowel links, the hidden Updates card, the removed store links, the debug-info labels, the `rowel.css` link, and the rail sidebar block: the `.rowel-brand` `h1`, `nav-link` without `text-light`/`text-white`, the toggler without `navbar-dark`, the footer credit without `small text-muted` |
| `rowel/options/options.css` | Take upstream's and re-apply the Rowel rules: page-title `h2`, `span.menu-item`, `footer`, `.card`/`.card-header`, the `.sidebar` block through `.navbar-toggler-icon`, and the three media queries at the end (mobile sheet, 240px rail from lg) |
| `rowel/options/getting_started.html`, `getting_started.css`, `options/shortcuts.html`, `shortcuts.css` | Take upstream's and re-apply the Rowel links, the `rowel.css` link, the `.rowel-brand` row (above the welcome `h2`; in place of the logo `<img>` in `.conf-title`), `rowel-ghost` on the Reset buttons, and the Rowel card rules in the two CSS files |
| `rowel/content/autocomplete.js`, `content/banner.js`, `content/custom-fields-banner.js`, `content/ui.js` | Take upstream's, then keep the `createStylesheet('css/rowel.css')` line and its `shadowRoot.append(rowelStyleSheet)` after the other sheets, in `ui.js` the `initColorTheme(notification)` call so notifications follow the chosen theme like the other surfaces, and the agent-mode gate: `if (kpxcAgentMode.isOn()) { return; }` at the top of `kpxcBanner.create`, `\|\| kpxcAgentMode.isOn()` in the first `if` of `kpxcCustomLoginFieldsBanner.create` |
| `eslint.config.mjs` | Keep the `grants` and `kpxcAgentMode` globals |
| `tests/tests.html` | Keep `<script defer src="../content/agent-mode.js">` after `fill.js`, as in the manifests |
| `rowel/css/define.css` | Keep ours (the field-picker chips are restyled in place, since `rowel.css` only overrides the other in-page sheets). If upstream added a class, add it with the same `color-mix` wash and 2px edge |
| `package.json`, `package-lock.json` | Keep our `name`, `version`, `description`, `main`, `build` script and URLs, and take upstream's dependencies. Then run `npm install` in `extension/` to regenerate the lock, and bump `keepassxcBrowserVersion` |
| `README.md` | Keep the fork note at the top |

Rowel-only files never conflict: `rowel-build.js`, `upstream-map.json`,
`rowel/css/rowel.css`, the generated `rowel/css/rowel-tokens.css`, the agent
mode files `rowel/background/grants.js`, `rowel/content/agent-mode.js`,
`rowel/popups/agent-mode.js` and their tests `tests/agent-mode.html`,
`tests/agent-mode-tests.js`, `tests/browser-stub.js`,
`tests/agent-mode.spec.ts`, `tests/grants.spec.ts`, and
everything outside `extension/`
(`scripts/extension-sync.mjs`, the other `scripts/extension-*.mjs`,
`scripts/check-extension-build.mjs`, this doc).

After merging:

1. Bump the extension's version (see Versioning).
2. `bun run extension:sync`, if it wasn't run before resolving conflicts, then
   `bun run extension:lint && bun run extension:test && bun run extension:build`.
3. Search `extension/` for `org.keepassxc.keepassxc_browser`, `keepassxc.org`
   and `KeePassXC` in HTML and non-locale JS. New user-facing strings upstream
   added outside `_locales` won't be caught by `extension:strings`.
4. Read upstream's `CHANGELOG` for protocol changes (`keepassxc-protocol.md`).
   New actions need host support in `src-tauri/crates/broker/src/actions.rs`
   before the extension relies on them.

## Not rebranded

Left as upstream has it, on purpose or because it isn't worth a fork change:

- Translations other than English say "Rowel" wherever they said
  KeePassXC(-Browser), but still use their own word for "database" rather than
  "vault". A few sentences name Rowel twice where upstream named both the
  extension and the app.
- The **Request Global Auto-Type** context menu entry and shortcut (a KeePassXC
  feature).
- The contributor and supporter credits on the About page, and upstream's
  `CHANGELOG`, `keepassxc-protocol.md`, `.github/` (inert below the repo root)
  and `dev-resources/` artwork.
- Internal identifiers: the `kpxc*` functions and classes, the `keepass`
  object, `kpxcEvent`, setting names and protocol strings are upstream's,
  while the paths (`rowel/`, `content/rowel.js`, `icons/rowel*`) are Rowel's.
  Renaming the identifiers would put every upstream diff in conflict for no
  user-visible gain.
- The KeePassXC update check is hidden, not removed. It already defaults to
  "never", and the Firefox build no longer requests `https://api.github.com/`.
