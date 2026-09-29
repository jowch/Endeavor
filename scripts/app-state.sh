#!/bin/sh
# app-state.sh [JQ_FILTER]: print what a running debug build of Endeavor shows,
# as JSON, or the part a jq filter picks (`scripts/app-state.sh .window.screen`).
#
# Start the app with ENDEAVOR_STATE_REQUEST and ENDEAVOR_STATE_OUT set to two
# file paths, and run this with the same two set. See docs/testing.md.
set -eu
: "${ENDEAVOR_STATE_REQUEST:?set it to the request file the app was started with}"
: "${ENDEAVOR_STATE_OUT:?set it to the dump file the app was started with}"

rm -f "$ENDEAVOR_STATE_OUT"
touch "$ENDEAVOR_STATE_REQUEST"
tries=0
while [ ! -f "$ENDEAVOR_STATE_OUT" ]; do
  tries=$((tries + 1))
  if [ "$tries" -gt 100 ]; then
    rm -f "$ENDEAVOR_STATE_REQUEST"
    echo "app-state: no dump after 10 s. Is a debug build running with these paths?" >&2
    exit 1
  fi
  sleep 0.1
done

if [ $# -gt 0 ]; then
  jq -r "$1" "$ENDEAVOR_STATE_OUT"
else
  cat "$ENDEAVOR_STATE_OUT"
fi
