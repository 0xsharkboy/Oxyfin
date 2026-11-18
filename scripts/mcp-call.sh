#!/usr/bin/env bash
# Calls a tool of the Slint MCP server of a running headless Oxyfin.
# Usage: mcp-call.sh TOOL [JSON_ARGS] [OUT.png]
#        mcp-call.sh --find LABEL [ROLE]
# TOOL is a Slint MCP tool (list_windows, take_screenshot, get_element_tree,
# click_element, set_element_value, dispatch_key_event, ...). With OUT.png the
# image of the answer is written there. --find prints the handle of the first
# element of the first window with that accessible label (and role).
# OXYFIN_MCP_URL sets the server (default http://127.0.0.1:18765/mcp).
set -euo pipefail

URL=${OXYFIN_MCP_URL:-http://127.0.0.1:18765/mcp}

die() {
  echo "mcp-call: $*" >&2
  exit 1
}

for tool in curl jq base64; do
  command -v "$tool" >/dev/null 2>&1 || die "$tool not found: run inside nix develop"
done

call() {
  local request answer
  request=$(jq -cn --arg name "$1" --argjson args "$2" \
    '{jsonrpc: "2.0", id: 1, method: "tools/call", params: {name: $name, arguments: $args}}') ||
    die "arguments are not valid JSON: $2"
  answer=$(curl -s -m 60 -X POST "$URL" -H 'Content-Type: application/json' -d "$request") ||
    die "no answer from $URL: is the headless app running?"
  [ -n "$answer" ] || die "empty answer from $URL"
  if echo "$answer" | jq -e '.error' >/dev/null; then
    die "$(echo "$answer" | jq -r '.error.message // .error')"
  fi
  if echo "$answer" | jq -e '.result.isError == true' >/dev/null; then
    die "$(echo "$answer" | jq -r '[.result.content[]? | .text // empty] | join(" ")')"
  fi
  echo "$answer"
}

first_window() {
  call list_windows '{}' | jq -r '.result.content[0].text' | jq -c '.windowHandles[0] // empty'
}

if [ "${1:-}" = --find ]; then
  [ $# -ge 2 ] || die "usage: mcp-call.sh --find LABEL [ROLE]"
  window=$(first_window)
  [ -n "$window" ] || die "no window"
  root=$(call get_window_properties "{\"windowHandle\":$window}" |
    jq -r '.result.content[0].text' | jq -c '.rootElementHandle')
  handle=$(call get_element_tree "{\"elementHandle\":$root,\"maxElements\":5000}" |
    jq -r '.result.content[0].text' |
    jq -c --arg label "$2" --arg role "${3:-}" \
      '[.. | objects | select(.accessibleLabel == $label and ($role == "" or .accessibleRole == $role)) | .handle][0] // empty')
  [ -n "$handle" ] || die "no element labelled \"$2\"${3:+ with role $3}"
  echo "$handle"
  exit 0
fi

[ $# -ge 1 ] || die "usage: mcp-call.sh TOOL [JSON_ARGS] [OUT.png] | --find LABEL [ROLE]"
args=${2:-}
[ -n "$args" ] || args='{}'
answer=$(call "$1" "$args")
if [ -n "${3:-}" ]; then
  data=$(echo "$answer" | jq -r '[.result.content[]? | select(.type == "image") | .data][0] // empty')
  [ -n "$data" ] || die "the answer has no image"
  echo "$data" | base64 -d >"$3"
  echo "$3"
else
  echo "$answer" | jq -r '[.result.content[]? | .text // empty] | join("\n")'
fi
