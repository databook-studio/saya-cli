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
