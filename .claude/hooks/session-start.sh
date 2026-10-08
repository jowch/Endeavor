#!/bin/bash
# SessionStart hook for Claude Code cloud sessions on this repository alone.
# A session with several repositories (a Claude project) doesn't run repo
# hooks; there the environment's setup script does this (docs/cloud.md).
set -uo pipefail
[ "${CLAUDE_CODE_REMOTE:-}" = true ] || exit 0
cd "$CLAUDE_PROJECT_DIR" || exit 0

scripts/cloud-setup.sh
# The page script's tests and typecheck (frontend/dist/page.js is committed).
if [ ! -d frontend/node_modules ]; then
  (cd frontend && npm install --no-audit --no-fund --silent) || echo "session-start: npm install in frontend/ failed" >&2
fi
exit 0
