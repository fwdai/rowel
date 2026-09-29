#!/usr/bin/env bash
#
# Build the Safari web extension (src-tauri/gen/safari) into a signed .appex at
#
#   src-tauri/gen/safari/build/Rowel Safari Extension.appex
#
# which tauri.macos.conf.json copies into Rowel.app/Contents/PlugIns. Tauri
# runs this as build.beforeBundleCommand, macOS only. It has to sign the appex
# itself: Tauri signs only the outer .app, and nested code it did not put
# there itself it leaves as it found it — unsigned code inside a notarized app
# is refused.
#
#   1. the Safari flavour of the web extension (extension/, `build:safari`);
#   2. the Xcode project, regenerated from project.yml when that changed;
#   3. xcodebuild, unsigned, both architectures (the app is universal);
#   4. codesign: with APPLE_SIGNING_IDENTITY — the Developer ID identity the
#      release build signs the app with — hardened runtime and a timestamp;
#      ad hoc without one, which Safari loads only with "Allow unsigned
#      extensions" on (docs/safari-extension.md).
#
# When the identity is not in a keychain but APPLE_CERTIFICATE is set (CI:
# tauri-action hands Tauri the .p12, which Tauri imports only when it signs,
# after this has run), the certificate is imported into a throwaway keychain
# for the length of this script.
#
# Environment:
#   TAURI_ENV_PLATFORM     set by Tauri; anything but darwin is a no-op.
#   TAURI_ENV_DEBUG        "true" for `tauri build --debug`: the debug
#                          configuration, whose socket is the debug app's.
#   APPLE_SIGNING_IDENTITY signing identity; ad hoc when unset.
#   APPLE_CERTIFICATE, APPLE_CERTIFICATE_PASSWORD
#                          base64 .p12 of that identity, imported if needed.
#   ROWEL_SAFARI_WEB_DIR   package this web extension directory instead of
#                          building extension/ (development).

set -euo pipefail

cd "$(dirname "$0")/.."
ROOT=$(pwd)

if [[ "${TAURI_ENV_PLATFORM:-darwin}" != darwin ]]; then
  echo "build-safari-extension: not a macOS build (${TAURI_ENV_PLATFORM}); nothing to do."
  exit 0
fi

PROJECT_DIR="$ROOT/src-tauri/gen/safari"
BUILD_DIR="$PROJECT_DIR/build"
PRODUCT="Rowel Safari Extension.appex"
OUT="$BUILD_DIR/$PRODUCT"
ENTITLEMENTS="$PROJECT_DIR/rowel_safari/rowel_safari.entitlements"

if [[ "${TAURI_ENV_DEBUG:-false}" == true ]]; then
  CONFIGURATION=debug
else
  CONFIGURATION=release
fi

VERSION=$(bun -e 'console.log(require("./src-tauri/tauri.conf.json").version)')

# --- 1. the web extension -----------------------------------------------------
if [[ -n "${ROWEL_SAFARI_WEB_DIR:-}" ]]; then
  WEB_DIR=$(cd "$ROWEL_SAFARI_WEB_DIR" && pwd)
else
  WEB_DIR="$ROOT/extension/build/safari"
  echo "build-safari-extension: building the Safari web extension…"
  (cd extension && bun run build:safari)
fi
if [[ ! -f "$WEB_DIR/manifest.json" ]]; then
  echo "error: no manifest.json in $WEB_DIR" >&2
  exit 1
fi

# --- 2. the Xcode project -----------------------------------------------------
# Committed, like the iOS one, so it opens in Xcode as checked out. Regenerated
# here only when project.yml changed since the cached run, so a build leaves the
# committed project alone.
if command -v xcodegen >/dev/null 2>&1; then
  (cd "$PROJECT_DIR" && xcodegen generate --quiet --use-cache --cache-path "$BUILD_DIR/xcodegen.cache")
elif [[ ! -d "$PROJECT_DIR/rowel_safari.xcodeproj" ]]; then
  echo "error: xcodegen is not installed and there is no project to build (brew install xcodegen)" >&2
  exit 1
else
  echo "build-safari-extension: xcodegen not found; building the committed project as is." >&2
fi

# --- 3. build -----------------------------------------------------------------
echo "build-safari-extension: xcodebuild ($CONFIGURATION, $VERSION)…"
rm -rf "$OUT"
xcodebuild \
  -project "$PROJECT_DIR/rowel_safari.xcodeproj" \
  -scheme rowel_safari \
  -configuration "$CONFIGURATION" \
  -destination 'generic/platform=macOS' \
  -derivedDataPath "$BUILD_DIR/DerivedData" \
  -quiet \
  ARCHS="arm64 x86_64" \
  ONLY_ACTIVE_ARCH=NO \
  CODE_SIGNING_ALLOWED=NO \
  MARKETING_VERSION="$VERSION" \
  CURRENT_PROJECT_VERSION="$VERSION" \
  WEB_EXTENSION_DIR="$WEB_DIR" \
  build
# ditto, not cp: a bundle's symlinks and modes survive it.
ditto "$BUILD_DIR/DerivedData/Build/Products/$CONFIGURATION/$PRODUCT" "$OUT"

# --- 4. sign ------------------------------------------------------------------
IDENTITY="${APPLE_SIGNING_IDENTITY:-}"
KEYCHAIN_ARGS=()
if [[ -n "$IDENTITY" ]]; then
  if ! security find-identity -v -p codesigning | grep -qF "$IDENTITY"; then
    if [[ -z "${APPLE_CERTIFICATE:-}" ]]; then
      echo "error: signing identity \"$IDENTITY\" is not in any keychain, and APPLE_CERTIFICATE is not set" >&2
      exit 1
    fi
    KEYCHAIN="$BUILD_DIR/signing.keychain-db"
    KEYCHAIN_PASSWORD=$(uuidgen)
    ORIGINAL_KEYCHAINS=()
    while IFS= read -r line; do
      line="${line#"${line%%[![:space:]]*}"}"
      ORIGINAL_KEYCHAINS+=("${line//\"/}")
    done < <(security list-keychains -d user)
    cleanup() {
      security list-keychains -d user -s "${ORIGINAL_KEYCHAINS[@]}" || true
      security delete-keychain "$KEYCHAIN" 2>/dev/null || true
      rm -f "$BUILD_DIR/signing.p12"
    }
    trap cleanup EXIT
    rm -f "$KEYCHAIN"
    security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
    security set-keychain-settings -lut 3600 "$KEYCHAIN"
    security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
    (umask 077 && printf '%s' "$APPLE_CERTIFICATE" | base64 --decode > "$BUILD_DIR/signing.p12")
    security import "$BUILD_DIR/signing.p12" -k "$KEYCHAIN" \
      -P "${APPLE_CERTIFICATE_PASSWORD:-}" -T /usr/bin/codesign >/dev/null
    rm -f "$BUILD_DIR/signing.p12"
    security set-key-partition-list -S apple-tool:,apple:,codesign: -s \
      -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null
    # On the search list too, so the chain up to Apple's root resolves.
    security list-keychains -d user -s "$KEYCHAIN" "${ORIGINAL_KEYCHAINS[@]}"
    KEYCHAIN_ARGS=(--keychain "$KEYCHAIN")
  fi
  echo "build-safari-extension: signing with $IDENTITY"
  codesign --force --sign "$IDENTITY" ${KEYCHAIN_ARGS[@]+"${KEYCHAIN_ARGS[@]}"} \
    --options runtime --timestamp \
    --entitlements "$ENTITLEMENTS" \
    "$OUT"
else
  echo "build-safari-extension: APPLE_SIGNING_IDENTITY not set; signing ad hoc (development only)"
  codesign --force --sign - \
    --options runtime \
    --entitlements "$ENTITLEMENTS" \
    "$OUT"
fi

codesign --verify --strict --verbose=1 "$OUT"
echo "build-safari-extension: $OUT"
