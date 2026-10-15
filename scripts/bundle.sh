#!/bin/sh
# Build target/release/Endeavor.app: the app (Contents/MacOS/endeavor), its
# own files (adapter/ and adapter-codex/, the agents' pinned adapters), and the runtime helpers servers are sent, each named
# endeavor, in Contents/Resources/helpers/<os>-<arch>/: macOS servers' (the
# endeavor-helper target) and any built for Linux servers (target/helpers).
# runtime/ and the skills are built into the binaries (endeavor_mcp::embedded),
# and the app unpacks them. Julia, Node and the ACP
# adapters are not bundled; the app installs them on first use.
# ponytail: ad-hoc signed; Developer ID signing + notarization come with sharing.
set -eu
cd "$(dirname "$0")/.."
cargo build --locked --release
# Linux servers' helpers: kept, downloaded or built (scripts/helpers.sh).
scripts/helpers.sh || echo "note: no helpers for Linux servers; this build can't connect to them" >&2

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
# The commit and its committer time (UTC), as build.rs records them. The
# signature seals Info.plist, so a nightly updating itself (src/updater.rs)
# reads them from a new app to know which build it really is.
commit=$(git rev-parse HEAD 2>/dev/null || true)
committed=$(TZ=UTC git log -1 --date=format-local:%Y-%m-%dT%H:%M:%SZ --format=%cd 2>/dev/null || true)
app=target/release/Endeavor.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/endeavor "$app/Contents/MacOS/"
cp -R adapter adapter-codex "$app/Contents/Resources/"
cp assets/icon/Endeavor.icns "$app/Contents/Resources/"
# Helpers for Linux servers (scripts/helpers.sh, above), without anything
# else in their folders, such as a helper under its old name.
for helper in target/helpers/*/endeavor; do
  [ -f "$helper" ] || continue
  platform=$(basename "$(dirname "$helper")")
  mkdir -p "$app/Contents/Resources/helpers/$platform"
  cp "$helper" "$app/Contents/Resources/helpers/$platform/"
done
# macOS servers', in the folder remote.rs looks in for `uname -s`-`uname -m`.
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
