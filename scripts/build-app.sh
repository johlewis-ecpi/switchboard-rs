#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
APP_VERSION="${APP_VERSION:-1.1.0}"
APP_BUILD_NUMBER="${APP_BUILD_NUMBER:-1}"
[[ "$APP_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "APP_VERSION must be MAJOR.MINOR.PATCH" >&2; exit 1; }
[[ "$APP_BUILD_NUMBER" =~ ^[0-9]+$ ]] || { echo "APP_BUILD_NUMBER must be numeric" >&2; exit 1; }
export MACOSX_DEPLOYMENT_TARGET=14.4
cargo build --release --locked
APP="$PWD/dist/Switchboard.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/switchboard "$APP/Contents/MacOS/Switchboard"
cp LICENSE "$APP/Contents/Resources/LICENSE"
rm -rf "$APP/Contents/Resources/Readers"
cp -R readers "$APP/Contents/Resources/Readers"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Switchboard</string>
<key>CFBundleDisplayName</key><string>Switchboard</string>
<key>CFBundleIdentifier</key><string>local.switchboard.rust.app</string>
<key>CFBundleExecutable</key><string>Switchboard</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$APP_VERSION</string>
<key>CFBundleVersion</key><string>$APP_BUILD_NUMBER</string>
<key>LSMinimumSystemVersion</key><string>14.4</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
/usr/bin/plutil -lint "$APP/Contents/Info.plist"
/usr/bin/codesign --force --deep --sign - "$APP"
/usr/bin/codesign --verify --deep --strict "$APP"
echo "Built $APP"
