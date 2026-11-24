#!/bin/sh
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
