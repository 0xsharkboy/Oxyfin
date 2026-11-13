#!/usr/bin/env bash
# Captures real Jellyfin JSON responses as test fixtures.
# Needs OXYFIN_TEST_* from `jellyfin-test-server.sh up`; refuses non-disposable servers.
set -euo pipefail

if [ "${OXYFIN_TEST_DISPOSABLE:-}" != 1 ]; then
  echo "refusing to run: OXYFIN_TEST_DISPOSABLE is not 1" >&2
  exit 1
fi

BASE=${OXYFIN_TEST_SERVER:?}
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
JSON='Content-Type: application/json'
ACCEPT='Accept: application/json'
AUTH='Authorization: MediaBrowser Client="Oxyfin", Device="Fixture Capture", DeviceId="oxyfin-fixture-capture", Version="0.1.0"'

# Responses are stored as returned (casing differs between versions and routes).
req() {
  local method=$1 path=$2 body=${3:-}
  if [ -n "$body" ]; then
    curl -sf -X "$method" -H "$ACCEPT" -H "$JSON" -H "$AUTH${TOKEN:+, Token=\"$TOKEN\"}" "$BASE$path" -d "$body"
  else
    curl -sf -X "$method" -H "$ACCEPT" -H "$AUTH${TOKEN:+, Token=\"$TOKEN\"}" "$BASE$path"
  fi
}

# Usage: cap NAME METHOD PATH [BODY]; echoes the raw response for reuse.
cap() {
  local name=$1 raw
  raw=$(req "${@:2}")
  jq . <<<"$raw" >"$OUT/$name.json"
  echo "captured $name" >&2
  printf '%s' "$raw"
}

TOKEN=
public=$(req GET /System/Info/Public)
VERSION=$(jq -r '.Version // .version' <<<"$public")
OUT=$ROOT/crates/oxyfin-api/tests/fixtures/$VERSION
rm -rf "$OUT"
mkdir -p "$OUT"
echo "capturing Jellyfin $VERSION into $OUT" >&2
cap system_info_public GET /System/Info/Public >/dev/null

login=$(cap authenticate_by_name POST /Users/AuthenticateByName \
  "$(jq -nc --arg u "$OXYFIN_TEST_USER" --arg p "$OXYFIN_TEST_PASSWORD" '{Username: $u, Pw: $p}')")
TOKEN=$(jq -r '.AccessToken // .accessToken' <<<"$login")
UID_=$(jq -r '(.User // .user) | .Id // .id' <<<"$login")

req GET /System/Configuration | jq -c '.QuickConnectAvailable = true' |
  curl -sf -X POST -H "$JSON" -H "$AUTH, Token=\"$TOKEN\"" "$BASE/System/Configuration" --data-binary @-

items() { jq -r "(.Items // .items)[] | select($1) | (.Id // .id)" <<<"$2" | head -n1; }
all=$(req GET "/Items?userId=$UID_&recursive=true&includeItemTypes=Movie,Series,Episode")
MOVIE=$(items '(.Type // .type) == "Movie"' "$all")
SERIES=$(items '(.Type // .type) == "Series"' "$all")
S1E1=$(jq -r '(.Items // .items)[] | select((.Type // .type) == "Episode" and (.ParentIndexNumber // .parentIndexNumber) == 1 and (.IndexNumber // .indexNumber) == 1) | (.Id // .id)' <<<"$all")
for v in MOVIE SERIES S1E1; do [ -n "${!v}" ] || { echo "missing $v" >&2; exit 1; }; done

# State: progress at 3 s on the movie, S01E01 watched, movie favorite.
playing=$(jq -nc --arg id "$MOVIE" '{ItemId: $id, CanSeek: true, PositionTicks: 0}')
req POST /Sessions/Playing "$playing" >/dev/null
req POST /Sessions/Playing/Progress "$(jq -c '.PositionTicks = 30000000' <<<"$playing")" >/dev/null
req POST /Sessions/Playing/Stopped "$(jq -c '.PositionTicks = 30000000' <<<"$playing")" >/dev/null
cap user_played_items POST "/UserPlayedItems/$S1E1?userId=$UID_" >/dev/null
cap user_favorite_items POST "/UserFavoriteItems/$MOVIE?userId=$UID_" >/dev/null

cap quick_connect_enabled GET /QuickConnect/Enabled >/dev/null
qc=$(cap quick_connect_initiate POST /QuickConnect/Initiate)
SECRET=$(jq -r '.Secret // .secret' <<<"$qc")
cap quick_connect_connect GET "/QuickConnect/Connect?secret=$SECRET" >/dev/null

cap users_me GET /Users/Me >/dev/null
views=$(cap user_views GET "/UserViews?userId=$UID_")
MOVIES_VIEW=$(jq -r '(.Items // .items)[] | select((.Name // .name) == "Movies") | (.Id // .id)' <<<"$views")
cap items_movies GET "/Items?userId=$UID_&parentId=$MOVIES_VIEW&recursive=true&fields=Overview,Genres,People,DateCreated" >/dev/null
cap items_search GET "/Items?userId=$UID_&recursive=true&searchTerm=test" >/dev/null
cap item_movie GET "/Items/$MOVIE?userId=$UID_" >/dev/null
cap item_series GET "/Items/$SERIES?userId=$UID_" >/dev/null
cap item_episode GET "/Items/$S1E1?userId=$UID_" >/dev/null
cap items_latest GET "/Items/Latest?userId=$UID_&parentId=$MOVIES_VIEW" >/dev/null
cap user_items_resume GET "/UserItems/Resume?userId=$UID_" >/dev/null
cap shows_next_up GET "/Shows/NextUp?userId=$UID_" >/dev/null
cap shows_seasons GET "/Shows/$SERIES/Seasons?userId=$UID_" >/dev/null
cap shows_episodes GET "/Shows/$SERIES/Episodes?userId=$UID_" >/dev/null
cap genres GET "/Genres?userId=$UID_" >/dev/null
cap playback_info POST "/Items/$MOVIE/PlaybackInfo?userId=$UID_" \
  "$(jq -nc --arg u "$UID_" '{UserId: $u, EnableDirectPlay: true, EnableDirectStream: true, EnableTranscoding: false, AutoOpenLiveStream: false}')" >/dev/null
