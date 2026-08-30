#!/bin/bash
# Submit a signed LuxMini DMG to Apple, staple the resulting ticket, and verify
# the final artifact. Credentials stay in the macOS Keychain under the profile
# created with `xcrun notarytool store-credentials`.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
# shellcheck source=scripts/lib.sh
. "$ROOT/scripts/lib.sh"

require codesign spctl xcrun

DMG_PATH="${1:-}"
NOTARY_PROFILE="${NOTARY_PROFILE:-luxmini-notary}"

if [[ -z "$DMG_PATH" || ! -f "$DMG_PATH" ]]; then
    echo "usage: $0 <signed-dmg>" >&2
    exit 1
fi

echo "==> Verifying Developer ID signature"
codesign --verify --strict --verbose=2 "$DMG_PATH"

echo "==> Submitting to Apple notarization"
xcrun notarytool submit \
    "$DMG_PATH" \
    --keychain-profile "$NOTARY_PROFILE" \
    --wait

echo "==> Stapling notarization ticket"
xcrun stapler staple "$DMG_PATH"
xcrun stapler validate "$DMG_PATH"

echo "==> Verifying Gatekeeper assessment"
spctl \
    --assess \
    --type open \
    --context context:primary-signature \
    --verbose=4 \
    "$DMG_PATH"

echo "==> Notarized artifact ready: ${DMG_PATH}"
