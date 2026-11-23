#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
key=${1:?usage: scripts/pinned-sums.sh KEY}
pin=${ENDEAVOR_MCP:-target/endeavor-mcp}/scripts/release-key
first=$(head -n 1 "$pin" 2>/dev/null | tr -d ' \t\r\n') || first=
if [ "$first" = "$key" ] && grep -Eq '^[0-9a-fA-F]{64} ' "$pin"; then
  exit 0
fi
echo "::error::The EndeavorMCP commit Cargo.lock pins doesn't pin its own build ($key) in scripts/release-key, so this app would check servers' helpers only against the Helpers release's own checksum file. Pin EndeavorMCP's \"Plugins: pin build $key\" commit instead: cargo update -p endeavor-mcp --precise <that commit>."
exit 1
