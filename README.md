![Rowel — Password Manager](docs/banner.svg)

## Free Offline-first Password Manager for MacOS, Windows and Linux.

<div align="center">
  
  [![](https://img.shields.io/badge/PayPal-Buy%20me%20a%20Coffee-blue)](https://www.paypal.me/alchaplinsky)
  
  [![Actions Status](https://github.com/fwdai/rowel/workflows/CI/badge.svg)](https://github.com/fwdai/rowel/actions)
  [![Financial Contributors on Open Collective](https://opencollective.com/rowelapp/all/badge.svg?label=financial+contributors)](https://opencollective.com/rowelapp) ![GitHub release (latest SemVer including pre-releases)](https://img.shields.io/github/v/release/fwdai/rowel?include_prereleases&label=Release)
  ![GitHub All Releases](https://img.shields.io/github/downloads/fwdai/rowel/total?label=Downloads)
  [![Encryption](https://img.shields.io/badge/Encryption-AES%20256%20GCM-green.svg)](https://tools.ietf.org/html/rfc5288)
  
</div>

❤️ it? Then ⭐️ it on GitHub or Tweet about it.

## Features

- Store Login/Password credentials
- Credit card Information 
- Secure notes to store sensitive information
- One-click Strong Password Generation
- Time-based One Time Passwords support (TOTP)
- Google Drive Sync (optional)
- Browser autofill through the KeePassXC-Browser extension (see below)
- No data is leaving your computer:
  - Your vault is a locally stored, encrypted SQLite database (SQLCipher); each
    entry's secrets are sealed in an extra application-level AEAD layer
  - Secrets stay encrypted at rest and in memory, decrypted only when you reveal
    or copy them
  - Ability to migrate from one computer to another using backup file or GDrive sync
- There's more to come...

### Browser extension

Rowel speaks the KeePassXC-Browser protocol, so the stock KeePassXC-Browser
extension fills logins from your vault:

1. Install KeePassXC-Browser from the
   [Chrome Web Store](https://chromewebstore.google.com/detail/keepassxc-browser/oboonakemofpalcgghocfoadofidjkkk),
   [Firefox Add-ons](https://addons.mozilla.org/firefox/addon/keepassxc-browser/)
   or [Edge Add-ons](https://microsoftedge.microsoft.com/addons/detail/keepassxcbrowser/pdffhmdngciaglkoonimfcmckehcpafo).
2. In Rowel, open **Settings › Browser extension** and turn it on.
3. Click the extension's icon and **Connect**, then approve the dialog in
   Rowel and give the browser a name.

Passkeys work through it too: a site's "create a passkey" or "sign in with a
passkey" reaches Rowel, which asks you in a dialog naming the site — and, for
a sign-in, the account, with a choice when you have several passkeys there —
before it answers. That needs KeePassXC-Browser 1.9 or newer, with its
passkeys option turned on in the extension's settings.

Because the extension looks for KeePassXC's host, a real KeePassXC install on
the same machine contends for the same browser manifest. Rowel leaves
KeePassXC's in place and Settings shows that browser as a conflict; use one or
the other.

## Screenshots

<img width="1012" alt="Rowel lock screen — vault sealed, master password prompt with Touch ID" src="docs/screenshots/lock-screen.png">
<img width="1012" alt="Rowel credit card view — card list and masked card details" src="docs/screenshots/card-view.png">


## Install

Check the [Latest Releases](https://github.com/fwdai/rowel/releases) page for the
most recent packaged app for MacOS, Windows or Linux.

## Verifying a release

Every release is built in GitHub Actions and ships with supply-chain evidence:

- **SLSA build provenance** — each installer is attested with
  [`actions/attest-build-provenance`](https://github.com/actions/attest-build-provenance)
  (keyless OIDC signing). You can prove an installer was built by this repo's
  workflow, from this source, with the [GitHub CLI](https://cli.github.com):

  ```bash
  gh attestation verify ./Rowel_1.0.0_amd64.AppImage --repo fwdai/rowel
  ```

  (works for the `.dmg`, `.msi`, `-setup.exe`, `.deb`, `.rpm` and `.AppImage`
  assets — point it at whichever you downloaded).

  Releases up to and including `v1.0.0-alpha.6` were built before the project
  moved from `swiftyapp/swifty` to `fwdai/rowel`, so their provenance names the
  old repository. Verify those with `--repo swiftyapp/swifty` instead.

- **CycloneDX SBOM** — every release attaches `rowel-rust.cdx.json` (the full
  Rust dependency graph) and, when available, `rowel-js.cdx.json` (the
  frontend). Feed them to any CycloneDX-aware scanner (e.g. `grype sbom:./rowel-rust.cdx.json`)
  to audit the exact dependencies a build shipped.

- **Update signature** — the auto-updater only installs updates signed with the
  project's minisign key (public key in `src-tauri/tauri.conf.json`); the
  matching private key never leaves CI.

The Rust toolchain (`rust-toolchain.toml`) and the bun version are both pinned,
so builds are reproducible from a fixed toolchain.

## Development

Rowel is built with [Tauri 2](https://v2.tauri.app) (Rust backend + TypeScript/React/Vite frontend).

### Prerequisites

- [Bun](https://bun.sh) 1.3.3 or newer (what CI pins) — the package manager and
  script runner for this repo; `bun.lock` is the committed lockfile
- [Node.js](https://nodejs.org) 22 (what `.nvmrc` and CI pin) — only the E2E
  suite needs it, since WebdriverIO runs under Node
- [Rust](https://rustup.rs) (stable toolchain)
- Platform build dependencies for Tauri — see the
  [Tauri prerequisites guide](https://v2.tauri.app/start/prerequisites/)
  (on Linux: `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libgtk-3-dev`)

### Commands

```bash
git clone git@github.com:fwdai/rowel.git
cd rowel
bun install

bun run tauri:dev     # run the app in development
bun run tauri:build   # produce a signed, packaged build for the current OS

bun run tauri:dev:fresh   # same, against a throwaway data dir (first-run flows)

bun run build         # build the frontend only (tsc + vite)
bun run test          # frontend unit tests (Vitest)
cd src-tauri && cargo test   # backend tests
```

Run the scripts with `bun run <name>`, not `bun <name>`: bare `bun test` starts
Bun's own test runner instead of the Vitest suite this repo is written against.

`tauri:dev:fresh` points `ROWEL_DB_DIR` at a **new** temp directory each run, so
the app finds no vault there and starts at the setup/restore screen every time.
Your real dev vault is left untouched — `bun run tauri:dev` goes back to it.

Setting `ROWEL_DB_DIR` yourself overrides that and keeps the same directory
across runs, which is how you set a throwaway vault up once and then relaunch
into its unlock screen. Nothing here ever deletes a data directory; to start
that one over, remove it yourself. Debug builds only: a release build ignores
the variable and always uses the OS app-data directory.

### Building the iOS AutoFill extension

The iOS app embeds an AutoFill credential provider, the `rowel_autofill`
target in `src-tauri/gen/apple/project.yml`. Its Swift is in
`src-tauri/gen/apple/Sources/autofill`; the vault it reads is the
`rowel-autofill` crate (`src-tauri/crates/autofill`), which Xcode builds for
the SDK being targeted in a pre-build step, like the app's own Rust. Swift
reaches the crate through UniFFI bindings committed under
`Sources/autofill/generated`: after changing anything the crate exports, run
`src-tauri/crates/autofill/bindgen.sh` and commit what it writes.

## Configuration

### Google Drive sync (optional)

Release builds ship with a working Google OAuth client, so sync works out of
the box. This section only applies when building from source, where you supply
a client of your own. Desktop and iOS need one client each, because Google will
not let a Desktop client redirect to a mobile app.

#### Desktop

1. In the [Google Cloud Console](https://console.cloud.google.com) create an
   OAuth 2.0 Client ID of type **Desktop app**.
2. Enable the **Google Drive API** for the project.
3. The app requests the `https://www.googleapis.com/auth/drive.file` scope and
   listens on the loopback redirect URI `http://127.0.0.1:4567/auth/callback`.
4. Provide the credentials as environment/build variables when running or
   building:

   ```bash
   export GOOGLE_OAUTH_CLIENT_ID=your-desktop-client-id.apps.googleusercontent.com
   export GOOGLE_OAUTH_CLIENT_SECRET=your-desktop-client-secret   # optional
   ```

   They are read at runtime (`std::env::var`) and, if absent, fall back to the
   value baked in at compile time (`option_env!`). The build succeeds without
   them — sync simply reports "Google OAuth client not configured" until a
   client id is supplied.

#### iOS

An iOS client is a *public* client: it has **no secret**, PKCE is mandatory, and
its redirect URI is its own client id reversed. So the one thing to configure is
that URL scheme, and the client id is derived from it.

1. In the same project create a second OAuth 2.0 Client ID, of type **iOS**,
   with the **iOS** bundle id from `src-tauri/tauri.ios.conf.json`
   (`app.rowel.mobile` — not the desktop one in `tauri.conf.json`).
2. Take the client id it gives you — `123456-abcdef.apps.googleusercontent.com`
   — and reverse it into a scheme: `com.googleusercontent.apps.123456-abcdef`.
   (Google shows this as the "iOS URL scheme" on the credential page.)
3. Add the scheme to `src-tauri/tauri.ios.conf.json` (there is no `plugins`
   block there until you do — a placeholder scheme is not a legal URL scheme
   and App Store Connect rejects the upload over it, so the file ships without
   one):

   ```json
   "plugins": {
     "deep-link": {
       "mobile": [{ "scheme": ["com.googleusercontent.apps.123456-abcdef"] }]
     }
   }
   ```

   This is the committed source of truth for iOS, and it is safe to commit —
   an iOS client id is public by design. The Tauri CLI registers the scheme as
   `CFBundleURLTypes` in the generated `Info.plist` during `tauri ios build`
   (via `tauri-plugin-deep-link`'s build script), and the app derives both the
   client id and the redirect URI `com.googleusercontent.apps.<id>:/oauth2redirect`
   from it at runtime. Re-run `bun run tauri ios init` if the Xcode project is
   out of date.
4. Add the **redirect URI** `com.googleusercontent.apps.123456-abcdef:/oauth2redirect`
   to the client in the console.

Release builds do not need the block committed: `bun run release:ios` and the
`Release iOS` workflow derive the scheme from `GOOGLE_OAUTH_IOS_CLIENT_ID` (in
`.env` / the repository secret) and pass it through `tauri ios build --config`,
which replaces a committed one. That is the only way in — exporting
`GOOGLE_OAUTH_CLIENT_ID` does nothing on iOS, because `tauri ios build` compiles
inside xcodebuild with a replaced environment that carries only the CLI's own
`TAURI_*` variables, so `option_env!` never sees it. `GOOGLE_OAUTH_CLIENT_SECRET`
is ignored on iOS and must never be shipped in a mobile binary.

### Auto-update signing

Release builds are signed for `tauri-plugin-updater`. The public key lives in
`src-tauri/tauri.conf.json`; the matching **private key is never committed** and
is provided to CI via the `TAURI_SIGNING_PRIVATE_KEY` (and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) secrets. Generate a keypair with
`bun run tauri signer generate -w ~/.rowel/updater.key`.

## Security

Rowel is offline-first: your vault is an encrypted SQLite database (SQLCipher)
on your own device, with each entry's secrets sealed in an additional
application-level AEAD layer, and there is no backend that holds your secrets.
See [`SECURITY.md`](SECURITY.md) for
how to report a vulnerability, and [`docs/threat-model.md`](docs/threat-model.md)
for what Rowel does and does not defend against.

## Contributors

### Code Contributors

This project exists thanks to all the people who contribute. [[Contribute](CONTRIBUTING.md)].
<a href="https://github.com/fwdai/rowel/graphs/contributors"><img src="https://opencollective.com/rowelapp/contributors.svg?width=890&button=false" /></a>

### Financial Contributors

Become a financial contributor and help us sustain our community. [[Contribute](https://opencollective.com/rowelapp/contribute)]

#### Individuals

<a href="https://opencollective.com/rowelapp"><img src="https://opencollective.com/rowelapp/individuals.svg?width=890"></a>

#### Organizations

Support this project with your organization. Your logo will show up here with a link to your website. [[Contribute](https://opencollective.com/rowelapp/contribute)]

<a href="https://opencollective.com/rowelapp/organization/0/website"><img src="https://opencollective.com/rowelapp/organization/0/avatar.svg"></a>
<a href="https://opencollective.com/rowelapp/organization/1/website"><img src="https://opencollective.com/rowelapp/organization/1/avatar.svg"></a>
<a href="https://opencollective.com/rowelapp/organization/2/website"><img src="https://opencollective.com/rowelapp/organization/2/avatar.svg"></a>
<a href="https://opencollective.com/rowelapp/organization/3/website"><img src="https://opencollective.com/rowelapp/organization/3/avatar.svg"></a>
<a href="https://opencollective.com/rowelapp/organization/4/website"><img src="https://opencollective.com/rowelapp/organization/4/avatar.svg"></a>
<a href="https://opencollective.com/rowelapp/organization/5/website"><img src="https://opencollective.com/rowelapp/organization/5/avatar.svg"></a>
<a href="https://opencollective.com/rowelapp/organization/6/website"><img src="https://opencollective.com/rowelapp/organization/6/avatar.svg"></a>
<a href="https://opencollective.com/rowelapp/organization/7/website"><img src="https://opencollective.com/rowelapp/organization/7/avatar.svg"></a>
<a href="https://opencollective.com/rowelapp/organization/8/website"><img src="https://opencollective.com/rowelapp/organization/8/avatar.svg"></a>
<a href="https://opencollective.com/rowelapp/organization/9/website"><img src="https://opencollective.com/rowelapp/organization/9/avatar.svg"></a>

## License

GNU/GPL Version 3
