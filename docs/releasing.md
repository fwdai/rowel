# Releasing Rowel (desktop)

Rowel ships for macOS (signed + notarized universal build), Windows (NSIS
installer) and Linux (.deb, .rpm, AppImage) through GitHub Releases, with a
silent in-app auto-updater. Every app carries the `rowel` command line tool
coding agents connect through; the release also offers that tool on its own,
and the browser extension's packages. This doc covers the one-time setup and
the per-release procedure. iOS is a separate workflow: see
[releasing-ios.md](releasing-ios.md).

> **Before the next release: merge PR D.** The command line tool still links
> `rowel-server` (`crates/server`, `rowel admin server`, `rowel serve
> --server`), which is leaving this repository for the private
> `fwdai/rowel-server`. Every binary this workflow builds would ship it, and
> the standalone archives would publish it. Do not run *Release* until that
> removal is on `main`.

## Overview

```
bump versions → run Release manually from Actions → CI builds all three platforms
             → DRAFT GitHub release (below)
             → you review, write the notes and publish → installed apps update on next launch
             → submit the extension packages to the stores by hand
```

What the draft holds:

| Asset | From |
| --- | --- |
| `Rowel_<version>_universal.dmg`, `Rowel_<version>_x64-setup.exe`, `Rowel_<version>_amd64.deb`, `Rowel-<version>-1.x86_64.rpm`, `Rowel_<version>_amd64.AppImage`, and their version-free copies | the app, each with the command line tool inside as a sidecar (`rowel-cli`), which Settings › General › Command line tool installs as `rowel` ([cli.md](cli.md#installing)) |
| `*.app.tar.gz`, `*.sig`, `latest.json` | the updater |
| `rowel-<version>-universal-apple-darwin.zip` | the command line tool on its own: the sidecar out of the signed app, notarized |
| `rowel-<version>-x86_64-pc-windows-msvc.zip` | the same, the exe the installer carries |
| `rowel-<version>-x86_64-unknown-linux-gnu.tar.gz`, `rowel-<version>-aarch64-unknown-linux-gnu.tar.gz` | the same, built on Ubuntu 22.04 for a glibc 2.35 floor |
| `rowel_<extension version>_chromium.zip`, `rowel_<extension version>_firefox.zip` | the browser extension, for the Chrome Web Store, Edge Add-ons and addons.mozilla.org (Safari's is inside the macOS app) |
| `SHA256SUMS` | the checksums of the command line archives and the extension zips |
| `rowel-rust.cdx.json`, `rowel-js.cdx.json` | the SBOMs |

- **Workflow:** `.github/workflows/release.yml`: one matrix job per platform
  (`release`), then `cli` for the two Linux archives and `checksums` for
  `SHA256SUMS`, which wait for it because it creates the draft.
- **macOS:** `universal-apple-darwin` (one binary for Apple Silicon + Intel),
  signed with the Developer ID Application cert (team `UFBL3F444A`), notarized
  and stapled by `tauri-apps/tauri-action`. The command line archive is
  notarized separately with `notarytool`; a bare binary cannot be stapled, so
  Gatekeeper checks it online.
- **Windows:** NSIS `-setup.exe` only, not code-signed (no certificate), and
  neither is the command line `rowel.exe`. The MSI bundler is skipped while
  versions carry a pre-release suffix such as `1.0.0-alpha.1`; WiX only
  accepts numeric pre-release identifiers.
- **Linux:** `.deb`, `.rpm` and `.AppImage`, unsigned.
- **Version-free copies:** every bundler filename carries the version
  (`Rowel_1.0.2_universal.dmg`), so each job also uploads a copy of its own
  installers under a fixed name — `Rowel_universal.dmg`, `Rowel_x64-setup.exe`,
  `Rowel_amd64.deb`, `Rowel_x86_64.rpm`, `Rowel_amd64.AppImage` — which makes
  `releases/latest/download/<name>` a permanent download link. Same bytes, same
  signatures, attested under both names. README.md and rowel.app link to these,
  so **the names are public API**: renaming one breaks those links. The
  `latestAssets` entry in the workflow's matrix is the mapping, and a job fails
  if its glob stops matching. The command line archives' names are public API
  too: `docs/serve.md` builds its download URL from them.
- **Update artifacts:** `createUpdaterArtifacts: true` in `tauri.conf.json`
  emits the platform-specific updater bundle plus a `.sig` made with the Rowel
  minisign key. `tauri-action` merges them into one `latest.json`, which the app
  fetches from `releases/latest/download/latest.json`.
- **Provenance + SBOM:** every installer, command line archive and extension
  zip gets a SLSA build-provenance attestation, and the Linux job attaches
  CycloneDX SBOMs to the release.

## One-time setup

### 1. Updater signing keypair

A minisign keypair signs every update artifact; the app verifies it with the
public key embedded in `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`).
The public key is committed. The private key lives outside the repo, for
example at `~/.tauri/rowel.key`. To regenerate:

```sh
bun run tauri signer generate -w ~/.tauri/rowel.key
```

Then put the new `~/.tauri/rowel.key.pub` contents into
`plugins.updater.pubkey`. **If you lose the private key, existing installs can
no longer auto-update** and need a fresh manual install.

### 2. GitHub repository secrets

Set these under **Settings → Secrets and variables → Actions**. The Apple and
Google names predate the Tauri port and are kept as-is; `release.yml` maps them
onto the env names the Tauri bundler expects.

| Secret | How to produce it |
| --- | --- |
| `CERTIFICATES_P12` | Export the **"Developer ID Application: Oleksandr Chaplinsky (UFBL3F444A)"** certificate *with its private key* from Keychain Access as a `.p12`, then `base64 -i cert.p12 \| pbcopy`. It must be this certificate: the bundler compares the `.p12` against the hard-coded `APPLE_SIGNING_IDENTITY` and aborts on a mismatch. An "Apple Development" or "Apple Distribution" cert cannot be notarized for direct download. |
| `CERTIFICATES_P12_PASSWORD` | The password you set when exporting the `.p12`. |
| `APPLE_ID` | Apple ID email of the developer account. |
| `APPLE_ID_PASSWORD` | An **app-specific password** (appleid.apple.com → Sign-In and Security → App-Specific Passwords). Not your account password. |
| `APPLE_TEAM_ID` | `UFBL3F444A`. |
| `TAURI_SIGNING_PRIVATE_KEY` | Contents of `~/.tauri/rowel.key`. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for that key (empty if generated without one). |
| `GOOGLE_OAUTH_CLIENT_ID` | Desktop-app OAuth client id from Google Cloud Console, baked in at compile time for Google Drive sync (`src-tauri/src/sync/auth.rs`). Unset → Drive sync reports "not configured". |
| `GOOGLE_OAUTH_CLIENT_SECRET` | The matching client secret. |
| `DROPBOX_APP_KEY` | App key of the Dropbox **App folder** app (App Console → your app → Settings), baked in at compile time for Dropbox sync (`src-tauri/src/sync/providers/dropbox/oauth.rs`). Public client with PKCE: there is no secret to set. Register `http://127.0.0.1:53682/auth/callback`, `…:53683/…` and `…:53684/…` as redirect URIs. Unset → Dropbox sync reports "not configured". |

`APPLE_SIGNING_IDENTITY` is not a secret (it is the certificate's display
name) and is written directly in `release.yml`. If the certificate is ever
reissued under a different name, update it there and in `.env.example`.

### 3. Update endpoint

The repo is public, so the updater reads straight from GitHub Releases:

```json
"endpoints": ["https://github.com/fwdai/rowel/releases/latest/download/latest.json"]
```

`/releases/latest` only ever points at a **published, non-draft, non-prerelease**
release, so the updater sees a version only once you publish it. Integrity is
enforced by the minisign signature the app verifies against the embedded
pubkey.

## The macOS provisioning profile

`src-tauri/Entitlements.plist` requests `keychain-access-groups`, which the
data-protection keychain needs before it will accept a biometry-protected item
(`secure_store::GateMode::Protected`). That entitlement is **provisioning-profile
gated**: macOS checks it against a profile embedded in the bundle, and if no
profile grants it the kernel refuses to spawn the app. The process dies on
SIGKILL before `main` — no crash report — while codesign, notarization and
`spctl` all still report success. This is exactly how 1.0.0-alpha.1 and
1.0.0-alpha.2 shipped unlaunchable, and why both were withdrawn.

So the entitlement and the profile are a matched pair. `src-tauri/embedded.provisionprofile`
is committed to the repo (a Developer ID profile is not a secret — it carries
public certificates, the team id, and an ~18-year expiry), and
`bundle.macOS.files` in `tauri.conf.json` copies it to `Contents/embedded.provisionprofile`
before signing.

### Creating or renewing it

The profile can only be made in the Apple Developer portal — it cannot be
generated from CI or from the signing certificate alone.

The current profile — App ID `app.rowel.desktop` (`HP35R6KD98`), profile
"Rowel Desktop Developer ID" (`4G56YBHLB4`) — expires **2044-09-11**, so this is
a once-a-decade chore. It grants `keychain-access-groups = ['UFBL3F444A.*']`,
which covers any group under the team prefix.

1. **Register the App ID.** [Identifiers](https://developer.apple.com/account/resources/identifiers/list)
   → **+** → *App IDs* → *App*. Bundle ID **explicit**, `app.rowel.desktop`,
   matching `identifier` in `tauri.conf.json` exactly.
2. **Create the profile.** [Profiles](https://developer.apple.com/account/resources/profiles/list)
   → **+** → under *Distribution* pick **Developer ID** — not Development, not
   Mac App Store. Select the App ID and the *Developer ID Application*
   certificate, and download the `.provisionprofile`.

   You do not need to tick a "Keychain Sharing" capability on the App ID.
   Developer ID profiles grant `TEAMID.*` for keychain access groups by
   default, and the capability is not one the App Store Connect API can set.
3. **Commit it** as `src-tauri/embedded.provisionprofile`.
4. **Verify** before building anything:

   ```sh
   bun scripts/check-macos-entitlements.mjs
   ```

   It fails if the profile is missing, expired, not a Developer ID profile, or
   does not grant every group the entitlements ask for. The release workflow
   runs it before the build, and smoke-tests that the signed app actually
   launches afterwards.

Renewing is step 2 onwards — the App ID persists. If the team id ever changes,
update the group in `Entitlements.plist` too.

### iCloud sync: the App ID capability and the CloudKit schema

`Entitlements.plist` also asks for `com.apple.developer.icloud-services`
(`CloudKit`) and `com.apple.developer.icloud-container-identifiers`
(`iCloud.app.rowel`), which iCloud sync runs on
(`src-tauri/src/sync/providers/icloud`). Both are profile-gated exactly like
`keychain-access-groups`, so the profile has to grant them — and a profile only
does once the App ID has the capability. The check script fails the build until
then, with "profile does not grant …", which is the intended outcome: a build
that asked for them without a grant would be SIGKILLed at launch.

One-time, in the Developer portal, before regenerating the profile:

1. **Create the container.** [Identifiers](https://developer.apple.com/account/resources/identifiers/list)
   → **+** → *iCloud Containers* → `iCloud.app.rowel`. One container for both
   platforms: the iOS app (`app.rowel.mobile`) syncs to the same one.
2. **Enable iCloud on both App IDs.** `app.rowel.desktop` and `app.rowel.mobile`
   → *iCloud* → tick *CloudKit* → *Configure* → select `iCloud.app.rowel`.
3. **Regenerate the Developer ID profile** (step 2 of the list above: the
   capability change invalidates the existing profile), commit it, and run
   `bun scripts/check-macos-entitlements.mjs`. It should now report the two
   iCloud keys among the granted ones.
4. **Deploy the schema.** In the [CloudKit Console](https://icloud.developer.apple.com/)
   for `iCloud.app.rowel`, Development environment, record type `VaultEntry`
   with fields `pack` (Asset) and `size` (Int64), and an index on
   `recordName` marked *Queryable* (the listing is a `TRUEPREDICATE` query on
   the type and CloudKit refuses it without that index). Then *Deploy Schema
   Changes…* to Production. Builds signed for distribution talk to Production;
   nothing synced from Xcode is visible to them, and a type that was never
   deployed makes every run fail with "did not find record type".

Until all four are done, a signed macOS build does not launch at all if the
entitlements are in and the profile is not, and a build without the
entitlements simply does not offer iCloud: the app reads its own signature at
startup (`icloud::supported`) and lists only the providers it can connect in
`app_status`. `tauri dev` is unsigned and never offers it.

The profile does **not** grant `com.apple.security.application-groups`, and
does not need to: the app and its Safari extension share
`UFBL3F444A.app.rowel.desktop`, a macOS-style group (team id, dot, name) that
macOS grants on the signature's team id alone. The check script accepts
exactly that form without a profile grant, and refuses a `group.` id, which
macOS 15+ gates behind a profile. The Safari extension's own entitlements
(`src-tauri/gen/safari`) are checked too, against no profile at all. See
[safari-extension.md](safari-extension.md).

This can also be driven through the App Store Connect API
(`POST /v1/bundleIds`, then `POST /v1/profiles` with
`profileType: MAC_APP_DIRECT` and the Developer ID certificate ids), which is
how the current profile was made.

## Per-release procedure

1. **Bump the versions.**
   - The app, in all three files (keep them identical):
     - `src-tauri/tauri.conf.json` → `version` (this is what the release tag
       and name are derived from, via `v__VERSION__`, and what the command
       line archives are named after)
     - `package.json` → `version`
     - `src-tauri/Cargo.toml` → `version` (then `cargo update -p rowel
       --manifest-path src-tauri/Cargo.toml` so `Cargo.lock` follows)

     The workspace crates (`crates/*`, the command line tool among them) keep
     `0.1.0`: they are never released on their own, and the tool ships under
     the app's version.
   - The browser extension, when it changed since the last release: `version`
     and `version_name` in `extension/rowel/manifest.json`, its one home
     ([browser-extension.md](browser-extension.md#versioning)). A store
     refuses an upload whose version it has already seen.
2. Merge that to `main`. CI's supply-chain job must be green: it fails when a
   `@tauri-apps/*` npm package and its Rust crate are on different major.minor
   releases, which is exactly the check `tauri build` would otherwise fail on
   release day (`scripts/check-tauri-versions.mjs`).
3. Trigger the build by hand: GitHub → Actions → *Release* → *Run workflow*
   (only offered for the default branch). Nothing releases automatically — not
   on merge, not on a pushed tag — so a release only ever happens when you ask
   for one. Shipping iOS in the same version is a separate manual run
   ([releasing-ios.md](releasing-ios.md)) that must come **after** this one:
   this workflow creates the draft release, and the iOS run only attaches its
   IPA to a draft that already exists.
4. Wait for every job: the three platform jobs, then the two Linux `cli` jobs,
   then `checksums`. Together they fill one **draft** release
   `Rowel v<version>` with the assets in the table above. The Linux job
   replaces the placeholder body with GitHub's auto-generated release notes.
5. **Smoke-test the draft's assets** on at least one machine per platform:
   install the app, then Settings › General › Command line tool › Install,
   and `rowel status` in a new terminal; unpack a command line archive and run
   `rowel --help`; check one archive against `SHA256SUMS`.
6. **Write the release notes.** The generated list is per pull request; put a
   short summary for users above it. While the agent features are new, say:
   - what they are and that they are off by default (Settings › Browser &
     agents › Agents, and a lockbox's *Agents can use items in this lockbox*),
     linking [agents.md](agents.md);
   - that coding agents connect through the bundled `rowel` command, installed
     from Settings › General › Command line tool, and that MCP configs should
     name it by the full path the row shows;
   - that the browser extension's agent mode needs the extension version in
     this release (and the store versions once they are approved);
   - that the command line tool is also attached on its own, for a box
     without the app ([serve.md](serve.md#installing-the-command-line-tool)),
     with `SHA256SUMS`;
   - what `env` mode does not protect: the approved command receives the
     value ([agents.md](agents.md#what-is-and-is-not-protected)).
7. Review the draft and **Publish** it. Once published, installed apps pick it
   up on next launch, and the `releases/latest/download/` links on README.md
   and rowel.app serve this version's installers.
8. **Submit the extension**, when its version changed. The release's zips are
   the uploads; nothing in CI talks to the stores.
   - **Chrome Web Store:** the developer dashboard → the Rowel item →
     *Package* → *Upload new package* → `rowel_<version>_chromium.zip`, then
     *Submit for review*. The zip has no `key`, which the store refuses; the
     listing has its own ID, which the host already allows. Edge Add-ons
     takes the same zip in Partner Center, but its listing's ID has to be in
     the host's `chromium_origins` before that extension can connect
     ([browser-extension.md](browser-extension.md)).
   - **Firefox (addons.mozilla.org):** the Developer Hub → *Upload New
     Version* → `rowel_<version>_firefox.zip`. The build copies the files,
     stamps the manifest and zips; nothing is compiled or minified by it. If
     a reviewer asks for source, give them `extension/` at the release's tag
     and the build steps in browser-extension.md.
   - **Safari:** nothing to submit. The extension is inside the macOS app and
     updates with it.

## Local signed build (macOS)

To produce a signed + notarized universal build on your Mac without CI:

```sh
cp .env.example .env   # then fill in the values
./scripts/release-local.sh
```

Artifacts land in `src-tauri/target/universal-apple-darwin/release/bundle/`.
The Developer ID certificate must be in your login keychain; the script signs
with `APPLE_SIGNING_IDENTITY` from `.env` and does not need the `.p12`.

Every macOS bundle carries the Safari extension in `Contents/PlugIns`, built
and signed before bundling by `scripts/build-safari-extension.sh`
(`beforeBundleCommand` in `tauri.macos.conf.json`). That needs Xcode, and
`xcodegen` (`brew install xcodegen`) whenever `src-tauri/gen/safari/project.yml`
changed; see [safari-extension.md](safari-extension.md).

For an unsigned smoke build of just the app bundle (no DMG, no notarization),
any minisign key satisfies the updater-artifact step:

```sh
bun run tauri signer generate -w /tmp/test.key --ci
TAURI_SIGNING_PRIVATE_KEY="$(cat /tmp/test.key)" bun run tauri build --bundles app
```

## How the in-app updater behaves

`src/api/autoUpdate.ts` runs once at launch. It is a no-op in dev builds,
calls `check()`, and on an available update downloads, installs and relaunches.
Failures (offline, endpoint down) are logged and swallowed so they never block
startup. Required capabilities live in `src-tauri/capabilities/desktop.json`
(`updater:default`, `process:default`).

## Known limitations

- Windows installers are unsigned; SmartScreen will warn on first launch.
- **Always launch the signed build before publishing the draft release.**
  Notarization does not check entitlements against a provisioning profile, so a
  bundle can pass signing, notarization and Gatekeeper and still be incapable of
  starting. The release workflow now smoke-tests this, but do it yourself too.
