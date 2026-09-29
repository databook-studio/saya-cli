#!/usr/bin/env bash
# scripts/walkthrough.sh — a recorded, reproducible synthetic walkthrough of
# the whole journey: demo → setup refusal → question → inspect SQL/evidence →
# save → export → import in a second environment → rerun, then the 0.4.2
# surface on top: a parameterised investigation (saved, run, and refused when
# the required parameter is missing), remembered context exported to the
# second environment and queued there pending review, a CSV staged by
# `saya open` as a read-only text snapshot (previewed, listed, queried,
# cleaned up), and the same bounded surface spoken to over MCP stdio JSON-RPC.
#
#   bash scripts/walkthrough.sh                 # uses `cargo run -q -p saya-cli --`
#   SAYA_BIN=/path/to/saya scripts/walkthrough.sh
#   WALKTHROUGH_KEEP=1 bash scripts/walkthrough.sh   # keep the sandbox for inspection
#
# Everything runs against SYNTHETIC data only, with no AI provider and no
# network: the demo database is a local SQLite file and every command below
# is the bounded, read-only path. Two isolated roots ("alice" and "bob") each
# get their own HOME, SAYA_CONFIG_HOME, SAYA_STATE_DB, SAYA_DEMO_DIR, and
# SAYA_INVESTIGATIONS_DIR, so the export/import leg really crosses between
# two environments. Every command is printed before its output, every exit
# code is checked against the expected one, and any unexpected result exits
# non-zero. docs/walkthrough.md embeds the recorded output of this script.

set -u -o pipefail

cd "$(dirname "$0")/.."

SAYA_DISPLAY="cargo run -q -p saya-cli --"
SAYA_PROG=(cargo run -q -p saya-cli --)
if [ -n "${SAYA_BIN:-}" ]; then
  SAYA_DISPLAY="$SAYA_BIN"
  SAYA_PROG=("$SAYA_BIN")
fi

ROOT=$(mktemp -d "${TMPDIR:-/tmp}/saya-walkthrough.XXXXXX")
ALICE_HOME="$ROOT/alice"
BOB_HOME="$ROOT/bob"
ALICE_CON="$ALICE_HOME/demo/connections.toml"
BOB_CON="$BOB_HOME/demo/connections.toml"
SHARED="$ROOT/shared"

if [ -z "${WALKTHROUGH_KEEP:-}" ]; then
  trap 'rm -rf "$ROOT"' EXIT
fi
mkdir -p "$SHARED"

# Nothing from the outer environment may steer this walkthrough.
unset SAYA_PROFILE SAYA_DB_TYPE SAYA_TRUST_PROJECT_CONFIG SAYA_EXTRACTION_TRACE

# Hijacking HOME below would send rustup and cargo to the empty sandbox, so
# pin their homes to the real one unless the environment already chose them.
REAL_HOME="$HOME"
RUSTUP_HOME="${RUSTUP_HOME:-$REAL_HOME/.rustup}"
CARGO_HOME="${CARGO_HOME:-$REAL_HOME/.cargo}"
export RUSTUP_HOME CARGO_HOME

LAST_OUT=""

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

step() {
  printf '\n== %s ==\n\n' "$*"
}

note() {
  printf '%s\n' "$*"
}

# run <expected-exit> <saya args...>: print the command, then its combined
# output, then require the exit code to be exactly <expected-exit>.
#
# The command line is echoed copy-paste-safe: an argument containing spaces or
# shell metacharacters is shown single-quoted, anything else verbatim.
display_arg() {
  case "$1" in
    "" | *[![:alnum:]%+,./:@^_-]*) printf "'%s'" "$1" ;;
    *) printf '%s' "$1" ;;
  esac
}

run() {
  local want="$1"
  shift
  printf '$ %s' "$SAYA_DISPLAY"
  local arg
  for arg in "$@"; do
    printf ' %s' "$(display_arg "$arg")"
  done
  printf '\n\n'
  local out rc
  out=$("${SAYA_PROG[@]}" "$@" 2>&1)
  rc=$?
  if [ -n "$out" ]; then
    printf '%s\n' "$out"
  fi
  if [ "$rc" -ne "$want" ]; then
    fail "\"$*\" exited $rc, expected $want"
  fi
  LAST_OUT="$out"
}

# require <needle>: the previous command's output had to contain <needle>.
require() {
  if [[ "$LAST_OUT" != *"$1"* ]]; then
    printf -- '--- output was ---\n%s\n' "$LAST_OUT" >&2
    fail "expected output to contain: $1"
  fi
}

# The journeys below run saya from INSIDE the sandbox: a project config layer
# (.saya/ in the checkout's cwd) would otherwise be consulted. In cargo-run
# mode the manifest path is pinned explicitly, so cargo still finds the
# workspace from the sandbox; the printed line keeps the reader-facing form.
REPO_DIR=$PWD
if [ -n "${SAYA_BIN:-}" ]; then
  RUN_PROG=("${SAYA_PROG[@]}")
else
  RUN_PROG=(cargo run -q --manifest-path "$REPO_DIR/Cargo.toml" -p saya-cli --)
fi

# run_from <dir> <expected-exit> <saya args...>: like run, but the command
# executes with <dir> as its working directory.
run_from() {
  local dir="$1" want="$2"
  shift 2
  printf '$ (cd %s) %s' "$dir" "$SAYA_DISPLAY"
  local arg
  for arg in "$@"; do
    printf ' %s' "$(display_arg "$arg")"
  done
  printf '\n\n'
  local out rc
  out=$(cd "$dir" && "${RUN_PROG[@]}" "$@" 2>&1)
  rc=$?
  if [ -n "$out" ]; then
    printf '%s\n' "$out"
  fi
  if [ "$rc" -ne "$want" ]; then
    fail "\"$*\" exited $rc, expected $want"
  fi
  LAST_OUT="$out"
}

# use_alice / use_bob: point the whole environment at one isolated root.
use_alice() {
  HOME="$ALICE_HOME"
  SAYA_CONFIG_HOME="$ALICE_HOME/config"
  SAYA_STATE_DB="$ALICE_HOME/state.sqlite3"
  SAYA_DEMO_DIR="$ALICE_HOME/demo"
  SAYA_INVESTIGATIONS_DIR="$ALICE_HOME/investigations"
  export HOME SAYA_CONFIG_HOME SAYA_STATE_DB SAYA_DEMO_DIR SAYA_INVESTIGATIONS_DIR
}

use_bob() {
  HOME="$BOB_HOME"
  SAYA_CONFIG_HOME="$BOB_HOME/config"
  SAYA_STATE_DB="$BOB_HOME/state.sqlite3"
  SAYA_DEMO_DIR="$BOB_HOME/demo"
  SAYA_INVESTIGATIONS_DIR="$BOB_HOME/investigations"
  export HOME SAYA_CONFIG_HOME SAYA_STATE_DB SAYA_DEMO_DIR SAYA_INVESTIGATIONS_DIR
}

step "0 · Environment"
note "Binary: $SAYA_DISPLAY (override with SAYA_BIN=…)"
note "Sandbox: $ROOT — two isolated roots, alice/ and bob/"
note "Isolated per root: HOME, SAYA_CONFIG_HOME, SAYA_STATE_DB, SAYA_DEMO_DIR, SAYA_INVESTIGATIONS_DIR"
note "No AI provider is configured and nothing here touches the network."

# The one saved query: the duplicate-join trap. Joining orders to
# customer_contacts repeats each order once per contact row, so count(*)
# reports 301 "orders" where only 226 distinct orders exist among customers
# that have contacts — and every order of a contactless customer vanishes.
TRAP_SQL="SELECT c.region, count(*) AS joined_rows, count(DISTINCT o.id) AS distinct_orders FROM orders o JOIN customers c ON c.id = o.customer_id JOIN customer_contacts cc ON cc.customer_id = c.id GROUP BY c.region ORDER BY joined_rows DESC"
PLAIN_SQL="SELECT c.region, count(*) AS orders FROM orders o JOIN customers c ON c.id = o.customer_id GROUP BY c.region ORDER BY orders DESC"

use_alice

step "1 · Alice builds the demo database (synthetic customers and orders)"
run 0 demo --non-interactive
require "Demo database (built a new fixture): $ALICE_HOME/demo/demo.sqlite3"
require "Connections file: $ALICE_CON"

step "2 · \`saya setup\` needs a terminal — from a script it refuses and points elsewhere"
note "(the guided \`saya setup\` itself is interactive; this walkthrough only shows the scripted refusal)"
run 2 setup --non-interactive
require "saya setup is interactive. For a scripted start use \`saya config init\` (templates) or \`saya demo\` (sample database)."

step "3 · Alice asks the demo database — the honest counts first"
run 0 --connections "$ALICE_CON" --profile demo --non-interactive query --sql "SELECT count(*) AS customers FROM customers"
require "240"
run 0 --connections "$ALICE_CON" --profile demo --non-interactive query --sql "SELECT count(*) AS contact_rows, count(DISTINCT cc.customer_id) AS customers_with_contacts FROM customer_contacts cc"
require "123"
require "93"

step "4 · The trap: join customer_contacts and the numbers stop agreeing"
note "joined_rows counts rows; distinct_orders counts real orders. The join repeats an order once per contact row."
run 0 --connections "$ALICE_CON" --profile demo --non-interactive query --sql "$TRAP_SQL"
require "north	83	59"
run 0 --connections "$ALICE_CON" --profile demo --non-interactive query --sql "$PLAIN_SQL"
require "north	155"

step "5 · Alice saves the trap query as a portable investigation"
run 0 --connections "$ALICE_CON" --profile demo --non-interactive investigation save --name "Orders-per-region via contact join" --description "Duplicate-join trap: order rows multiplied by customer_contacts" --sql "$TRAP_SQL"
require "Saved exactly as shown. Review the SQL before sharing: literals are stored verbatim."
ID=$(printf '%s\n' "$LAST_OUT" | sed -n 1p)
case "$ID" in
*[!a-z0-9-]* | "") fail "unexpected investigation id: $ID" ;;
esac

step "6 · list and show record exactly what was saved"
run 0 --connections "$ALICE_CON" --profile demo --non-interactive investigation list
require "$ID"
run 0 --connections "$ALICE_CON" --profile demo --non-interactive investigation show "$ID"
require '"format": "saya.investigation"'
require "local binding: demo (reviewed revision 1)"

step "7 · Alice runs the saved investigation — same query, same evidence line"
run 0 --connections "$ALICE_CON" --profile demo --non-interactive investigation run "$ID"
require "saved investigation: demo · 4 rows ·"
require "north	83	59"
run 0 --connections "$ALICE_CON" --profile demo --non-interactive investigation run "$ID" --report "$SHARED/report.md"
require "Wrote report to $SHARED/report.md (rows omitted)"

step "8 · Alice exports the portable definition (no binding, no rows, no credentials)"
run 0 --connections "$ALICE_CON" --profile demo --non-interactive investigation export "$ID" "$SHARED/investigation.json"
require "Exported $ID to $SHARED/investigation.json."

use_bob

step "9 · Bob builds the same demo database (the fixture is deterministic)"
run 0 demo --non-interactive
require "Demo database (built a new fixture): $BOB_HOME/demo/demo.sqlite3"

step "10 · Bob imports the definition — a preview, and nothing executes"
run 0 --connections "$BOB_CON" --non-interactive investigation import "$SHARED/investigation.json"
require "Imported without a local connection. Run with --connection <profile> to map it; nothing was executed."

step "11 · Bob's first run is refused — no connection mapping travelled with the file"
run 2 --connections "$BOB_CON" --non-interactive investigation run "$ID"
require "no local connection mapped: pass --connection <profile>"

step "12 · Bob runs it on his own demo database with an explicit --connection"
run 0 --connections "$BOB_CON" --non-interactive investigation run "$ID" --connection demo
require "saved investigation: demo · 4 rows ·"
require "north	83	59"

if command -v sqlite3 >/dev/null 2>&1 || command -v python3 >/dev/null 2>&1; then
  step "13 · Bonus: a stale review is refused without any ALTER"
  note "A second SQLite database with the same table names but a different shape"
  note "(orders gains a note column; customer_contacts loses channel). No ALTER ran —"
  note "the differently-shaped schema was built directly, and the review refuses it."
  SKEWED_DB="$BOB_HOME/skewed.sqlite3"
  SKEWED_CON="$BOB_HOME/skewed-connections.toml"
  if command -v sqlite3 >/dev/null 2>&1; then
    sqlite3 "$SKEWED_DB" "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER NOT NULL, order_date TEXT NOT NULL, amount_cents INTEGER, status TEXT NOT NULL, note TEXT); CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL, region TEXT NOT NULL); CREATE TABLE customer_contacts (customer_id INTEGER NOT NULL, value TEXT NOT NULL);"
    note "skewed schema:"
    sqlite3 "$SKEWED_DB" ".schema"
  else
    python3 - "$SKEWED_DB" <<'PY'
import sqlite3, sys
db = sqlite3.connect(sys.argv[1])
db.executescript(
    "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER NOT NULL,"
    " order_date TEXT NOT NULL, amount_cents INTEGER, status TEXT NOT NULL, note TEXT);"
    "CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL, region TEXT NOT NULL);"
    "CREATE TABLE customer_contacts (customer_id INTEGER NOT NULL, value TEXT NOT NULL);")
db.commit()
print("skewed schema: same tables, different columns")
PY
  fi
  printf '[profiles.skewed]\ntype = "sqlite"\npath = "%s"\nread_only = true\n' "$SKEWED_DB" > "$SKEWED_CON"
  run 2 --connections "$SKEWED_CON" --non-interactive investigation run "$ID" --connection skewed
  require "review is stale (target changed, schema changed); pass --revalidate to re-review the current state"
else
  step "13 · Bonus skipped: neither sqlite3 nor python3 is available"
fi

use_alice

step "14 · Parameters: declare them, run with values, refuse a missing one"
note "The demo orders carry three statuses; the parameterised query counts one"
note "status since a date. The distinct values first, so the bound value is real:"
run_from "$ALICE_HOME" 0 --connections "$ALICE_CON" --profile demo --non-interactive query --sql "SELECT DISTINCT status FROM orders ORDER BY status"
require "completed"
run_from "$ALICE_HOME" 0 --connections "$ALICE_CON" --profile demo --non-interactive investigation save --name "orders since" --sql "SELECT count(*) AS n FROM orders WHERE status = :status AND order_date >= :min_date" --param-spec status:string:required --param-spec min_date:date
require "Saved exactly as shown. Review the SQL before sharing: literals are stored verbatim."
require '"required": true'
require '"type": "date"'
PARAM_ID=$(printf '%s\n' "$LAST_OUT" | sed -n 1p)
case "$PARAM_ID" in
  *[!a-z0-9-]* | "") fail "unexpected investigation id: $PARAM_ID" ;;
esac
run_from "$ALICE_HOME" 0 --connections "$ALICE_CON" --profile demo --non-interactive investigation run "$PARAM_ID" --param status=completed --param min_date=2025-08-01
require "246"
require "params: status, min_date"
case "$LAST_OUT" in
  *completed* | *2025-08-01*) fail "parameter values leaked into the evidence line" ;;
esac
run_from "$ALICE_HOME" 2 --connections "$ALICE_CON" --profile demo --non-interactive investigation run "$PARAM_ID" --param min_date=2025-08-01
require "missing required parameter(s): status (string) — pass each as --param <name>=<value>"

step "15 · Context: alice records a confirmed claim and exports the portable file"
note "The claim binds against the cached schema: refresh first, then remember —"
note "a claim recorded before any refresh reads stale, not current."
run_from "$ALICE_HOME" 0 --connections "$ALICE_CON" --profile demo --non-interactive connection schema demo --refresh
require "demo.main.orders"
run_from "$ALICE_HOME" 0 --connections "$ALICE_CON" --profile demo --non-interactive contracts remember demo.main.orders --kind description --value "Orders by synthetic customers; region joins through customers"
require "remembered description Orders by synthetic customers; region joins through customers for demo.main.orders (confirmed)"
run_from "$ALICE_HOME" 0 --connections "$ALICE_CON" --profile demo --non-interactive contracts list
require "[current]"
run_from "$ALICE_HOME" 0 --connections "$ALICE_CON" --profile demo --non-interactive contracts export "$SHARED/context.json"
require "exported 1 claims to $SHARED/context.json"
require "Review the file before sharing."

use_bob

step "16 · Bob imports the context — every item lands pending review"
run_from "$BOB_HOME" 0 --connections "$BOB_CON" --profile demo --non-interactive connection schema demo --refresh
require "demo.main.orders"
run_from "$BOB_HOME" 0 --connections "$BOB_CON" --profile demo --non-interactive contracts import "$SHARED/context.json"
require "imported 1, skipped 0, conflicts 0, unavailable 0"
require "Imported items are pending review: saya contracts queue"
run_from "$BOB_HOME" 0 --connections "$BOB_CON" --profile demo --non-interactive contracts queue
require "candidate  table_description"
require "demo.main.orders"

step "17 · \`saya open\` stages a CSV as a read-only, text-typed snapshot"
note "zip leads with a zero: a numeric cast would eat it. The snapshot keeps"
note "every column text — the preview says so, and one query proves it travels."
printf 'zip,city,amount,note\n02134,Boston,120.50,\n90210,Los Angeles,89.99,ok\n10001,New York,,\n' > "$BOB_HOME/deliveries.csv"
export SAYA_FILES_DIR="$BOB_HOME/files"
run_from "$BOB_HOME" 0 open deliveries.csv --non-interactive
require "SHA-256: 98262980cbee"
require "Rows: 3 · Columns: 4"
require "Delimiter: , · Header row: yes"
require "zip: text (0 nulls)"
require "amount: decimal (1 nulls)"
require "Stored as text columns; use --typed for a typed copy."
STAGED_DB=$(printf '%s\n' "$LAST_OUT" | sed -n 's|^Staged: \(.*\.duckdb\) (.*)$|\1|p')
if [ -z "$STAGED_DB" ]; then
  fail "could not read the staged path from open's output"
fi
STAGED_CON="$(dirname "$STAGED_DB")/connections.toml"
run_from "$BOB_HOME" 0 open --list
require "98262980cbee  deliveries.csv  3 rows"
run_from "$BOB_HOME" 0 --connections "$STAGED_CON" --profile file_deliveries --non-interactive query --sql "SELECT zip, city FROM deliveries ORDER BY city LIMIT 1"
require "02134	Boston"
run_from "$BOB_HOME" 0 open --cleanup all
require "Removed: 98262980cbee deliveries.csv"
run_from "$BOB_HOME" 0 open --list
require "No staged file sources."
unset SAYA_FILES_DIR

use_alice

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