#!/usr/bin/env bash
# Runs Oxyfin without a window, with Slint's MCP server, for agent design checks.
# Usage: headless-app.sh [--size WxH] [--scale F] [--lang CODE] [--port N] [--no-build]
# Builds into target/headless (no mpv: video does not show), then runs the app
# with throwaway config, data and cache directories and tokens kept in memory.
# Prints OXYFIN_MCP_URL=... once the server answers, then waits on the app;
# stopping this script stops the app. Drive it with scripts/mcp-call.sh.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

if ! command -v cargo >/dev/null 2>&1; then
  if [ -z "${OXYFIN_HEADLESS_IN_NIX:-}" ] && command -v nix >/dev/null 2>&1; then
    export OXYFIN_HEADLESS_IN_NIX=1
    exec nix develop --quiet "$ROOT" -c "$ROOT/scripts/headless-app.sh" "$@"
  fi
  echo "cargo not found: run inside nix develop" >&2
  exit 1
fi

SIZE=1280x800
SCALE=
LANG_CODE=
PORT=18765
BUILD=1

usage() {
  sed -n '2,7p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
  exit 1
}

while [ $# -gt 0 ]; do
  case $1 in
    --size) SIZE=${2:?--size needs WxH}; shift 2 ;;
    --scale) SCALE=${2:?--scale needs a factor}; shift 2 ;;
    --lang) LANG_CODE=${2:?--lang needs a code}; shift 2 ;;
    --port) PORT=${2:?--port needs a number}; shift 2 ;;
    --no-build) BUILD=; shift ;;
    -h | --help) usage ;;
    *) echo "unknown option: $1" >&2; usage ;;
  esac
done

URL=http://127.0.0.1:$PORT/mcp
TARGET=$ROOT/target/headless
BIN=$TARGET/debug/oxyfin

answers() {
  curl -s -m 2 -X POST "$URL" -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_windows","arguments":{}}}' \
    2>/dev/null | grep -q windowHandles
}

if answers; then
  echo "port $PORT already answers: another headless app is running (use --port)" >&2
  exit 1
fi

if [ -n "$BUILD" ]; then
  echo "building into target/headless (the first build takes several minutes)" >&2
  (cd "$ROOT" && CARGO_TARGET_DIR=$TARGET CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 \
    SLINT_EMIT_DEBUG_INFO=1 cargo build -j 2 -p oxyfin --no-default-features \
    --features slint/mcp,slint/renderer-software) >&2
fi
[ -x "$BIN" ] || { echo "$BIN not found: run without --no-build" >&2; exit 1; }

TMP=$(mktemp -d -t oxyfin-headless.XXXXXX)
APP=
# shellcheck disable=SC2329 # called by the EXIT trap
cleanup() {
  if [ -n "$APP" ]; then
    kill "$APP" 2>/dev/null || true
    wait "$APP" 2>/dev/null || true
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

mkdir -p "$TMP/config" "$TMP/data" "$TMP/cache"
env=(
  XDG_CONFIG_HOME="$TMP/config"
  XDG_DATA_HOME="$TMP/data"
  XDG_CACHE_HOME="$TMP/cache"
  SLINT_BACKEND=headless
  SLINT_MCP_PORT="$PORT"
  OXYFIN_MEMORY_TOKENS=1
  OXYFIN_WINDOW_SIZE="$SIZE"
)
[ -n "$SCALE" ] && env+=(OXYFIN_SCALE_FACTOR="$SCALE")
[ -n "$LANG_CODE" ] && env+=(LANGUAGE="$LANG_CODE" LANG="$LANG_CODE.UTF-8")

env "${env[@]}" "$BIN" >&2 &
APP=$!

for _ in $(seq 1 60); do
  if answers; then
    echo "OXYFIN_MCP_URL=$URL"
    wait "$APP"
    exit $?
  fi
  kill -0 "$APP" 2>/dev/null || { echo "the app exited before its MCP server answered" >&2; exit 1; }
  sleep 1
done
echo "the MCP server did not answer within 60 s" >&2
exit 1
