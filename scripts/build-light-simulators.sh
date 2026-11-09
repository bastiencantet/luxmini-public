#!/bin/sh
# Both previews run the same binary and controller, with different presentation metadata.
set -eu
repo_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_dir"
cargo build --locked --bin mac-led-tray
for light_kind in logo led; do
    if [ "$light_kind" = logo ]; then
        app_name="LuxMini Simulator"
        bundle_id="com.bastiencantet.luxmini.simulator"
    else
        app_name="LuxMini LED Simulator"
        bundle_id="com.bastiencantet.luxmini.simulator.led"
    fi
    bundle_dir="$repo_dir/dist/$app_name.app"
    mkdir -p "$bundle_dir/Contents/MacOS" "$bundle_dir/Contents/Resources" "$bundle_dir/Contents/Frameworks"
    cp "${CARGO_TARGET_DIR:-target}/debug/mac-led-tray" "$bundle_dir/Contents/MacOS/luxmini-engine.next"
    mv -f "$bundle_dir/Contents/MacOS/luxmini-engine.next" "$bundle_dir/Contents/MacOS/luxmini-engine"
    cp assets/Icon.icns "$bundle_dir/Contents/Resources/Icon.icns"
    ditto vendor/Sparkle.framework "$bundle_dir/Contents/Frameworks/Sparkle.framework"
    cat > "$bundle_dir/Contents/MacOS/LuxMiniSimulator" <<LAUNCHER
#!/bin/sh
exec "\$(dirname -- "\$0")/luxmini-engine" --simulate-$light_kind
LAUNCHER
    chmod +x "$bundle_dir/Contents/MacOS/LuxMiniSimulator"
    cat > "$bundle_dir/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>LuxMiniSimulator</string>
<key>CFBundleIdentifier</key><string>$bundle_id</string>
<key>CFBundleName</key><string>$app_name</string>
<key>CFBundleDisplayName</key><string>$app_name</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleIconFile</key><string>Icon</string>
<key>CFBundleVersion</key><string>1</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
    printf 'Simulator app: %s\n' "$bundle_dir"
done
