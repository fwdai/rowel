# Rowel browser extension

Rowel's browser extension lives in [`extension/`](../extension). It is a fork
of [KeePassXC-Browser](https://github.com/keepassxreboot/keepassxc-browser)
(GPL-3.0, like Rowel), vendored as a **git subtree** so upstream releases can be
pulled in. The desktop app already speaks KeePassXC-Browser's native messaging
protocol (`src-tauri/src/browser/`), so the fork keeps the protocol, crypto,
form detection and content scripts exactly as upstream has them. It changes:

- branding: name, icons, user-facing copy ("Rowel", "vault");
- identity: its own native messaging host name, Firefox add-on ID and a pinned
  Chromium key;
- KeePassXC-only surfaces: the KeePassXC update check and links to KeePassXC's
  docs, downloads and store listings.

This covers Chrome, Chromium, Edge and Firefox. Safari on macOS runs the same
extension from a Safari build that ships inside Rowel.app, with no native
messaging host: see [safari-extension.md](safari-extension.md).

## Fixed identifiers

The desktop host allows exactly these (`src-tauri/src/browser/manifest.rs`).
Changing any of them breaks the connection until the host changes too.

| What | Value | Where in `extension/` |
| --- | --- | --- |
| Native messaging host | `app.rowel.browser` | `keepassxc-browser/background/client.js` (`nativeHostName`) |
| Firefox add-on ID | `browser@rowel.app` | `dist/manifest_firefox.json` (`browser_specific_settings.gecko.id`) |
| Chromium extension ID | `dimghkhcdfaokfingegmgbnpnpcoeofj` | derived from `key` in `keepassxc-browser/manifest.json` and `dist/manifest_chromium.json` |

The `key` is only the public half. It makes an unpacked Chromium build get the
same ID on every machine. The Chrome Web Store and Edge Add-ons won't take an
upload that carries a `key`, so the build strips it from the store zip. A
listing then gets an ID from the store, and that ID has to be added to the
host's `chromium_origins` before the listed extension can connect. (To keep
the pinned ID on the Chrome Web Store instead, the first upload can include
the matching private key as `key.pem` at the zip root. Whoever created the key
has it; it is not in this repo.)

## Versioning

The extension has its own version, starting at `0.1.0`, independent of the
app's. `0.1.0` is KeePassXC-Browser **1.10.4** plus the Rowel changes. The
upstream base is recorded as `keepassxcBrowserVersion` in
`extension/package.json`. To bump, edit `version` (and `version_name`) in
`extension/keepassxc-browser/manifest.json`, which the build stamps into both
browsers' manifests, and `version` in `extension/package.json`.

## Commands

The extension keeps upstream's tooling: npm, its own `package-lock.json`,
ESLint 9 and Playwright. None of it is in the root `package.json`, and the
root ESLint, TypeScript, Vitest and Tailwind configs don't look inside
`extension/`. Root scripts wrap it:

| Script | Does |
| --- | --- |
| `bun run extension:install` | `npm ci` in `extension/` |
| `bun run extension:lint` | checks the strings are rebranded, then runs upstream's ESLint |
| `bun run extension:test` | upstream's Playwright tests, in Chromium and Firefox. Run `npx playwright install chromium firefox` in `extension/` once first |
| `bun run extension:build` | writes `extension/build/` (below), Safari included, then checks its IDs and host name |
| `bun run extension:strings` | re-applies the Rowel copy to every `_locales/*/messages.json` |
| `bun run extension:icons` | regenerates every icon from Rowel's artwork |

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

## Developing against a debug Rowel

1. Run the app: `bun run tauri:dev`. In **Settings › Browser extension**, turn
   it on. Rowel then registers the `app.rowel.browser` native messaging host
   for each browser it finds, pointing at the running debug binary (see
   `src-tauri/src/browser/manifest.rs`). Rowel is the native host itself: the
   browser launches the same executable.
2. Load the extension.
   - **Chrome, Chromium, Edge:** open `chrome://extensions` (or
     `edge://extensions`), turn on Developer mode, click **Load unpacked** and
     pick `extension/keepassxc-browser/`. That is the live source, so reloading
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
  a folder without the `key`, or installed from a store, has a different ID.
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

This lands as a squash commit plus a merge commit. Conflicts can only happen
in files the fork changed. That is the whole rebase surface, and each file has
a set way to resolve it:

| Files (under `extension/`) | Resolve by |
| --- | --- |
| `keepassxc-browser/_locales/*/messages.json` | Take upstream's (`git checkout --theirs -- <files>`), then `bun run extension:strings`. If upstream reworded an English string listed in `OVERRIDES` in `scripts/extension-strings.mjs`, review the override |
| `keepassxc-browser/icons/**` (PNG + SVG) | Take upstream's, then `bun run extension:icons`. The toolbar state badges come from upstream's SVGs, so new ones are picked up |
| `keepassxc-browser/manifest.json`, `dist/manifest_chromium.json`, `dist/manifest_firefox.json` | Take upstream's changes and keep ours for `name`, `version`/`version_name`, `author`, `homepage_url`, `key`, `default_title`, and on Firefox `browser_specific_settings` (add-on ID, `data_collection_permissions`) plus the dropped `https://api.github.com/` permission |
| `keepassxc-browser/background/client.js` | Keep `nativeHostName = 'app.rowel.browser'` |
| `keepassxc-browser/common/global.js` | Keep `EXTENSION_NAME` and the notification title as `Rowel` |
| `keepassxc-browser/options/options.html`, `options/getting_started.html`, `popups/popup.html`, `popups/popup_login.html`, `popups/popup_httpauth.html` | Take upstream's markup and re-apply the Rowel links, the options sidebar title and footer, the hidden Updates card, the removed store links and the debug-info labels |
| `package.json`, `package-lock.json` | Keep our `name`, `version`, `description`, `main`, `build` script and URLs, and take upstream's dependencies. Then run `npm install` in `extension/` to regenerate the lock, and bump `keepassxcBrowserVersion` |
| `README.md` | Keep the fork note at the top |

Rowel-only files never conflict: `rowel-build.js`, and everything outside
`extension/` (`scripts/extension-*.mjs`, `scripts/check-extension-build.mjs`,
this doc).

After merging:

1. Bump the extension's version (see Versioning).
2. `bun run extension:lint && bun run extension:test && bun run extension:build`.
3. Search `extension/` for `org.keepassxc.keepassxc_browser`, `keepassxc.org`
   and `KeePassXC` in HTML and non-locale JS. New user-facing strings upstream
   added outside `_locales` won't be caught by `extension:strings`.
4. Read upstream's `CHANGELOG` for protocol changes (`keepassxc-protocol.md`).
   New actions need host support in `src-tauri/src/browser/` before the
   extension relies on them.

## Not rebranded

Left as upstream has it, on purpose or because it isn't worth a fork change:

- Translations other than English say "Rowel" wherever they said
  KeePassXC(-Browser), but still use their own word for "database" rather than
  "vault". A few sentences name Rowel twice where upstream named both the
  extension and the app.
- The default group names `KeePassXC-Browser Passwords` / `KeePassXC-Browser
  Passkeys` (the options placeholders and `content/banner.js`). They are sent to
  the host as group names, so renaming them is a protocol decision, not a copy
  change.
- The **Request Global Auto-Type** context menu entry and shortcut (a KeePassXC
  feature).
- The contributor and supporter credits on the About page, and upstream's
  `CHANGELOG`, `keepassxc-protocol.md`, `.github/` (inert below the repo root)
  and `dev-resources/` artwork.
- Internal identifiers: file names such as `icons/keepassxc_*.png`, function
  and setting names, and protocol strings.
- The KeePassXC update check is hidden, not removed. It already defaults to
  "never", and the Firefox build no longer requests `https://api.github.com/`.
