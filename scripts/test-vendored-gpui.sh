#!/bin/sh
# Runs the line-wrapper tests of the vendored GPUI (vendor/README.md).
# The crate's other tests include files the published crate leaves out, so
# this works on a copy with empty stand-ins for them.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/a/b"
cp -R "$root/vendor/gpui-pre" "$work/a/b/gpui-pre"
crate="$work/a/b/gpui-pre"
mkdir -p "$crate/examples/image" "$work/a/assets/fonts/ibm-plex-sans" "$work/a/assets/fonts/lilex"
: > "$crate/examples/image/exif-orientation-rotate-180.jpg"
: > "$work/a/assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf"
: > "$work/a/assets/fonts/lilex/Lilex-Regular.ttf"
cd "$crate"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target/vendored-gpui}" cargo test --lib --features test-support line_wrapper
