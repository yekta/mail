#!/usr/bin/env bash
# Builds Mail.app into apps/macos/build. Needs Xcode and Rust.
#
#   scripts/build-app.sh [--open]
#
# MAIL_SERVER_URL, if set, is the server a new install uses (default http://localhost:3000).
# MAIL_SIGN_IDENTITY, if set, is the Developer ID certificate the app is signed with.
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(cd ../.. && pwd)"
APP="build/Mail.app"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"

echo "▸ Building the core…"
export MACOSX_DEPLOYMENT_TARGET=14.0
(cd "$ROOT" && cargo build --release -p mail-core)
# What the static library needs from the system, as rustc reports it.
LINK_FLAGS="$(cd "$ROOT" && cargo rustc --release -p mail-core --lib --crate-type staticlib -- --print native-static-libs 2>&1 \
    | sed -n 's/.*native-static-libs: //p' | tail -1)"
export MAIL_CORE_LIB_DIR="$ROOT/target/release"
if [ -n "$LINK_FLAGS" ]; then
    export MAIL_CORE_LINK_FLAGS="$LINK_FLAGS"
fi

echo "▸ Building the app…"
swift build -c release
BINARY="$(swift build -c release --show-bin-path)/Mail"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BINARY" "$APP/Contents/MacOS/Mail"
cp -R "$ROOT/packages/apple/Sources/MailUI/Fonts" "$APP/Contents/Resources/Fonts"

ICONSET="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" Resources/AppIcon.png --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
    sips -z $((size * 2)) $((size * 2)) Resources/AppIcon.png --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Mail</string>
    <key>CFBundleDisplayName</key><string>Mail</string>
    <key>CFBundleExecutable</key><string>Mail</string>
    <key>CFBundleIdentifier</key><string>com.yekta.mail.mac</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
    <key>NSPrincipalClass</key><string>NSApplication</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSAppTransportSecurity</key><dict><key>NSAllowsLocalNetworking</key><true/></dict>
    <key>CFBundleURLTypes</key>
    <array><dict>
        <key>CFBundleURLName</key><string>com.yekta.mail.auth</string>
        <key>CFBundleURLSchemes</key><array><string>mailapp</string></array>
    </dict></array>
    <key>MailServerURL</key><string>${MAIL_SERVER_URL:-http://localhost:3000}</string>
</dict>
</plist>
PLIST

if [ -n "${MAIL_SIGN_IDENTITY:-}" ]; then
    codesign --force --options runtime --timestamp --sign "$MAIL_SIGN_IDENTITY" "$APP"
else
    codesign --force --deep --sign - "$APP" >/dev/null
fi
echo "✓ Built apps/macos/$APP"
if [ "${1:-}" = "--open" ]; then open "$APP"; fi
