#!/usr/bin/env bash
#
# Fails unless both the app and its AutoFill extension inside the IPA are
# signed with the credential provider entitlement and the App Group.
#
#   scripts/check-ipa-entitlements.sh <path.ipa>
#
# `tauri ios build` with an App Store Connect key archives unsigned and signs
# the app with rowel_iOS.entitlements, and the extension through the app
# target's "Sign AutoFill Extension For Export" phase (project.yml). If either
# loses the entitlement, App Store Connect refuses it only after the upload
# (90729). Both the local release script and the workflow run this between the
# build and the upload.
set -euo pipefail

IPA="${1:?usage: check-ipa-entitlements.sh <path.ipa>}"

FILES=$(unzip -Z1 "$IPA")
APP_DIR=$(grep -E '^Payload/[^/]+\.app/Info\.plist$' <<< "$FILES" | head -1 | sed -E 's#/Info\.plist$##')
if [[ -z $APP_DIR ]]; then
  echo "error: no Payload/*.app inside $IPA" >&2
  exit 1
fi
APPEX_DIR="$APP_DIR/PlugIns/rowel_autofill.appex"
if ! grep -qxF "$APPEX_DIR/Info.plist" <<< "$FILES"; then
  echo "error: no rowel_autofill.appex inside $IPA" >&2
  exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
unzip -q "$IPA" -d "$TMP"

for bundle in "$APP_DIR" "$APPEX_DIR"; do
  codesign -d --entitlements - --xml "$TMP/$bundle" 2>/dev/null > "$TMP/ents.plist"
  for key in com.apple.developer.authentication-services.autofill-credential-provider \
    com.apple.security.application-groups; do
    # PlistBuddy, not plutil: plutil reads the dots in the key as a key path.
    if ! /usr/libexec/PlistBuddy -c "Print :$key" "$TMP/ents.plist" >/dev/null 2>&1; then
      echo "error: $(basename "$bundle") in the IPA is not signed with $key." >&2
      exit 1
    fi
  done
done

echo "check-ipa-entitlements: app and AutoFill extension carry their entitlements"
