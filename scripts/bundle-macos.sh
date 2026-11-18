#!/usr/bin/env bash
# Build ocrouter + ocrouter-gui and assemble dist/Ocrouter.app (ad-hoc signed).
set -euo pipefail

cd "$(dirname "$0")/.."

export PATH="$HOME/.cargo/bin:$PATH"
TARGET=aarch64-apple-darwin
APP="dist/Ocrouter.app"
BIN_DIR="target/$TARGET/release"

echo "building release binaries..."
cargo build --release -p ocrouter-cli -p ocrouter-gui

echo "assembling ${APP}..."
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BIN_DIR/ocrouter" "$BIN_DIR/ocrouter-gui" "$APP/Contents/MacOS/"
chmod +x "$APP/Contents/MacOS/ocrouter" "$APP/Contents/MacOS/ocrouter-gui"

cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Ocrouter</string>
  <key>CFBundleDisplayName</key><string>Ocrouter</string>
  <key>CFBundleIdentifier</key><string>com.oniz93.ocrouter</string>
  <key>CFBundleVersion</key><string>0.1.0</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleExecutable</key><string>ocrouter-gui</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
</dict>
</plist>
PLIST

printf 'APPL????' > "$APP/Contents/PkgInfo"

# Ad-hoc signature so Gatekeeper on the local machine will launch it.
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || \
  echo "warning: codesign failed; the app may need a manual right-click → Open"

echo
echo "built $APP"
echo "  GUI:    $APP/Contents/MacOS/ocrouter-gui"
echo "  daemon: $APP/Contents/MacOS/ocrouter"
echo "open with:  open $APP"
