#!/bin/bash
# Build LED.app and package into a .dmg.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
# shellcheck source=scripts/lib.sh
. "$ROOT/scripts/lib.sh"

require hdiutil

APP_NAME="LuxMini"
VERSION="${VERSION:-$(cargo_version)}"
APP_DIR="dist/${APP_NAME}.app"
DMG_PATH="dist/${APP_NAME}-${VERSION}.dmg"
STAGE_DIR="dist/dmg-stage"

VERSION="${VERSION}" ./scripts/bundle.sh

echo "==> Staging dmg contents"
rm -rf "${STAGE_DIR}"
mkdir -p "${STAGE_DIR}"
cp -R "${APP_DIR}" "${STAGE_DIR}/"
ln -s /Applications "${STAGE_DIR}/Applications"

echo "==> Creating ${DMG_PATH}"
rm -f "${DMG_PATH}"
hdiutil create \
    -volname "${APP_NAME}" \
    -srcfolder "${STAGE_DIR}" \
    -ov \
    -format UDZO \
    "${DMG_PATH}"

rm -rf "${STAGE_DIR}"

echo "==> Done: ${DMG_PATH}"
ls -lh "${DMG_PATH}"
