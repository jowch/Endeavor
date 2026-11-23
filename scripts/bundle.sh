#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cargo build --locked --release
scripts/helpers.sh || echo "note: no helpers for Linux servers; this build can't connect to them" >&2

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
commit=$(git rev-parse HEAD 2>/dev/null || true)
committed=$(TZ=UTC git log -1 --date=format-local:%Y-%m-%dT%H:%M:%SZ --format=%cd 2>/dev/null || true)
app=target/release/Endeavor.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/endeavor "$app/Contents/MacOS/"
cp -R adapter adapter-codex "$app/Contents/Resources/"
cp assets/icon/Endeavor.icns "$app/Contents/Resources/"
for helper in target/helpers/*/endeavor; do
  [ -f "$helper" ] || continue
  platform=$(basename "$(dirname "$helper")")
  mkdir -p "$app/Contents/Resources/helpers/$platform"
  cp "$helper" "$app/Contents/Resources/helpers/$platform/"
done
mac="$app/Contents/Resources/helpers/darwin-$(uname -m | sed s/arm64/aarch64/)"
mkdir -p "$mac"
cp target/release/endeavor-helper "$mac/endeavor"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Endeavor</string>
  <key>CFBundleDisplayName</key><string>Endeavor</string>
  <key>CFBundleIdentifier</key><string>io.github.jowch.endeavor</string>
  <key>CFBundleExecutable</key><string>endeavor</string>
  <key>CFBundleIconFile</key><string>Endeavor</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>EndeavorCommit</key><string>$commit</string>
  <key>EndeavorCommitted</key><string>$committed</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

codesign --force --sign - "$app"
echo "$app"
