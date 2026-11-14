#!/usr/bin/env bash
# Starts or stops a disposable Jellyfin server with generated media.
# Usage: jellyfin-test-server.sh up [TAG] | down
# `up` prints `export` lines on stdout; progress goes to stderr.
# OXYFIN_TEST_SCAN_TIMEOUT sets the library scan wait in seconds (default 600).
set -euo pipefail

if [ "${1:-}" = up ] && ! command -v ffmpeg >/dev/null 2>&1; then
  if [ -z "${OXYFIN_TEST_IN_NIX:-}" ] && command -v nix >/dev/null 2>&1; then
    echo "ffmpeg not found, re-running under nix develop" >&2
    export OXYFIN_TEST_IN_NIX=1
    exec nix develop --quiet "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" -c "${BASH_SOURCE[0]}" "$@"
  fi
  echo "ffmpeg not found: install it or run inside nix develop" >&2
  exit 1
fi

NAME=oxyfin-test-jellyfin
PORT=${OXYFIN_TEST_PORT:-18096}
BASE=http://127.0.0.1:$PORT
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
MEDIA=$ROOT/target/jellyfin-test-media
JSON='Content-Type: application/json'
ACCEPT='Accept: application/json'
AUTH='Authorization: MediaBrowser Client="Oxyfin Test", Device="test", DeviceId="oxyfin-test", Version="0.0.0"'

log() { echo "$*" >&2; }

ffm() { env -u LD_LIBRARY_PATH ffmpeg -v error -y "$@"; }

gen_video() {
  local out=$1
  ffm -f lavfi -i testsrc=size=160x90:rate=10:duration=10 \
    -f lavfi -i sine=frequency=440:duration=10 \
    -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest "$out"
}

gen_media() {
  rm -rf "$MEDIA"
  local movie=$MEDIA/movies/"Test Movie (2020)"
  mkdir -p "$movie"
  ffm -f lavfi -i testsrc=size=160x90:rate=10:duration=10 \
    -f lavfi -i sine=frequency=440:duration=10 \
    -f lavfi -i sine=frequency=660:duration=10 \
    -map 0:v -map 1:a -map 2:a \
    -c:v libx264 -pix_fmt yuv420p -c:a aac \
    -metadata:s:a:0 language=eng -metadata:s:a:1 language=fre \
    "$movie/Test Movie (2020).mkv"
  printf '1\n00:00:00,000 --> 00:00:05,000\nHello\n' >"$movie/Test Movie (2020).en.srt"
  printf '%s' '<movie><title>Test Movie (2020)</title><year>2020</year><plot>A generated test movie.</plot><genre>Drama</genre><genre>Comedy</genre></movie>' >"$movie/movie.nfo"
  ffm -f lavfi -i testsrc=size=200x300:rate=1 -frames:v 1 "$movie/poster.jpg"

  local show=$MEDIA/shows/"Test Show" s e dir
  mkdir -p "$show"
  printf '%s' '<tvshow><title>Test Show</title><year>2021</year><plot>A generated test show.</plot><genre>Animation</genre></tvshow>' >"$show/tvshow.nfo"
  for s in 01 02; do
    dir=$show/"Season $s"
    mkdir -p "$dir"
    for e in 01 02; do
      gen_video "$dir/Test Show S${s}E${e}.mkv"
    done
  done
}

# The server answers 503 for a few seconds while it starts up.
retry() {
  local deadline=$((SECONDS + 120)) code
  while :; do
    code=$(curl -s -o /dev/null -w '%{http_code}' "$@" || true)
    case $code in 2*) return 0 ;; esac
    if ((SECONDS >= deadline)); then
      log "timeout (HTTP $code): $*"
      return 1
    fi
    sleep 1
  done
}

wait_public_info() {
  local deadline=$((SECONDS + 120))
  until curl -sf -H "$ACCEPT" "$BASE/System/Info/Public" | jq -e . >/dev/null 2>&1; do
    if ((SECONDS >= deadline)); then
      log "server did not start within 120 s"
      return 1
    fi
    sleep 1
  done
}

run_wizard() {
  retry -X POST -H "$JSON" "$BASE/Startup/Configuration" \
    -d '{"UICulture":"en-US","MetadataCountryCode":"US","PreferredMetadataLanguage":"en"}'
  retry "$BASE/Startup/User"
  retry -X POST -H "$JSON" "$BASE/Startup/User" -d '{"Name":"oxyfin","Password":"oxyfin"}'
  retry -X POST -H "$JSON" "$BASE/Startup/RemoteAccess" -d '{"EnableRemoteAccess":true}'
  retry -X POST "$BASE/Startup/Complete"
  local completed
  completed=$(curl -sf -H "$ACCEPT" "$BASE/System/Info/Public" | jq -r .StartupWizardCompleted)
  if [ "$completed" != true ]; then
    log "startup wizard did not complete"
    return 1
  fi
}

add_library() {
  local name=$1 type=$2 path=$3 types=$4 options
  options=$(jq -nc --arg path "$path" --argjson types "$types" \
    '{LibraryOptions: {PathInfos: [{Path: $path}], TypeOptions: ($types | map({Type: ., MetadataFetchers: [], ImageFetchers: []}))}}')
  curl -sf -X POST -H "$JSON" -H "$AUTH, Token=\"$TOKEN\"" \
    "$BASE/Library/VirtualFolders?name=$name&collectionType=$type&refreshLibrary=true" \
    -d "$options" >/dev/null
}

up() {
  local tag=${1:-12.4}
  down_quiet
  log "generating media in $MEDIA"
  gen_media
  log "starting jellyfin/jellyfin:$tag"
  docker run -d --rm --name "$NAME" -p "127.0.0.1:$PORT:8096" \
    -v "$MEDIA:/media:ro" "jellyfin/jellyfin:$tag" >/dev/null
  log "waiting for the server"
  wait_public_info
  log "running the startup wizard"
  run_wizard

  local login
  login=$(curl -sf -X POST -H "$JSON" -H "$ACCEPT" -H "$AUTH" \
    "$BASE/Users/AuthenticateByName" -d '{"Username":"oxyfin","Pw":"oxyfin"}')
  TOKEN=$(jq -r .AccessToken <<<"$login")
  local token_header="$AUTH, Token=\"$TOKEN\""

  log "disabling the minimum resume duration"
  curl -sf -H "$ACCEPT" -H "$token_header" "$BASE/System/Configuration" |
    jq -c '.MinResumeDurationSeconds = 0' |
    curl -sf -X POST -H "$JSON" -H "$token_header" "$BASE/System/Configuration" --data-binary @- >/dev/null

  log "creating libraries"
  add_library Movies movies /media/movies '["Movie"]'
  add_library Shows tvshows /media/shows '["Series","Season","Episode"]'

  log "waiting for the library scan"
  local timeout=${OXYFIN_TEST_SCAN_TIMEOUT:-600} count=0 start=$SECONDS
  local deadline=$((SECONDS + timeout)) next_log=$((SECONDS + 10))
  while ((SECONDS < deadline)); do
    count=$(curl -sf -H "$ACCEPT" -H "$token_header" \
      "$BASE/Items?recursive=true&includeItemTypes=Movie,Episode" | jq -r .TotalRecordCount || echo 0)
    [ "$count" = 5 ] && break
    if ((SECONDS >= next_log)); then
      log "  scan in progress: $count/5 items after $((SECONDS - start)) s (limit ${timeout} s)"
      next_log=$((SECONDS + 10))
    fi
    sleep 2
  done
  if [ "$count" != 5 ]; then
    log "expected 5 items, found $count after ${timeout} s (raise OXYFIN_TEST_SCAN_TIMEOUT)"
    return 1
  fi

  echo "export OXYFIN_TEST_SERVER=$BASE"
  echo "export OXYFIN_TEST_USER=oxyfin"
  echo "export OXYFIN_TEST_PASSWORD=oxyfin"
  echo "export OXYFIN_TEST_DISPOSABLE=1"
}

down_quiet() { docker rm -f "$NAME" >/dev/null 2>&1 || true; }

case ${1:-} in
  up) up "${2:-12.4}" ;;
  down) down_quiet ;;
  *)
    log "usage: $0 up [TAG] | down"
    exit 2
    ;;
esac
