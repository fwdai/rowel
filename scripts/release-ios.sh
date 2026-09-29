#!/usr/bin/env bash
#
# Build the App Store IPA and upload it to App Store Connect (TestFlight) from
# this machine, in one command: `bun run release:ios`.
#
# This is the local twin of .github/workflows/release-ios.yml and uses the same
# App Store Connect API key, so signing is automatic (the key mints the Apple
# Distribution certificate and the App Store profile) and the upload needs no
# Apple ID password and no Xcode account. Setup: docs/releasing-ios.md.
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ ! -f .env ]]; then
  echo "error: .env not found. Copy .env.example to .env and fill it in." >&2
  exit 1
fi

set -a
source .env
set +a

# This script signs automatically, through the App Store Connect API key. These
# three switch the Tauri CLI to *manual* signing with whatever certificate and
# profile they name, and it then writes CODE_SIGN_STYLE=Manual,
# CODE_SIGN_IDENTITY and PROVISIONING_PROFILE_SPECIFIER into the committed
# project.pbxproj, where they persist long after the variable is gone. One left
# over in .env or the shell — say from another Tauri app — fails the export with
# a profile/bundle-id mismatch. Drop them for the duration of this build.
for stale in IOS_CERTIFICATE IOS_CERTIFICATE_PASSWORD IOS_MOBILE_PROVISION; do
  if [[ -n ${!stale:-} ]]; then
    echo "warning: ignoring $stale — this script signs via the App Store Connect key." >&2
    unset "$stale"
  fi
done

require() {
  if [[ -z "${!1:-}" ]]; then
    echo "error: $1 is not set in .env — see docs/releasing-ios.md." >&2
    exit 1
  fi
}
require APPLE_API_ISSUER
require APPLE_API_KEY
require APPLE_API_KEY_PATH

# Expand a leading ~ so the .env value can be written the way it is typed.
APPLE_API_KEY_PATH="${APPLE_API_KEY_PATH/#\~/$HOME}"
if [[ ! -f $APPLE_API_KEY_PATH ]]; then
  echo "error: no API key at $APPLE_API_KEY_PATH (APPLE_API_KEY_PATH)." >&2
  exit 1
fi

# The marketing version (CFBundleShortVersionString) is derived from the config
# version by the Tauri CLI, which strips any prerelease tag and keeps exactly
# three integers — so the desktop's `1.0.0-alpha.1` ships to TestFlight as
# `1.0.0` with no iOS-specific config. Mirror that here for the log line only.
VERSION=$(bun -e 'console.log(require("./src-tauri/tauri.conf.json").version)')
SHORT_VERSION=$(bun -e 'const v = require("./src-tauri/tauri.conf.json").version
const m = /^(\d+)\.(\d+)\.(\d+)/.exec(v)
console.log(m ? `${m[1]}.${m[2]}.${m[3]}` : v)')

# CFBundleVersion has to be strictly greater than every previous upload of this
# marketing version. Minutes since the epoch is monotonic, needs no state, and
# stays inside u32. Override for a specific number.
BUILD_NUMBER="${BUILD_NUMBER:-$(( $(date -u +%s) / 60 ))}"

# The Google iOS OAuth client goes in as the app's deep-link URL scheme — the
# client id reversed (README, "Drive sync on iOS") — which registers the OAuth
# redirect and is where the app derives the client id from. It cannot simply be
# exported: `tauri ios build` compiles inside xcodebuild with a replaced
# environment that carries only the CLI's own TAURI_* variables, so an
# `option_env!` never sees the shell. The `--config` patch does get through.
SCHEME=""
if [[ -n ${GOOGLE_OAUTH_IOS_CLIENT_ID:-} ]]; then
  # Anything else — the desktop client, a placeholder with an underscore — would
  # ship as an illegal URL scheme and be rejected at upload (90158).
  if [[ ! $GOOGLE_OAUTH_IOS_CLIENT_ID =~ ^[A-Za-z0-9.+-]+\.apps\.googleusercontent\.com$ ]]; then
    echo "error: GOOGLE_OAUTH_IOS_CLIENT_ID is not an iOS OAuth client id (<id>.apps.googleusercontent.com)." >&2
    exit 1
  fi
  SCHEME="com.googleusercontent.apps.${GOOGLE_OAUTH_IOS_CLIENT_ID%.apps.googleusercontent.com}"
else
  echo "warning: GOOGLE_OAUTH_IOS_CLIENT_ID is not set — building without Drive sync." >&2
fi

# The build number is set as a config value rather than with `tauri ios build
# --build-number`: that flag *appends* to the app version, producing a
# CFBundleVersion like `1.0.0.1.29822658`, and App Store Connect rejects
# anything longer than three period-separated integers (ITMS-90060). Setting
# it outright makes CFBundleVersion exactly the build number.
BUILD_CONFIG=$(bun -e 'const [build, scheme] = process.argv.slice(1)
const config = { bundle: { iOS: { bundleVersion: build } } }
if (scheme) config.plugins = { "deep-link": { mobile: [{ scheme: [scheme] }] } }
console.log(JSON.stringify(config))' "$BUILD_NUMBER" "$SCHEME")

export APPLE_DEVELOPMENT_TEAM="${APPLE_TEAM_ID:-UFBL3F444A}"
export APPLE_API_KEY_PATH

# Deep-link schemes ship verbatim as CFBundleURLTypes and a malformed one is
# rejected at upload (90158). The release workflow runs this same check.
bun scripts/check-ios-url-schemes.mjs

bun scripts/check-tauri-versions.mjs

rustup target add aarch64-apple-ios >/dev/null 2>&1 || true

if [[ ! -d src-tauri/gen/apple ]]; then
  echo "error: src-tauri/gen/apple is missing; run 'bun run tauri ios init'." >&2
  exit 1
fi

# A previous run with one of the variables above leaves manual-signing settings
# behind in the project, and the CLI only ever writes them — it never resets
# them once the variable is gone. Automatic signing cannot take over while they
# are there, so refuse rather than fail later inside exportArchive.
PBXPROJ=src-tauri/gen/apple/rowel.xcodeproj/project.pbxproj
if grep -qE "CODE_SIGN_STYLE = Manual|PROVISIONING_PROFILE_SPECIFIER" "$PBXPROJ"; then
  echo "error: $PBXPROJ carries manual-signing settings from an earlier build." >&2
  echo "       Restore it and re-run:  git checkout -- $PBXPROJ" >&2
  exit 1
fi

shopt -s nullglob

# Unlike the CI runner, this build directory survives between runs, so an IPA
# left by an earlier build — a previous productName, another target — would be
# a candidate below and could be uploaded in place of the build just made.
# Everything under build/ is generated and gitignored, so clearing it is free.
rm -f src-tauri/gen/apple/build/*.ipa src-tauri/gen/apple/build/*/*.ipa

# The scheme reaches Info.plist through tauri-plugin-deep-link's build script,
# and cargo will not rerun that script on its own: the plugin declares
# `rerun-if-env-changed=TAURI_DEEP_LINK_PLUGIN_CONFIG` only when the variable
# is set, so a script that last ran without a config is fingerprinted as
# up-to-date forever and never learns one appeared. Info.plist is then left as
# committed, whatever that holds. Drop its cached run so it re-executes.
cargo clean --manifest-path src-tauri/Cargo.toml --release \
  --target aarch64-apple-ios -p tauri-plugin-deep-link

echo "Building Rowel $SHORT_VERSION build $BUILD_NUMBER (config version $VERSION)…"
bun run tauri ios build --ci \
  --export-method app-store-connect \
  --config "$BUILD_CONFIG"

ipas=(src-tauri/gen/apple/build/*.ipa src-tauri/gen/apple/build/*/*.ipa)
if [[ ${#ipas[@]} -eq 0 ]]; then
  echo "error: no .ipa under src-tauri/gen/apple/build." >&2
  exit 1
fi
# One target means one IPA. Several is ambiguous, and guessing risks uploading
# the wrong one, so say which were found instead of picking.
if [[ ${#ipas[@]} -gt 1 ]]; then
  echo "error: more than one .ipa after the build; not guessing which to upload:" >&2
  printf '  %s\n' "${ipas[@]}" >&2
  exit 1
fi
IPA="$PWD/${ipas[0]}"

# What the IPA registers has to be exactly the scheme asked for (or none): a
# stale or placeholder scheme is rejected by App Store Connect only after the
# whole upload, and a missing one ships a build whose OAuth redirect cannot
# come back. Check here, before the upload.
scripts/check-ipa-url-schemes.sh "$IPA" "$SCHEME"
# Likewise an app or AutoFill extension signed without the credential provider
# entitlement (90729).
scripts/check-ipa-entitlements.sh "$IPA"

# altool ignores APPLE_API_KEY_PATH and only looks for AuthKey_<id>.p8 inside a
# `private_keys` directory (cwd, $HOME, or API_PRIVATE_KEYS_DIR). Stage a copy
# in a temp dir that is removed on any exit, so no key is left lying around.
KEY_DIR=$(mktemp -d)
trap 'rm -rf "$KEY_DIR"' EXIT
mkdir -p "$KEY_DIR/private_keys"
cp "$APPLE_API_KEY_PATH" "$KEY_DIR/private_keys/AuthKey_$APPLE_API_KEY.p8"
chmod 600 "$KEY_DIR/private_keys/AuthKey_$APPLE_API_KEY.p8"

echo
echo "Uploading $(basename "$IPA") to App Store Connect…"
API_PRIVATE_KEYS_DIR="$KEY_DIR/private_keys" \
  xcrun altool --upload-app --type ios --file "$IPA" \
  --apiKey "$APPLE_API_KEY" --apiIssuer "$APPLE_API_ISSUER"

echo
echo "Done. Build $BUILD_NUMBER of $SHORT_VERSION is processing; it shows up under"
echo "TestFlight → iOS builds in roughly 5–30 minutes."
echo "  IPA: $IPA"
