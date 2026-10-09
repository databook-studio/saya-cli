step "18 · MCP: the same bounded surface over stdio JSON-RPC"
note "Frames are newline-delimited JSON-RPC on stdin; responses stream on stdout."
note "Requests still pending when stdin hits EOF are dropped, so the frames are"
note "followed by a short sleep that keeps stdin open until they are answered."

INIT='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"walkthrough","version":"0"}}}'
INITIALIZED='{"jsonrpc":"2.0","method":"notifications/initialized"}'
LIST='{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
CALL='{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"query","arguments":{"profile":"demo","sql":"SELECT count(*) FROM customers"}}}'
OTHER='{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"query","arguments":{"profile":"other","sql":"SELECT 1"}}}'

# One compact line per response frame — jq when present, python3 otherwise.
if command -v jq >/dev/null 2>&1; then
  mcp_report() {
    jq -r '
      if .id == 1 then "initialize → protocolVersion \(.result.protocolVersion)"
      elif .id == 2 then "tools: \(.result.tools | map(.name) | join(", "))"
      elif .id == 3 and (.result.isError // false) then "query refused: \(.result.content[0].text)"
      elif .id == 3 then "query → \(.result.content[0].text | fromjson | .rows[0] | map(tostring) | join(", "))"
      elif .id == 4 then "off-allowlist profile → isError: \(.result.content[0].text)"
      else empty end'
  }
else
  mcp_report() {
    python3 -c '
import json, sys
for line in sys.stdin:
    m = json.loads(line)
    i, r = m.get("id"), m.get("result", {})
    if i == 1:
        print("initialize → protocolVersion", r["protocolVersion"])
    elif i == 2:
        print("tools:", ", ".join(t["name"] for t in r["tools"]))
    elif i == 3 and r.get("isError"):
        print("query refused:", r["content"][0]["text"])
    elif i == 3:
        print("query →", ", ".join(str(c) for c in json.loads(r["content"][0]["text"])["rows"][0]))
    elif i == 4:
        print("off-allowlist profile → isError:", r["content"][0]["text"])
'
  }
fi

# mcp_probe <dir> <want-exit> <frame names> <frames...> -- <serve args...>:
# pipe the JSON-RPC frames into `mcp serve`, hold stdin open 3s past the last
# frame, then require the server's exit code and show its stderr with the
# extracted report.
mcp_probe() {
  local dir="$1" want="$2" desc="$3"
  shift 3
  local frames=()
  while [ $# -gt 0 ] && [ "$1" != "--" ]; do
    frames+=("$1")
    shift
  done
  shift
  printf '$ (cd %s) %s mcp serve' "$dir" "$SAYA_DISPLAY"
  local arg
  for arg in "$@"; do
    printf ' %s' "$(display_arg "$arg")"
  done
  printf '\n  stdin frames: %s — held open 3s past the last frame\n\n' "$desc"
  local err="$ROOT/mcp-server.err" rcfile="$ROOT/mcp-server.rc" out rc servrc
  out=$(
    { printf '%s\n' "${frames[@]}"; sleep 3; } |
      { (cd "$dir" && "${RUN_PROG[@]}" mcp serve "$@") 2>"$err"; printf '%s' "$?" >"$rcfile"; } |
      mcp_report
  )
  rc=$?
  servrc=$(cat "$rcfile")
  printf '%s\n' "$out"
  printf 'server exit: %s\n' "$servrc"
  printf 'stderr: %s\n' "$(cat "$err")"
  if [ "$rc" -ne 0 ]; then
    fail "the MCP report extractor exited $rc"
  fi
  if [ "$servrc" -ne "$want" ]; then
    fail "saya mcp serve exited $servrc, expected $want"
  fi
  LAST_OUT="$out
stderr: $(cat "$err")"
}

if command -v jq >/dev/null 2>&1 || command -v python3 >/dev/null 2>&1; then
  mcp_probe "$ALICE_HOME" 0 "initialize, notifications/initialized, tools/list" "$INIT" "$INITIALIZED" "$LIST" -- --connections "$ALICE_CON" --profile demo
  require "initialize → protocolVersion 2025-06-18"
  require "tools: contracts, list_profiles, schema"
  require "stderr: saya mcp: profiles: demo (sqlite); data sharing: off"

  mcp_probe "$ALICE_HOME" 0 "initialize, notifications/initialized, tools/list, tools/call query, tools/call query as another profile" "$INIT" "$INITIALIZED" "$LIST" "$CALL" "$OTHER" -- --connections "$ALICE_CON" --profile demo --allow-data-sharing
  require "tools: contracts, investigation_run, list_profiles, query, schema"
  require "query → 240"
  require "off-allowlist profile → isError: profile not available: other"
  require "stderr: saya mcp: profiles: demo (sqlite); data sharing: allowed"
else
  note "MCP probe skipped: neither jq nor python3 is available to extract the responses."
fi

step "Walkthrough complete"
note "All commands behaved as recorded; exit 0."