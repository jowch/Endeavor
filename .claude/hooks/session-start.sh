#!/bin/bash
set -uo pipefail
[ "${CLAUDE_CODE_REMOTE:-}" = true ] || exit 0
cd "$CLAUDE_PROJECT_DIR" || exit 0

scripts/cloud-setup.sh
if [ ! -d frontend/node_modules ]; then
  (cd frontend && npm install --no-audit --no-fund --silent) || echo "session-start: npm install in frontend/ failed" >&2
fi
exit 0
