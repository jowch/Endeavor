#!/bin/sh
set -eu
[ $# -eq 3 ] || { echo "usage: $0 DIR COMMIT COMMITTED" >&2; exit 2; }
dir=$1 commit=$2 committed=$3
printf '%s' "$commit" | grep -Eq '^[0-9a-f]{40}$' || { echo "$0: not a full commit hash: $commit" >&2; exit 2; }
printf '%s' "$committed" | grep -Eq '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$' || { echo "$0: not a UTC time: $committed" >&2; exit 2; }

sha256() { if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }

files='{}'
for pair in macos-arm64:Endeavor-macos-arm64.dmg windows-x86_64:Endeavor-windows-x86_64-setup.exe; do
  platform=${pair%%:*} name=${pair#*:}
  [ -f "$dir/$name" ] || continue
  files=$(printf '%s' "$files" | jq --arg p "$platform" --arg n "$name" --arg h "$(sha256 "$dir/$name")" --argjson s "$(wc -c < "$dir/$name" | tr -d ' ')" \
    '.[$p] = { name: $n, sha256: $h, size: $s }')
done
[ "$files" != '{}' ] || { echo "$0: no build to list in $dir" >&2; exit 1; }

jq -n --arg c "$commit" --arg t "$committed" --arg p "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --argjson f "$files" \
  '{ schema: 1, build: $c[0:7], commit: $c, committed: $t, published: $p, files: $f }'
