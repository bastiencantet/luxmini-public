#!/bin/bash
# Build the LuxMini app bundle, including Sparkle only for direct releases.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
# shellcheck source=scripts/lib.sh
. "$ROOT/scripts/lib.sh"

require cargo lipo codesign

FIELD_TEST="${FIELD_TEST:-0}"
if [[ "${FIELD_TEST}" == "1" ]]; then
    APP_NAME="LuxMini Field Test"
    BUNDLE_ID="com.bastiencantet.luxmini.fieldtest"
    APP_DIR="${FIELD_TEST_OUTPUT_DIR:-dist/field-test}/${APP_NAME}.app"
    CARGO_FEATURES=(--features field-test)
else
    APP_NAME="LuxMini"
    BUNDLE_ID="com.bastiencantet.luxmini"
    APP_DIR="dist/${APP_NAME}.app"
    # Keep the array non-empty for the Bash 3.2 shipped with macOS. Expanding
    # an empty array while `set -u` is active raises an unbound-variable error.
    CARGO_FEATURES=(--features direct)
fi
VERSION="${VERSION:-$(cargo_version)}"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
APPCAST_URL="https://dlkmv09vcurlo2fb.public.blob.vercel-storage.com/appcast.xml"
SPARKLE_PUBLIC_KEY=""
if [[ "$FIELD_TEST" != "1" ]]; then
    SPARKLE_PUBLIC_KEY="$(cat "${ROOT}/.sparkle_public_key")"
fi
SIGNING_IDENTITY="${SIGNING_IDENTITY:--}"

sign_code() {
    local target="$1"
    if [[ "$SIGNING_IDENTITY" == "-" ]]; then
        codesign --force --sign - "$target"
    else
        codesign \
            --force \
            --options runtime \
            --timestamp \
            --sign "$SIGNING_IDENTITY" \
            "$target"
    fi
}

echo "==> Building universal release binaries (arm64 + x86_64, version ${VERSION})"
for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --target "${target}" "${CARGO_FEATURES[@]}" --bin mac-led-tray
    cargo build --release --target "${target}" "${CARGO_FEATURES[@]}" --bin led-helper
done

echo "==> Creating bundle: ${APP_DIR}"
rm -rf "${APP_DIR}"
mkdir -p "${APP_DIR}/Contents/MacOS"
mkdir -p "${APP_DIR}/Contents/Resources"
mkdir -p "${APP_DIR}/Contents/Frameworks"

lipo -create \
    "${TARGET_DIR}/aarch64-apple-darwin/release/mac-led-tray" \
    "${TARGET_DIR}/x86_64-apple-darwin/release/mac-led-tray" \
    -output "${APP_DIR}/Contents/MacOS/mac-led-tray"
lipo -create \
    "${TARGET_DIR}/aarch64-apple-darwin/release/led-helper" \
    "${TARGET_DIR}/x86_64-apple-darwin/release/led-helper" \
    -output "${APP_DIR}/Contents/MacOS/led-helper"
chmod +x "${APP_DIR}/Contents/MacOS/mac-led-tray"
chmod +x "${APP_DIR}/Contents/MacOS/led-helper"

echo "==> Embedding app icon"
if [[ ! -f "assets/Icon.icns" ]]; then
    echo "    assets/Icon.icns missing — running make_icon.py"
    require python3
    python3 scripts/make_icon.py
fi
cp "assets/Icon.icns" "${APP_DIR}/Contents/Resources/Icon.icns"
cp "assets/mac-mini-m4.png" "${APP_DIR}/Contents/Resources/mac-mini-m4.png"
cp "assets/mac-mini-silicon.png" "${APP_DIR}/Contents/Resources/mac-mini-silicon.png"
cp "assets/mac-studio.png" "${APP_DIR}/Contents/Resources/mac-studio.png"
cp "assets/welcome-mini-m4.png" "${APP_DIR}/Contents/Resources/welcome-mini-m4.png"
cp "assets/welcome-mini-m4-off.jpg" "${APP_DIR}/Contents/Resources/welcome-mini-m4-off.jpg"
cp "assets/welcome-mini-legacy.png" "${APP_DIR}/Contents/Resources/welcome-mini-legacy.png"
cp "assets/welcome-mini-legacy-off.jpg" "${APP_DIR}/Contents/Resources/welcome-mini-legacy-off.jpg"
cp "assets/welcome-studio.png" "${APP_DIR}/Contents/Resources/welcome-studio.png"
cp "assets/welcome-studio-off.jpg" "${APP_DIR}/Contents/Resources/welcome-studio-off.jpg"

if [[ "$FIELD_TEST" != "1" ]]; then
    echo "==> Embedding Sparkle.framework"
    cp -R "vendor/Sparkle.framework" "${APP_DIR}/Contents/Frameworks/"
fi

SPARKLE_PLIST=""
if [[ "$FIELD_TEST" != "1" ]]; then
    SPARKLE_PLIST="    <key>SUFeedURL</key>
    <string>${APPCAST_URL}</string>
    <key>SUPublicEDKey</key>
    <string>${SPARKLE_PUBLIC_KEY}</string>
    <key>SUEnableAutomaticChecks</key>
    <true/>
    <key>SUScheduledCheckInterval</key>
    <integer>86400</integer>"
fi

cat > "${APP_DIR}/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleIdentifier</key>
    <string>${BUNDLE_ID}</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleExecutable</key>
    <string>mac-led-tray</string>
    <key>CFBundleIconFile</key>
    <string>Icon</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleSignature</key>
    <string>????</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSLocationWhenInUseUsageDescription</key>
    <string>LuxMini uses your location only to compute local sunrise/sunset times for auto-dim. It stays on your Mac.</string>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
${SPARKLE_PLIST}
</dict>
</plist>
PLIST

if [[ "$SIGNING_IDENTITY" == "-" ]]; then
    echo "==> Ad-hoc codesigning app and bundled components"
else
    echo "==> Developer ID codesigning with hardened runtime"
fi

# Sign nested code from the inside out. This keeps every executable covered by
# the same Developer ID identity and avoids relying on codesign --deep to guess
# the framework's bundle boundaries.
if [[ "$FIELD_TEST" != "1" ]]; then
    SPARKLE_DIR="${APP_DIR}/Contents/Frameworks/Sparkle.framework/Versions/B"
    sign_code "${SPARKLE_DIR}/XPCServices/Downloader.xpc"
    sign_code "${SPARKLE_DIR}/XPCServices/Installer.xpc"
    sign_code "${SPARKLE_DIR}/Updater.app"
    sign_code "${SPARKLE_DIR}/Autoupdate"
    sign_code "${APP_DIR}/Contents/Frameworks/Sparkle.framework"
fi
sign_code "${APP_DIR}/Contents/MacOS/led-helper"
sign_code "${APP_DIR}/Contents/MacOS/mac-led-tray"
sign_code "${APP_DIR}"

codesign --verify --deep --strict --verbose=2 "${APP_DIR}"

echo "==> Done: ${APP_DIR}"
