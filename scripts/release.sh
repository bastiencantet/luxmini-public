#!/bin/bash
# Release flow (Vercel Blob distribution):
#   1. Build LuxMini-$VERSION.dmg with Developer ID and hardened runtime
#   2. Submit the dmg to Apple, staple its notarization ticket, and verify it
#   3. Sign the dmg with Sparkle's EdDSA private key from Keychain
#   4. Prepend a new <item> to appcast.xml
#   5. Upload both appcast.xml and the dmg to Vercel Blob
#      (via the web repo's `npm run upload-blob` helper)
#
# Usage:  ./scripts/release.sh <version> <release-notes>
# Example: ./scripts/release.sh 0.2.0 "Add presets and launch at login"
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
# shellcheck source=scripts/lib.sh
. "$ROOT/scripts/lib.sh"

require npm awk

VERSION="${1:-}"
NOTES="${2:-Minor update.}"
WEB_DIR="${LUXMINI_WEB_DIR:-${ROOT}/../luxmini-web}"
BLOB_BASE="https://dlkmv09vcurlo2fb.public.blob.vercel-storage.com"
SIGNING_IDENTITY="${SIGNING_IDENTITY:-}"

if [[ -z "$VERSION" ]]; then
    echo "usage: $0 <version> <release-notes>" >&2
    exit 1
fi
if [[ -z "$SIGNING_IDENTITY" ]]; then
    echo "ERROR: SIGNING_IDENTITY must name a Developer ID Application identity" >&2
    exit 1
fi
# Catch a forgotten Cargo.toml bump: the bundle's Info.plist version comes from
# Cargo.toml, so a mismatch would publish a dmg whose internal version is wrong.
CARGO_VERSION="$(cargo_version)"
if [[ "$VERSION" != "$CARGO_VERSION" ]]; then
    echo "WARNING: release version ${VERSION} != Cargo.toml version ${CARGO_VERSION}" >&2
    echo "         bump 'version' in Cargo.toml first so the bundle matches." >&2
fi
if [[ ! -d "$WEB_DIR" ]]; then
    echo "ERROR: luxmini-web repo not found at $WEB_DIR" >&2
    echo "       set LUXMINI_WEB_DIR if it's somewhere else." >&2
    exit 1
fi

DMG="dist/LuxMini-${VERSION}.dmg"
DOWNLOAD_URL="${BLOB_BASE}/LuxMini-${VERSION}.dmg"

echo "==> Building dmg for version ${VERSION}"
VERSION="${VERSION}" ./scripts/dmg.sh

if [[ ! -f "$DMG" ]]; then
    echo "ERROR: dmg not produced at ${DMG}" >&2
    exit 1
fi

echo "==> Notarizing ${DMG}"
./scripts/notarize.sh "$DMG"

echo "==> Signing ${DMG} with EdDSA key from Keychain"
if [[ ! -x ./vendor/bin/sign_update ]]; then
    echo "ERROR: ./vendor/bin/sign_update not found or not executable" >&2
    exit 1
fi
SIGN_OUTPUT="$(./vendor/bin/sign_update "$DMG")"
if [[ -z "$SIGN_OUTPUT" ]]; then
    echo "ERROR: sign_update produced no output" >&2
    exit 1
fi
echo "    ${SIGN_OUTPUT}"

echo "==> Prepending <item> to appcast.xml"
PUB_DATE="$(LC_ALL=C date -u +"%a, %d %b %Y %H:%M:%S +0000")"
SAFE_NOTES="${NOTES//]]>/]]&gt;}"

ITEM="    <item>\\
      <title>Version ${VERSION}</title>\\
      <sparkle:version>${VERSION}</sparkle:version>\\
      <sparkle:shortVersionString>${VERSION}</sparkle:shortVersionString>\\
      <sparkle:minimumSystemVersion>11.0</sparkle:minimumSystemVersion>\\
      <pubDate>${PUB_DATE}</pubDate>\\
      <description><![CDATA[${SAFE_NOTES}]]></description>\\
      <enclosure url=\"${DOWNLOAD_URL}\" ${SIGN_OUTPUT} type=\"application/octet-stream\" />\\
    </item>"

TMP="$(mktemp)"
awk -v item="$ITEM" '
/<\/channel>/ {
    gsub(/\\\n[ \t]*/, "\n", item)
    print item
}
{ print }
' appcast.xml > "$TMP"
mv "$TMP" appcast.xml

echo "==> Uploading dmg to Vercel Blob"
( cd "$WEB_DIR" && npm run upload-blob -- "${ROOT}/${DMG}" )

echo "==> Uploading appcast.xml to Vercel Blob"
( cd "$WEB_DIR" && npm run upload-blob -- "${ROOT}/appcast.xml" )

echo
echo "==> Done. LuxMini ${VERSION} published."
echo "    dmg:   ${DOWNLOAD_URL}"
echo "    feed:  ${BLOB_BASE}/appcast.xml"
echo
echo "Remember to commit the updated appcast.xml to source control if you want"
echo "to keep a history of released versions."
