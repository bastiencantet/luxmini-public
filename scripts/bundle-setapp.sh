#!/bin/bash
# Build the Setapp-specific app bundle without Sparkle, donations, or direct-sale UI.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
# shellcheck source=scripts/lib.sh
. "$ROOT/scripts/lib.sh"

require cargo lipo codesign plutil xcrun

APP_NAME="LuxMini"
BUNDLE_ID="${SETAPP_BUNDLE_ID:-com.bastiencantet.luxmini-setapp}"
VERSION="${VERSION:-$(cargo_version)}"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
APP_DIR="${SETAPP_APP_DIR:-dist/setapp/${APP_NAME}.app}"
SETAPP_FRAMEWORK_ROOT="${SETAPP_FRAMEWORK_ROOT:-${ROOT}/vendor/Setapp.xcframework}"
SETAPP_RESOURCES="${SETAPP_RESOURCES:-${ROOT}/vendor/SetappFramework-Resources.bundle}"
SETAPP_PUBLIC_KEY="${SETAPP_PUBLIC_KEY:-${ROOT}/vendor/setappPublicKey.pem}"
SETAPP_LIBRARY_DIR="${SETAPP_FRAMEWORK_ROOT}/macos-arm64_x86_64"
SETAPP_LIBRARY="${SETAPP_LIBRARY_DIR}/libSetapp.a"
SWIFT_RUNTIME_LIBRARY_DIR="${SWIFT_RUNTIME_LIBRARY_DIR:-$(
    xcrun swiftc -print-target-info \
        | plutil -extract paths.runtimeLibraryPaths.0 raw -o - -
)}"
SIGNING_IDENTITY="${SIGNING_IDENTITY:--}"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.5}"
SETAPP_RUSTFLAGS="${RUSTFLAGS:+${RUSTFLAGS} }-L native=${SETAPP_LIBRARY_DIR} -L native=${SWIFT_RUNTIME_LIBRARY_DIR}"

for required_file in "$SETAPP_LIBRARY" "$SETAPP_PUBLIC_KEY"; do
    if [[ ! -f "$required_file" ]]; then
        echo "ERROR: missing Setapp integration file: ${required_file}" >&2
        echo "See docs/SETAPP.md for setup instructions." >&2
        exit 1
    fi
done

if [[ ! -d "$SETAPP_RESOURCES" ]]; then
    echo "ERROR: missing Setapp resource bundle: ${SETAPP_RESOURCES}" >&2
    echo "Setapp Framework 5 and later require this bundle." >&2
    exit 1
fi

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

echo "==> Building universal Setapp binaries"
for target in aarch64-apple-darwin x86_64-apple-darwin; do
    RUSTFLAGS="$SETAPP_RUSTFLAGS" \
        cargo build \
        --release \
        --target "$target" \
        --no-default-features \
        --features setapp \
        --bin mac-led-tray \
        --bin led-helper
done

echo "==> Creating Setapp bundle: ${APP_DIR}"
rm -rf "$APP_DIR"
mkdir -p "${APP_DIR}/Contents/MacOS" "${APP_DIR}/Contents/Resources"

lipo -create \
    "${TARGET_DIR}/aarch64-apple-darwin/release/mac-led-tray" \
    "${TARGET_DIR}/x86_64-apple-darwin/release/mac-led-tray" \
    -output "${APP_DIR}/Contents/MacOS/mac-led-tray"
lipo -create \
    "${TARGET_DIR}/aarch64-apple-darwin/release/led-helper" \
    "${TARGET_DIR}/x86_64-apple-darwin/release/led-helper" \
    -output "${APP_DIR}/Contents/MacOS/led-helper"
chmod +x "${APP_DIR}/Contents/MacOS/mac-led-tray" "${APP_DIR}/Contents/MacOS/led-helper"

cp assets/Icon.icns "${APP_DIR}/Contents/Resources/Icon.icns"
cp assets/mac-mini-m4.png "${APP_DIR}/Contents/Resources/mac-mini-m4.png"
cp assets/mac-mini-silicon.png "${APP_DIR}/Contents/Resources/mac-mini-silicon.png"
cp assets/mac-studio.png "${APP_DIR}/Contents/Resources/mac-studio.png"
cp assets/welcome-mini-m4.png "${APP_DIR}/Contents/Resources/welcome-mini-m4.png"
cp assets/welcome-mini-m4-off.jpg "${APP_DIR}/Contents/Resources/welcome-mini-m4-off.jpg"
cp assets/welcome-mini-legacy.png "${APP_DIR}/Contents/Resources/welcome-mini-legacy.png"
cp assets/welcome-mini-legacy-off.jpg "${APP_DIR}/Contents/Resources/welcome-mini-legacy-off.jpg"
cp assets/welcome-studio.png "${APP_DIR}/Contents/Resources/welcome-studio.png"
cp assets/welcome-studio-off.jpg "${APP_DIR}/Contents/Resources/welcome-studio-off.jpg"
cp "$SETAPP_PUBLIC_KEY" "${APP_DIR}/Contents/Resources/setappPublicKey.pem"
cp -R "$SETAPP_RESOURCES" "${APP_DIR}/Contents/Resources/SetappFramework-Resources.bundle"

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
    <string>${MACOSX_DEPLOYMENT_TARGET}</string>
    <key>LSUIElement</key>
    <true/>
    <key>MPSupportedArchitectures</key>
    <array>
        <string>arm64</string>
        <string>x86_64</string>
    </array>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSLocationWhenInUseUsageDescription</key>
    <string>LuxMini uses your location only to compute local sunrise and sunset times for auto-dim. It stays on your Mac.</string>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
    <key>NSUpdateSecurityPolicy</key>
    <dict>
        <key>AllowProcesses</key>
        <dict>
            <key>MEHY5QF425</key>
            <array>
                <string>com.setapp.DesktopClient.SetappAgent</string>
            </array>
        </dict>
    </dict>
</dict>
</plist>
PLIST

plutil -lint "${APP_DIR}/Contents/Info.plist"

if [[ "$SIGNING_IDENTITY" == "-" ]]; then
    echo "==> Ad-hoc signing the Setapp development bundle"
else
    echo "==> Developer ID signing the Setapp bundle"
fi
sign_code "${APP_DIR}/Contents/MacOS/led-helper"
sign_code "${APP_DIR}/Contents/MacOS/mac-led-tray"
sign_code "$APP_DIR"

codesign --verify --deep --strict --verbose=2 "$APP_DIR"
"${ROOT}/scripts/verify-setapp.sh" "$APP_DIR"

echo "==> Done: ${APP_DIR}"
