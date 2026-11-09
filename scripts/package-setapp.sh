#!/bin/bash
# Build, notarize, staple, and package LuxMini for the Setapp review portal.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
# shellcheck source=scripts/lib.sh
. "$ROOT/scripts/lib.sh"

require codesign ditto plutil spctl xcrun zipinfo

APP_NAME="LuxMini"
VERSION="${VERSION:-$(cargo_version)}"
APP_DIR="dist/setapp/${APP_NAME}.app"
UPLOAD_ZIP="dist/setapp/${APP_NAME}-${VERSION}-notary-upload.zip"
SUBMISSION_ZIP="dist/setapp/${APP_NAME}-${VERSION}-setapp.zip"
STAGE_DIR="dist/setapp/submission"
NOTARY_PROFILE="${NOTARY_PROFILE:-luxmini-notary}"
SIGNING_IDENTITY="${SIGNING_IDENTITY:-}"

if [[ -z "$SIGNING_IDENTITY" || "$SIGNING_IDENTITY" == "-" ]]; then
    echo "ERROR: a Developer ID Application identity is required for Setapp packaging" >&2
    exit 1
fi

SIGNING_IDENTITY="$SIGNING_IDENTITY" VERSION="$VERSION" ./scripts/bundle-setapp.sh

echo "==> Creating temporary notarization archive"
rm -f "$UPLOAD_ZIP"
ditto -c -k --keepParent "$APP_DIR" "$UPLOAD_ZIP"

echo "==> Submitting the Setapp app to Apple notarization"
xcrun notarytool submit \
    "$UPLOAD_ZIP" \
    --keychain-profile "$NOTARY_PROFILE" \
    --wait

echo "==> Stapling and validating the app"
xcrun stapler staple "$APP_DIR"
xcrun stapler validate "$APP_DIR"
codesign --verify --deep --strict --verbose=2 "$APP_DIR"
spctl --assess --type execute --verbose=4 "$APP_DIR"
./scripts/verify-setapp.sh "$APP_DIR"

echo "==> Creating the Setapp review archive"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR"
cp -R "$APP_DIR" "$STAGE_DIR/"
cp assets/Icon.iconset/icon_512x512@2x.png "$STAGE_DIR/AppIcon.png"
rm -f "$SUBMISSION_ZIP"
ditto -c -k --keepParent "$STAGE_DIR" "$SUBMISSION_ZIP"

if zipinfo -1 "$SUBMISSION_ZIP" | grep '^__MACOSX/' >/dev/null; then
    echo "ERROR: submission archive contains a __MACOSX directory" >&2
    exit 1
fi

rm -f "$UPLOAD_ZIP"
rm -rf "$STAGE_DIR"

echo "==> Setapp submission package ready: ${SUBMISSION_ZIP}"
ls -lh "$SUBMISSION_ZIP"
