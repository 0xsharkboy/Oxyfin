#!/usr/bin/env bash
# Regenerates the test media used by the player integration tests.
set -euo pipefail
cd "$(dirname "$0")"

internal="$(mktemp --suffix=.srt)"
trap 'rm -f "$internal"' EXIT

cat >"$internal" <<'SRT'
1
00:00:00,000 --> 00:00:02,500
Internal subtitle
SRT

cat >external.srt <<'SRT'
1
00:00:00,000 --> 00:00:02,500
External subtitle
SRT

ffmpeg -hide_banner -loglevel error -y -f lavfi -i "testsrc=size=64x48:rate=10:duration=3" \
  -f lavfi -i "sine=frequency=440:duration=3" -f lavfi -i "sine=frequency=880:duration=3" -i "$internal" \
  -map 0:v -map 1:a -map 2:a -map 3:s -c:v mpeg4 -q:v 10 -c:a flac -ac 1 -ar 8000 -c:s srt \
  -metadata:s:a:0 language=eng -metadata:s:a:0 title=English \
  -metadata:s:a:1 language=fra -metadata:s:a:1 title=French \
  -metadata:s:s:0 language=eng -disposition:a:0 default -disposition:s:0 0 sample.mkv
