#!/bin/bash
# Build LED.app bundle, including the Sparkle.framework update engine.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
# shellcheck source=scripts/lib.sh
. "$ROOT/scripts/lib.sh"

require cargo lipo codesign

APP_NAME="LuxMini"
BUNDLE_ID="com.bastiencantet.luxmini"
VERSION="${VERSION:-$(cargo_version)}"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
APP_DIR="dist/${APP_NAME}.app"
APPCAST_URL="https://dlkmv09vcurlo2fb.public.blob.vercel-storage.com/appcast.xml"
SPARKLE_PUBLIC_KEY="$(cat "${ROOT}/.sparkle_public_key")"

echo "==> Building universal release binaries (arm64 + x86_64, version ${VERSION})"
for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --target "${target}" --bin mac-led-tray
    cargo build --release --target "${target}" --bin led-helper
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

echo "==> Embedding Sparkle.framework"
cp -R "vendor/Sparkle.framework" "${APP_DIR}/Contents/Frameworks/"

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
    <key>SUFeedURL</key>
    <string>${APPCAST_URL}</string>
    <key>SUPublicEDKey</key>
    <string>${SPARKLE_PUBLIC_KEY}</string>
    <key>SUEnableAutomaticChecks</key>
    <true/>
    <key>SUScheduledCheckInterval</key>
    <integer>86400</integer>
</dict>
</plist>
PLIST

echo "==> Ad-hoc codesigning (Sparkle + app)"
# Sign the embedded framework first (recursively covers XPCServices inside).
codesign --force --deep --sign - "${APP_DIR}/Contents/Frameworks/Sparkle.framework"
codesign --force --sign - "${APP_DIR}/Contents/MacOS/led-helper"
codesign --force --sign - "${APP_DIR}/Contents/MacOS/mac-led-tray"
codesign --force --sign - "${APP_DIR}"

echo "==> Done: ${APP_DIR}"
