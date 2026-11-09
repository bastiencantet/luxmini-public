#!/bin/bash
# Validate the structural Setapp requirements that can be checked locally.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
# shellcheck source=scripts/lib.sh
. "$ROOT/scripts/lib.sh"

require codesign lipo plutil strings

APP_DIR="${1:-dist/setapp/LuxMini.app}"
INFO_PLIST="${APP_DIR}/Contents/Info.plist"
MAIN_BINARY="${APP_DIR}/Contents/MacOS/mac-led-tray"

if [[ ! -d "$APP_DIR" ]]; then
    echo "ERROR: Setapp app bundle not found: ${APP_DIR}" >&2
    exit 1
fi

assert_plist_value() {
    local key="$1" expected="$2" actual
    actual="$(plutil -extract "$key" raw -o - "$INFO_PLIST")"
    if [[ "$actual" != "$expected" ]]; then
        echo "ERROR: ${key} is '${actual}', expected '${expected}'" >&2
        exit 1
    fi
}

assert_plist_value CFBundleIdentifier "${SETAPP_BUNDLE_ID:-com.bastiencantet.luxmini-setapp}"
assert_plist_value NSUpdateSecurityPolicy.AllowProcesses.MEHY5QF425.0 \
    "com.setapp.DesktopClient.SetappAgent"

architectures="$(lipo -archs "$MAIN_BINARY")"
for architecture in arm64 x86_64; do
    if [[ " ${architectures} " != *" ${architecture} "* ]]; then
        echo "ERROR: main binary does not contain ${architecture}" >&2
        exit 1
    fi
done

for required_path in \
    "${APP_DIR}/Contents/Resources/setappPublicKey.pem" \
    "${APP_DIR}/Contents/Resources/SetappFramework-Resources.bundle"; do
    if [[ ! -e "$required_path" ]]; then
        echo "ERROR: missing required Setapp resource: ${required_path}" >&2
        exit 1
    fi
done

if [[ -e "${APP_DIR}/Contents/Frameworks/Sparkle.framework" ]]; then
    echo "ERROR: the Setapp build must not include Sparkle" >&2
    exit 1
fi

if strings "$MAIN_BINARY" | grep -F "buymeacoffee.com" >/dev/null; then
    echo "ERROR: the Setapp binary contains donation functionality" >&2
    exit 1
fi

if ! strings "$MAIN_BINARY" | grep -F "Setapp.SetappManager" >/dev/null; then
    echo "ERROR: the Setapp static framework was not linked into the main binary" >&2
    exit 1
fi

codesign --verify --deep --strict --verbose=2 "$APP_DIR"
echo "Setapp structural checks passed for ${APP_DIR}"
