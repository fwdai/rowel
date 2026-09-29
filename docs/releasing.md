# Releasing Rowel (desktop)

Rowel ships for macOS (signed + notarized universal build), Windows (NSIS
installer) and Linux (.deb, .rpm, AppImage) through GitHub Releases, with a
silent in-app auto-updater. This doc covers the one-time setup and the
per-release procedure. iOS is a separate workflow: see
[releasing-ios.md](releasing-ios.md).

## Overview

```
bump version → run Release manually from Actions → CI builds all three platforms
            → DRAFT GitHub release (installers + .app.tar.gz + .sig + latest.json)
            → you review and publish the draft → installed apps update on next launch
```

- **Workflow:** `.github/workflows/release.yml`, one matrix job per platform.
- **macOS:** `universal-apple-darwin` (one binary for Apple Silicon + Intel),
  signed with the Developer ID Application cert (team `UFBL3F444A`), notarized
  and stapled by `tauri-apps/tauri-action`.
- **Windows:** NSIS `-setup.exe` only, not code-signed (no certificate). The
  MSI bundler is skipped while versions carry a pre-release suffix such as
  `1.0.0-alpha.1`; WiX only accepts numeric pre-release identifiers.
- **Linux:** `.deb`, `.rpm` and `.AppImage`, unsigned.
- **Update artifacts:** `createUpdaterArtifacts: true` in `tauri.conf.json`
  emits the platform-specific updater bundle plus a `.sig` made with the Rowel
  minisign key. `tauri-action` merges them into one `latest.json`, which the app
  fetches from `releases/latest/download/latest.json`.
- **Provenance + SBOM:** every installer gets a SLSA build-provenance
  attestation, and the Linux job attaches CycloneDX SBOMs to the release.

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

1. **Bump the version** in all three files (keep them identical):
   - `src-tauri/tauri.conf.json` → `version` (this is what the release tag and
     name are derived from, via `v__VERSION__`)
   - `package.json` → `version`
   - `src-tauri/Cargo.toml` → `version` (then `cargo update -p rowel
     --manifest-path src-tauri/Cargo.toml` so `Cargo.lock` follows)
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
4. Wait for all three platform jobs. They create one **draft** release
   `Rowel v<version>` with the installers, the updater artifacts, their
   `.sig` files, `latest.json` and the SBOMs. The Linux job replaces the
   placeholder body with GitHub's auto-generated release notes.
5. Review the draft and **Publish** it. Once published, installed apps pick it
   up on next launch.

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
