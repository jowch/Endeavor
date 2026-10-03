#!/bin/sh
# Put the runtime helpers for Linux servers (endeavor-remote, x86_64 and
# aarch64) in target/helpers/<platform>/, where the app and scripts/bundle.sh
# look, for the EndeavorMCP commit that Cargo.lock pins.
#
#   scripts/helpers.sh               keep what's there if it matches, else
#                                    download it, else build it
#   scripts/helpers.sh --fetch-only  keep or download, never build (the git
#                                    hooks in .githooks run this)
#
# It checks out that commit in target/endeavor-mcp and runs EndeavorMCP's own
# scripts/helpers.sh there, which downloads from EndeavorMCP's Helpers release
# or builds. ENDEAVOR_MCP=DIR uses that EndeavorMCP checkout instead, such as
# the one a local [patch] points at (docs/testing.md).
set -eu
cd "$(dirname "$0")/.."
url=https://github.com/jowch/EndeavorMCP
if [ -n "${ENDEAVOR_MCP:-}" ]; then
  mcp=$ENDEAVOR_MCP
else
  rev=$(sed -n "s|^source = \"git+$url[^#]*#\\([0-9a-f]*\\)\"\$|\\1|p" Cargo.lock | head -1)
  [ -n "$rev" ] || { echo "Cargo.lock doesn't pin EndeavorMCP." >&2; exit 1; }
  mcp=target/endeavor-mcp
  [ -d "$mcp/.git" ] || git clone -q --filter=blob:none --no-checkout "$url" "$mcp"
  git -C "$mcp" cat-file -e "$rev^{commit}" 2>/dev/null || git -C "$mcp" fetch -q origin
  git -C "$mcp" -c advice.detachedHead=false checkout -q --detach "$rev"
fi
HELPERS_OUT="$PWD/target/helpers" exec "$mcp/scripts/helpers.sh" "$@"
