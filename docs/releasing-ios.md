# Releasing Rowel for iOS

The `.github/workflows/release-ios.yml` workflow builds the App Store IPA and
uploads it to App Store Connect, where it appears as a TestFlight build. There
is no updater plugin on iOS: the App Store is the update channel, so this
workflow produces no `latest.json` and no updater artifacts. Desktop releases
are a separate workflow (`.github/workflows/release.yml`).

App identity: bundle id `app.rowel.mobile`, team `UFBL3F444A`, minimum iOS
16.0.

iOS has its **own** bundle id. `identifier` in `src-tauri/tauri.conf.json` is
`app.rowel.desktop`, and `src-tauri/tauri.ios.conf.json` overrides
it for iOS builds only — Tauri merges the platform config over the base one
(RFC 7396), and `tauri ios build` writes the merged value into the Xcode
project's `PRODUCT_BUNDLE_IDENTIFIER` on every build. Everything below that
names a bundle id means the iOS one.

## One-time setup

### 1. Apple Developer Program

A paid Apple Developer Program membership is required; a free account cannot
sign for distribution. The team id (`UFBL3F444A`) is the same one already used
for macOS notarization.

### 2. App Store Connect app record

In App Store Connect, **Apps → +  → New App**:

- Platform: iOS
- Bundle ID: `app.rowel.mobile` (register the App ID in the Developer portal
  first if it is not offered. Face ID requires only the usage string, not an
  entitlement; the capabilities the App ID does need are AutoFill Credential
  Provider, the App Group and iCloud — see 4 below for iCloud)
- Name, primary language, SKU: free choice

The record must exist before the first upload; `altool` rejects an IPA whose
bundle id has no app record.

### 3. App Store Connect API key

**Users and Access → Integrations → App Store Connect API → Team Keys →
Generate API Key**:

- Access: **Admin**. Signing here is cloud signing: the key has Xcode create
  the Apple Distribution certificate and the App Store provisioning profile.
  Only Admin and Account Holder can use cloud-managed distribution
  certificates by default, so an **App Manager key fails** with "Cloud signing
  permission error — You haven't been given access to cloud-managed
  distribution certificates", followed by "No profiles for `<bundle id>` were
  found" because the profile cannot be built without the certificate. A key's
  role cannot be changed after creation; generate a new one.
- Download `AuthKey_<KEYID>.p8`. **It can only be downloaded once.** Keep a
  copy in a password manager.
- Note the **Issuer ID** (a UUID, shown above the key list) and the **Key ID**.

Because signing is automatic, no `.p12` certificate and no
`.mobileprovision` profile have to be exported or stored as secrets.

### 4. Google iOS OAuth client (Drive sync)

In the Google Cloud console for the existing Swifty project, **APIs & Services
→ Credentials → Create credentials → OAuth client ID → iOS**, with bundle id
`app.rowel.mobile`. iOS OAuth clients are public: there is **no client
secret**, and none must be set in CI. Copy the client id into the
`GOOGLE_OAUTH_IOS_CLIENT_ID` secret and add the redirect URI
`com.googleusercontent.apps.<id>:/oauth2redirect` to the client. The workflow
passes the id to the build as the app's deep-link URL scheme through
`--config` (environment variables do not reach the iOS compile — README,
"Drive sync on iOS"), and the app derives the client id back from the scheme
(`src-tauri/src/sync/auth.rs`).

Dropbox works the same way with the app key the desktop build already uses:
put it in the `DROPBOX_APP_KEY` secret and the workflow registers
`db-<DROPBOX_APP_KEY>` as a second deep-link scheme beside Google's; the app
reads the key back from it and redirects to `db-<DROPBOX_APP_KEY>://2/token`
(`src-tauri/src/sync/providers/dropbox/oauth.rs`). There is no secret, and
nothing to add in the App Console for the phone: only the OS-level scheme is
registered (confirm on the first device run; see
`docs/sync-providers-design.md` §3).

iCloud needs no secret and no scheme: the device's own account is the
credential. It needs the App ID instead. `rowel_iOS.entitlements` asks for
`com.apple.developer.icloud-services` (`CloudKit`) and the container
`iCloud.app.rowel`, and automatic signing can only mint a profile that grants
them once `app.rowel.mobile` has the *iCloud → CloudKit* capability with that
container attached in the Developer portal — the same container and the same
CloudKit schema the desktop build uses, so do the one-time steps under "iCloud
sync" in [releasing.md](releasing.md) once for both. `check-ipa-entitlements.sh`
refuses an IPA whose app lost either key: a phone build assumes it carries
them (`sync::providers::icloud::supported`), and CloudKit aborts the process on
first use when it does not. The AutoFill extension does not sync and does not
get them.

### 5. Repository secrets

Add under **Settings → Secrets and variables → Actions**:

| Secret | Value |
| --- | --- |
| `APPLE_API_ISSUER` | App Store Connect API **Issuer ID** (UUID) |
| `APPLE_API_KEY` | App Store Connect API **Key ID** (the `<KEYID>` in `AuthKey_<KEYID>.p8`) |
| `APPLE_API_KEY_P8` | base64 of the `AuthKey_<KEYID>.p8` file |
| `APPLE_TEAM_ID` | `UFBL3F444A` — **already set** for desktop notarization; reused here as `APPLE_DEVELOPMENT_TEAM` |
| `GOOGLE_OAUTH_IOS_CLIENT_ID` | Google iOS OAuth client id (no secret) |
| `DROPBOX_APP_KEY` | Dropbox app key (no secret); **already set** if desktop Dropbox sync is configured |

Base64 the key file (macOS):

```sh
base64 -i ~/Downloads/AuthKey_ABC1234567.p8 | pbcopy
```

Line wrapping in the output is fine; the workflow's `base64 --decode` accepts
it. If `APPLE_TEAM_ID` is ever removed, the workflow falls back to the literal
`UFBL3F444A`, which matches `bundle.iOS.developmentTeam` in
`src-tauri/tauri.conf.json`.

## Releasing from your Mac

`bun run release:ios` (`scripts/release-ios.sh`) does the whole thing in one
command: it builds the App Store IPA and uploads it to App Store Connect, where
it becomes a TestFlight build. It is the local twin of the workflow above and
uses the same App Store Connect API key, so signing is automatic and there is
no Apple ID password and no Xcode account involved.

One-time, per machine:

```sh
xcode-select --install                 # or a full Xcode from the App Store
sudo xcodebuild -license accept
rustup target add aarch64-apple-ios
brew install cocoapods                 # `tauri ios build` runs `pod install`
bun install
```

Then put the same values CI uses into `.env` (see `.env.example`):
`APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_API_KEY_PATH` (path to your
`AuthKey_<KEYID>.p8`, kept outside the repo), `GOOGLE_OAUTH_IOS_CLIENT_ID`
and `DROPBOX_APP_KEY`.
`APPLE_TEAM_ID` is already there for desktop notarization and is reused.

```sh
bun run release:ios
```

The script derives `CFBundleVersion` from the clock — minutes since the epoch,
so it always increases — and stages the API key in a temp directory that is
deleted on exit. Pass `BUILD_NUMBER=<n>` to set the build number yourself.

It sets the build number through `--config bundle.iOS.bundleVersion` rather
than through `tauri ios build --build-number`, because that flag *appends* to
the app version: with the version below it would produce a `CFBundleVersion` of
`1.0.0.1.29822658`, and App Store Connect rejects anything longer than three
period-separated integers (ITMS-90060). Set outright, `CFBundleVersion` is just
the build number.

Local and CI build numbers come from different sequences: the workflow uses the
GitHub run number, which is much smaller than a timestamp. Once a local build
of a version has been uploaded, CI cannot upload that same version any more
(its build number would not be strictly greater). Use one or the other per
version, or pass `BUILD_NUMBER` explicitly.

`bun run ios:init` and `bun run ios:dev` wrap `tauri ios init` / `tauri ios dev`
for the rare case where the committed Xcode project has to be regenerated or
you want the app on a connected device.

## Per-release procedure

1. **Bump the version** in `src-tauri/tauri.conf.json`, `package.json` and
   `src-tauri/Cargo.toml` (they must stay in sync; the tag and the release name
   are derived from `tauri.conf.json`).

   A pre-release version needs no special handling: App Store Connect requires
   `CFBundleShortVersionString` to be exactly three dot-separated non-negative
   integers, and the Tauri CLI already strips the prerelease tag on the way
   into the Xcode project (with a warning). The desktop's `1.0.0-alpha.1` ships
   to TestFlight as marketing version `1.0.0`, so desktop and iOS can stay on
   one version string. What distinguishes successive alpha uploads under that
   one marketing version is `CFBundleVersion`, which the release script sets
   per build.

2. **Run the desktop release first.** *Release* ([releasing.md](releasing.md))
   is what creates the draft GitHub release; this workflow only attaches its
   IPA to an existing one. Started first, the iOS run finds no draft, logs a
   notice, exits green, and the IPA survives only as the `rowel-ios-ipa`
   artifact — so the draft the desktop run then creates has no IPA on it and
   you have to re-run iOS anyway. iOS-only build (TestFlight upload, no
   GitHub-release asset)? Then order does not matter.

3. **Run the workflow**: Actions → *Release iOS* → *Run workflow*. It is
   manual-only — nothing triggers it on merge or on a pushed tag — and GitHub
   only offers "Run workflow" for workflows on the default branch.

   **Pick the ref deliberately**, because it decides both what gets built and
   how the IPA is attached in step 5:

   - *A `v<version>` tag* — builds that exact revision, and attaches with
     `--clobber`, replacing any IPA already on that release. This is the one to
     use when re-running a release, and the only way to replace an IPA.
   - *A branch (e.g. `main`)* — builds the branch tip, which may be **ahead of
     the version being released**, and attaches to `v<version from
     tauri.conf.json>` only if that release carries no IPA yet. It never
     replaces one.

4. **Watch the run.** `CFBundleVersion` is set to the workflow run number
   (through `--config bundle.iOS.bundleVersion`, for the ITMS-90060 reason
   above), which is strictly increasing, so re-running the workflow for the
   same version always produces an acceptable new build.

5. **TestFlight processing** takes roughly 5–30 minutes after the upload step
   succeeds. The build then appears under **TestFlight → iOS builds**. The IPA
   is always kept as the `rowel-ios-ipa` workflow artifact — including when
   App Store Connect rejects the upload — and is also attached to the draft
   GitHub release created by the desktop workflow, per the ref rules in step 3.
   No draft yet is a notice, not a failure: the artifact is then the only copy,
   and re-running against the tag once the draft exists attaches it.

6. **Export compliance** must be answered for the build — see below. Until it
   is answered the build stays in "Missing Compliance" and cannot be
   distributed to testers.

7. **App Store submission**: **Apps → Rowel → iOS App → + Version**, fill in
   what's new, screenshots and the review notes (include test-vault credentials
   if the reviewer needs them), select the processed build, then **Add for
   Review → Submit**.

## Export compliance

Rowel encrypts the user's vault with standard, published algorithms:
AES-256-GCM, Argon2id and SQLCipher. That is **not** an exemption. The common
`ITSAppUsesNonExemptEncryption = false` shortcut is wrong for this app and must
**not** be added to `Info.ios.plist`.

Answer the App Store Connect questionnaire as:

- Does your app use encryption? **Yes.**
- Does it qualify for any of the listed exemptions? **No.** The exemptions
  cover apps that only use the encryption built into Apple's OS, only make
  HTTPS calls, or only use encryption for authentication or DRM. Rowel ships
  its own cryptography for user data at rest, so none of them apply.
- Which algorithms? **Standard encryption algorithms instead of, or in addition
  to, using or accessing the encryption in Apple's operating system** — i.e.
  standard published algorithms, no proprietary or non-standard cryptography.

This classification (mass-market software using standard algorithms) requires
an **annual self-classification report to the US Bureau of Industry and
Security (BIS) and the ENC Encryption Request Coordinator**, filed by 1
February each year for the previous calendar year. See
<https://developer.apple.com/documentation/security/complying-with-encryption-export-regulations>.

## App Store privacy nutrition labels

Under **App Privacy**, declare **Data Not Collected**:

- The vault is local to the device; nothing is sent to a Rowel-operated
  server (there is none).
- Drive sync uploads the *encrypted* vault to the user's **own** Google
  account, chosen by the user. It is not collected by the developer and is not
  linked to the user by us.
- No analytics, no crash reporting, no advertising identifiers, no third-party
  SDKs that collect data.

The only user-facing permission is biometrics: the `NSFaceIDUsageDescription`
string in `src-tauri/Info.ios.plist` (Face ID / Touch ID is used to unlock the
local vault). Face ID usage does not itself require a privacy label entry, as
no biometric data leaves the device or reaches the app.

## Troubleshooting

- **"No suitable application records were found"** — the App Store Connect app
  record for `app.rowel.mobile` does not exist yet (step 2).
- **`exportArchive` says a provisioning profile "does not match the bundle ID"**
  — `IOS_CERTIFICATE`, `IOS_CERTIFICATE_PASSWORD` or `IOS_MOBILE_PROVISION` is
  set. Any of them switches the Tauri CLI to manual signing with the profile it
  names, which is how another app's profile ends up signing this one. The
  release script now unsets all three, but a run that got through before writes
  `CODE_SIGN_STYLE = Manual`, `CODE_SIGN_IDENTITY` and
  `PROVISIONING_PROFILE_SPECIFIER` into the *committed* `project.pbxproj` and
  the CLI never resets them. Remove the variables from `.env`, then restore the
  project: `git checkout -- src-tauri/gen/apple/rowel.xcodeproj/project.pbxproj`.
- **ITMS-90060 / invalid `CFBundleVersion`** — the build version came out with
  more than three period-separated integers, which happens when the build
  number is appended to the app version instead of replacing it; see step 1 of
  the release procedure.
- **`src-tauri/gen/apple is missing`** — the Xcode project is committed to the
  repo; regenerate it locally with `bun run tauri ios init` and commit.
- **90158, "URL schemes found in your app are not in the correct format"** —
  the IPA's `CFBundleURLTypes` holds a scheme that is not a legal URL scheme,
  typically the `YOUR_IOS_CLIENT_ID` placeholder. The scheme is written into
  the committed `src-tauri/gen/apple/rowel_iOS/Info.plist` by
  `tauri-plugin-deep-link`'s build script, and that script does **not** rerun
  on its own when a config appears: the plugin declares
  `rerun-if-env-changed` for its config variable only while the variable is
  set, so a run that saw no config is fingerprinted as up-to-date forever and
  the plist ships as committed. Both the release script and the workflow now
  `cargo clean -p tauri-plugin-deep-link` for the iOS target before the build
  so it re-executes, and `scripts/check-ipa-url-schemes.sh` refuses to upload
  an IPA whose schemes differ from the scheme the build was given. If the
  committed plist ever carries a stale scheme again, `check-ios-url-schemes`
  fails before the build; remove the `CFBundleURLTypes` block from it.
- **90737, "Missing Document Configuration"** — the IPA declares
  `CFBundleDocumentTypes`, and iOS then wants `UISupportsDocumentBrowser` or
  `LSSupportsOpeningDocumentsInPlace` as well. `tauri.ios.conf.json` sets
  `bundle.fileAssociations` to `null`, which the RFC 7396 merge treats as a
  delete, so the `.rowel` / `.swftx` associations are desktop-only: nothing on iOS handles
  an opened file today. The block was nevertheless left behind in the committed
  `src-tauri/gen/apple/rowel_iOS/Info.plist` by an earlier `tauri ios init`
  (the CLI only ever adds to that file) and has been removed. Declare the
  association in `tauri.ios.conf.json` (and add one of those keys to
  `Info.ios.plist`) if that changes.
- **"Cloud signing permission error"** — the API key is not an **Admin** key;
  see step 3. Note that `found cert "Apple Distribution: Tauri (unset)"` just
  above it in the log is *not* the cause: that is a self-signed dummy the CLI
  makes to preserve entitlements whenever it skips signing, which it does
  precisely because the API key was found (`mobile/ios/build.rs`).
- **Other signing failures** — usually a key that was revoked; generate a new
  one and update `APPLE_API_KEY`, `APPLE_API_ISSUER` and `APPLE_API_KEY_P8`
  together.
