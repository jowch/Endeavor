#!/bin/sh
# Check that the build of EndeavorMCP this app embeds carries the checksums of
# its own servers' helpers, so that a helper downloaded from the Helpers release
# is checked against them and not only against the release's own checksum file.
#
#   scripts/pinned-sums.sh KEY
#
# EndeavorMCP's build.rs embeds its scripts/release-key (as PINNED_SUMS) when
# that file pins the build's own key with checksums. That holds for the
# "Plugins: pin build KEY" commit the Helpers pin job pushes, not for the
# commit before it. Run after scripts/helpers.sh, which checks the commit
# Cargo.lock pins out in target/endeavor-mcp (or ENDEAVOR_MCP).
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
