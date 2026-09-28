#!/usr/bin/env bash
# scripts/walkthrough.sh — a recorded, reproducible synthetic walkthrough of
# the whole journey: demo → setup refusal → question → inspect SQL/evidence →
# save → export → import in a second environment → rerun.
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

step "Walkthrough complete"
note "All commands behaved as recorded; exit 0."