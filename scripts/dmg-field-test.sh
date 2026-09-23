#!/bin/bash
# Build a private field-test app without replacing the production bundle.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
# shellcheck source=scripts/lib.sh
. "$ROOT/scripts/lib.sh"

require ditto hdiutil

VERSION="${VERSION:-$(cargo_version)}"
APP_NAME="LuxMini Field Test"
OUTPUT_DIR="${FIELD_TEST_OUTPUT_DIR:-dist/field-test}"
APP_DIR="${OUTPUT_DIR}/${APP_NAME}.app"
DMG_PATH="${OUTPUT_DIR}/${APP_NAME}-${VERSION}.dmg"
ZIP_PATH="${OUTPUT_DIR}/${APP_NAME}-${VERSION}.zip"
SIGNING_IDENTITY="${SIGNING_IDENTITY:--}"

mkdir -p "$OUTPUT_DIR"
FIELD_TEST=1 FIELD_TEST_OUTPUT_DIR="$OUTPUT_DIR" VERSION="$VERSION" "$ROOT/scripts/bundle.sh"
ditto -c -k --sequesterRsrc --keepParent "$APP_DIR" "$ZIP_PATH"

stage_dir="$(mktemp -d "${OUTPUT_DIR}/dmg-stage.XXXXXX")"
trap 'rm -rf "$stage_dir"' EXIT
cp -R "$APP_DIR" "$stage_dir/"
ln -s /Applications "$stage_dir/Applications"
if hdiutil create -volname "$APP_NAME" -srcfolder "$stage_dir" -ov -format UDZO "$DMG_PATH"; then
    if [[ "$SIGNING_IDENTITY" != "-" ]]; then
        codesign --force --timestamp --sign "$SIGNING_IDENTITY" "$DMG_PATH"
        codesign --verify --strict --verbose=2 "$DMG_PATH"
    fi
    echo "==> Done: $DMG_PATH"
else
    rm -f "$DMG_PATH"
    echo "==> Disk image unavailable on this host; ZIP is ready instead" >&2
fi

echo "==> Done: $ZIP_PATH"
