#!/bin/sh
# Build target/release/Endeavor.app: the release binary plus the app's own files
# (runtime/, plugin/, adapter/) in Contents/Resources. Julia, Node and the ACP
# adapter are not bundled; the app installs them on first launch.
# ponytail: ad-hoc signed; Developer ID signing + notarization come with sharing.
set -eu
cd "$(dirname "$0")/.."
cargo build --release

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
app=target/release/Endeavor.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/endeavor "$app/Contents/MacOS/endeavor"
cp -R runtime plugin adapter "$app/Contents/Resources/"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Endeavor</string>
  <key>CFBundleDisplayName</key><string>Endeavor</string>
  <key>CFBundleIdentifier</key><string>io.github.jowch.endeavor</string>
  <key>CFBundleExecutable</key><string>endeavor</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

codesign --force --sign - "$app"
echo "$app"
